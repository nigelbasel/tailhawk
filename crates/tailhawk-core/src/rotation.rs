//! Rotation and truncation — `SPEC.md` §5.5, the file-side half.
//!
//! [`crate::follow`] handles a file that grows. This handles the file being replaced underneath it,
//! which is the ordinary condition of every production log and the thing a tail tool exists to
//! survive.
//!
//! ## Identity, never the path
//!
//! §5.5: "Tab identity, dedup and rotation detection are keyed on file identity, never on the path
//! string." A path is a name, and rotation is precisely the act of moving names around — `app.log`
//! becomes `app.log.1` and a new `app.log` appears, and every byte of the old file is still there
//! under a different name. [`FileIdentity`] is `(VolumeSerialNumber, FILE_ID_128)`, which survives
//! the rename and differs for the new file, so comparing it answers the question the path cannot.
//!
//! ## Drain before switching, and this is the whole point
//!
//! §5.5 on rename-and-recreate: "**Drain the old handle to EOF first**, then switch to the new file
//! at 0. This is where naive tools lose the last KB." A writer that rotates has usually just written
//! its final lines to the old file; a reader that notices the new file and jumps to it loses exactly
//! the lines most likely to say why the process rolled. [`Rotation::Replaced`] is therefore a
//! *report*, not an instruction to switch — the caller finishes reading the old handle first, and
//! `a_replaced_file_is_drained_before_the_switch` is what holds that to it.
//!
//! ## ⚠ What this does **not** do
//!
//! - **Roll-to-new-name** — Serilog's and NLog's *default*. §5.5 is explicit that "the written-to
//!   path never changes identity and never shrinks — nothing about the old file signals the roll",
//!   so detection is directory-side and belongs with §5.5b's rolling sets. This module reports
//!   [`Rotation::Stable`] for it, correctly and uselessly, and the gap is recorded rather than
//!   papered over.
//! - **The "file truncated" separator row** §5.5 asks for. That is a rendering feature.

use std::path::{Path, PathBuf};

use crate::file::{FileIdentity, LogFile};
use crate::Result;

/// What the watched path looks like now, relative to the handle being held.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Rotation {
    /// The path still names the file we hold, and it has not shrunk.
    Stable,
    /// **Copy-truncate.** The file we hold is shorter than we have already read, so its contents
    /// were replaced in place. §5.5: reset to offset 0.
    Truncated,
    /// **Rename-and-recreate.** A *different* file now occupies the path. §5.5: drain the handle we
    /// hold to EOF before attaching to it, or the last lines before the roll are lost.
    Replaced,
    /// The path does not exist. §5.5 wants `tail -F` semantics — keep the handle, keep retrying,
    /// say so in the UI, and never treat it as an error.
    Missing,
}

/// Watches one path for the file under it being replaced.
#[derive(Clone, Debug)]
pub struct Watch {
    path: PathBuf,
    identity: FileIdentity,
}

impl Watch {
    /// Starts watching, remembering the identity of the file currently at `path`.
    pub fn new(path: &Path, identity: FileIdentity) -> Self {
        Self {
            path: path.to_path_buf(),
            identity,
        }
    }

    /// Adopts a new file as the one being watched, after a switch.
    pub fn adopt(&mut self, identity: FileIdentity) {
        self.identity = identity;
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn identity(&self) -> FileIdentity {
        self.identity
    }

    /// Classifies the path against the handle being held.
    ///
    /// `held_len` is the length of the file we hold **now**; `scanned_to` is how far it has been
    /// read. Truncation is tested first and deliberately: a copy-truncated file keeps its identity,
    /// so the identity comparison would call it [`Stable`](Rotation::Stable) and the reader would
    /// sit on a file whose contents have been replaced under it.
    ///
    /// **Opening the path is the only way to ask this question.** §5.5 keys detection on identity,
    /// and an identity can only be read from a handle; there is no path-level query for it. The open
    /// is share-all and read-only, so it cannot block the writer — the guarantee `file.rs` opens
    /// with, applied to a file we may not end up keeping.
    pub fn check(&self, held_len: u64, scanned_to: u64) -> Rotation {
        if held_len < scanned_to {
            return Rotation::Truncated;
        }
        let Ok(at_path) = LogFile::open(&self.path) else {
            return Rotation::Missing;
        };
        match at_path.identity() {
            Ok(identity) if identity == self.identity => Rotation::Stable,
            Ok(_) => Rotation::Replaced,
            // The file is there but will not answer. Treating that as a rotation would throw away a
            // perfectly good handle over a transient failure, so it is left alone.
            Err(_) => Rotation::Stable,
        }
    }

    /// Opens whatever is at the path now, for the caller to switch to.
    ///
    /// Separate from [`check`](Self::check) so that a caller **cannot** switch without having first
    /// decided to — the drain has to happen in between, and a `check` that returned an open handle
    /// would invite skipping it.
    pub fn open_current(&self) -> Result<LogFile> {
        LogFile::open(&self.path).map_err(|e| crate::Error(format!("{e}")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join("tailhawk-rotation");
        std::fs::create_dir_all(&dir).expect("scratch dir");
        let path = dir.join(name);
        let _ = std::fs::remove_file(&path);
        path
    }

    fn write(path: &Path, text: &str) {
        std::fs::write(path, text).expect("write");
    }

    fn watch_for(path: &Path) -> (Watch, LogFile) {
        let file = LogFile::open(path).expect("open");
        let watch = Watch::new(path, file.identity().expect("identity"));
        (watch, file)
    }

    #[test]
    fn an_untouched_file_is_stable() {
        let path = scratch("stable.log");
        write(&path, "one\ntwo\n");
        let (watch, file) = watch_for(&path);
        let len = file.len().expect("len");
        assert_eq!(watch.check(len, len), Rotation::Stable);
    }

    /// **Copy-truncate keeps the file's identity**, which is why length is tested first.
    #[test]
    fn a_copy_truncated_file_is_not_mistaken_for_a_stable_one() {
        let path = scratch("truncate.log");
        write(&path, "one\ntwo\nthree\n");
        let (watch, file) = watch_for(&path);
        let before = file.len().expect("len");

        // What a copy-truncating writer does: copy elsewhere, then truncate in place.
        std::fs::OpenOptions::new()
            .write(true)
            .truncate(true)
            .open(&path)
            .expect("truncate must not be blocked");

        let after = file.len().expect("len after");
        assert!(after < before, "the truncation did not take");
        assert_eq!(
            watch.check(after, before),
            Rotation::Truncated,
            "identity is unchanged, so only the length can reveal this"
        );
    }

    /// **Rename-and-recreate**: our handle follows the renamed file, and a new one takes the path.
    #[test]
    fn a_recreated_file_at_the_path_is_a_replacement() {
        let path = scratch("rename.log");
        let rolled = scratch("rename.log.1");
        write(&path, "before the roll\n");
        let (watch, file) = watch_for(&path);
        let len = file.len().expect("len");

        std::fs::rename(&path, &rolled).expect("rename must not be blocked");
        // The handle follows the bytes, not the name.
        assert_eq!(file.len().expect("len after rename"), len);
        assert_eq!(
            watch.check(len, len),
            Rotation::Missing,
            "between the rename and the recreate there is nothing at the path"
        );

        write(&path, "after the roll\n");
        assert_eq!(watch.check(len, len), Rotation::Replaced);

        let _ = std::fs::remove_file(&rolled);
    }

    /// **The old handle still reads after the rename**, which is what makes draining possible.
    ///
    /// §5.5: "Drain the old handle to EOF first … this is where naive tools lose the last KB." The
    /// bytes a writer appends *between* the rename and the reader noticing are only reachable
    /// through the handle already held, so this asserts they are.
    #[test]
    fn a_replaced_file_is_drained_before_the_switch() {
        let path = scratch("drain.log");
        let rolled = scratch("drain.log.1");
        write(&path, "first\n");
        let (mut watch, old) = watch_for(&path);

        std::fs::rename(&path, &rolled).expect("rename");
        // The writer's last lines, written after the roll and before anyone noticed.
        {
            let mut f = std::fs::OpenOptions::new()
                .append(true)
                .open(&rolled)
                .expect("append to the rolled file");
            f.write_all(b"last words\n").expect("append");
        }
        write(&path, "brand new\n");

        assert_eq!(watch.check(6, 6), Rotation::Replaced);

        // Drain: the old handle still sees the bytes written after the rename.
        let mut buf = [0u8; 64];
        let n = old.read_at(0, &mut buf).expect("read old");
        assert_eq!(
            &buf[..n],
            b"first\nlast words\n",
            "the last lines before the roll were unreachable, which is the bug §5.5 names"
        );

        // Only then switch.
        let new = watch.open_current().expect("open new");
        watch.adopt(new.identity().expect("identity"));
        assert_eq!(
            watch.check(10, 10),
            Rotation::Stable,
            "the new file is now ours"
        );
        let n = new.read_at(0, &mut buf).expect("read new");
        assert_eq!(&buf[..n], b"brand new\n");

        let _ = std::fs::remove_file(&rolled);
    }

    /// A path that has gone is `tail -F`'s wait, not an error.
    #[test]
    fn a_missing_path_is_waited_for_rather_than_failed() {
        let path = scratch("missing.log");
        write(&path, "here\n");
        let (watch, file) = watch_for(&path);
        let len = file.len().expect("len");

        std::fs::remove_file(&path).expect("delete must not be blocked");
        assert_eq!(watch.check(len, len), Rotation::Missing);
        // And the handle we hold still works, which is what makes waiting reasonable.
        let mut buf = [0u8; 16];
        let n = file
            .read_at(0, &mut buf)
            .expect("read a deleted-but-open file");
        assert_eq!(&buf[..n], b"here\n");
    }

    /// **Roll-to-new-name is invisible from here, and the test says so rather than pretending.**
    ///
    /// Serilog and NLog default to this: the writer stops touching `log-20260813.txt` and starts
    /// `log-20260814.txt`. The watched path keeps its identity and never shrinks, so §5.5's
    /// file-side detection cannot see it — which is exactly why §5.5b makes it a directory-side
    /// problem. Asserting `Stable` here keeps the gap visible instead of letting someone assume it
    /// is covered.
    #[test]
    fn a_roll_to_a_new_name_is_invisible_to_file_side_detection() {
        let today = scratch("log-20260813.txt");
        let tomorrow = scratch("log-20260814.txt");
        write(&today, "monday\n");
        let (watch, file) = watch_for(&today);
        let len = file.len().expect("len");

        write(&tomorrow, "tuesday\n");

        assert_eq!(
            watch.check(len, len),
            Rotation::Stable,
            "file-side detection cannot see a roll to a new name — §5.5b is where this is solved"
        );

        let _ = std::fs::remove_file(&tomorrow);
    }
}
