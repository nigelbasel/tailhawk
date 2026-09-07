//! Reading a pipe — `SPEC.md` §4.2, and §13.2's rules about where the bytes land.
//!
//! `type app.log | tailhawk` and `Get-Content -Wait x.log | tailhawk` are how a stream arrives, and
//! a pipe is **unseekable and consume-once**: there is no scrollback in it, no length to poll, and
//! no second chance at a byte once read.
//!
//! ## The spill is the whole design
//!
//! §4.2: the stream is "**spilled to a temp file**, not held in memory. This gives scrollback (a pipe
//! is unseekable and consume-once), **reuses the same index path as a real file**, and preserves the
//! multi-GB promise."
//!
//! Taken literally, that sentence means **this module contains no tailing, indexing, decoding or
//! rendering code at all.** A pump thread copies bytes from the pipe into a file; the shell opens
//! that file with [`LogSet`](crate::set::LogSet) exactly as it opens any other, and everything
//! already built — encoding detection on the piped bytes, the line index, following, the viewport —
//! applies without knowing a pipe was involved. A parallel in-memory stream path would have had to
//! re-earn every one of those.
//!
//! It also gets §4.2's encoding rule for free: detection runs on the *spilled bytes*, which are the
//! piped bytes, and PowerShell's native-command pipeline has historically emitted UTF-16 and OEM
//! codepages.
//!
//! ## End of stream is not end of program
//!
//! §4.2: "`ERROR_BROKEN_PIPE` or a 0-byte read means **stream complete** — the window stays open and
//! the trailing partial line is flushed. **It is not an app exit.**" A user who pipes a finished
//! command's output wants to *read* it; closing on EOF would make the tool useless for exactly the
//! case it is most convenient for. [`Pump::finished`] reports the transition and nothing acts on it
//! but the title.
//!
//! ## Where the bytes land is a privacy question
//!
//! §13.2 is unambiguous, because the working set is "customer log files containing PII, connection
//! strings, bearer tokens and session IDs": spill files are "created with a **restrictive DACL** in
//! `%TEMP%`, **deleted on clean exit**, and **reaped on next launch if orphaned**. The spill location
//! is displayed in source properties, because a user piping production logs deserves to know where
//! they landed."
//!
//! All four are here. The DACL is built from the calling user's own SID and supplied at creation —
//! see [`Spill::create`] — because a file created the ordinary way in `%TEMP%` **inherits ACEs for
//! SYSTEM and BUILTIN\\Administrators**, measured on this machine, and on a shared or domain-joined
//! one can inherit more.

use std::ffi::c_void;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use windows::core::{PCWSTR, PWSTR};
use windows::Win32::Foundation::{
    CloseHandle, GetLastError, LocalFree, ERROR_BROKEN_PIPE, HANDLE, HLOCAL, INVALID_HANDLE_VALUE,
};
use windows::Win32::Security::Authorization::{
    ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
};
use windows::Win32::Security::{
    GetTokenInformation, TokenUser, PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES, TOKEN_QUERY,
    TOKEN_USER,
};
use windows::Win32::Storage::FileSystem::{
    CreateDirectoryW, CreateFileW, GetFileType, ReadFile, CREATE_NEW, FILE_ATTRIBUTE_NORMAL,
    FILE_SHARE_DELETE, FILE_SHARE_MODE, FILE_SHARE_READ, FILE_SHARE_WRITE, FILE_TYPE_CHAR,
    FILE_TYPE_DISK, FILE_TYPE_PIPE,
};
use windows::Win32::System::Console::{GetStdHandle, STD_INPUT_HANDLE};
use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

use crate::{Error, Result};

/// The standard input handle, carried to the pump thread.
///
/// `HANDLE` wraps a raw pointer and so is not `Send`. This is sound for the one use it has: the
/// handle is the process's own standard input, **nothing else in Tailhawk ever reads it**, and after
/// [`Pump::start`] returns only the pump thread touches it — so there is no sharing to synchronise,
/// only a move across a thread boundary. The same reasoning `file.rs` records for `LogFile`.
struct StdinHandle(HANDLE);

unsafe impl Send for StdinHandle {}

impl StdinHandle {
    /// The handle itself.
    ///
    /// A method rather than a public field, and that is load-bearing: edition 2021 closures
    /// capture individual **fields**, so `handle.0` inside a `move` closure captures the bare
    /// `HANDLE` and the `Send` above never applies. A call takes `&self` and captures the whole.
    fn get(&self) -> HANDLE {
        self.0
    }
}

/// Bytes read from the pipe per `ReadFile`.
///
/// One comfortable read, matching `indexer.rs` and `rows.rs`. The pump thread blocks, so this is not
/// a latency budget — it is how much a slow producer's line can sit in the buffer before it is
/// written and becomes visible, and 64 KB is well under what a reader notices.
const READ_BYTES: usize = 64 * 1024;

/// What is on the standard input handle.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Stdin {
    /// A pipe. §4.2's main case — read it.
    Pipe,
    /// A redirected file (`tailhawk < app.log`). Also read; it is seekable, but spilling it costs
    /// one copy and keeps a single code path.
    Disk,
    /// **An interactive console.** §4.2: "do not block." Reading here waits for a human to type,
    /// which for a GUI application means a window that never appears.
    Console,
    /// No handle, or one that will not answer.
    None,
}

impl Stdin {
    /// Whether there is a stream worth spilling. The one question a caller asks.
    pub fn readable(self) -> bool {
        matches!(self, Stdin::Pipe | Stdin::Disk)
    }
}

/// Classifies the standard input handle. §4.2's `GetFileType(GetStdHandle(STD_INPUT_HANDLE))`.
pub fn stdin() -> Stdin {
    let handle = match unsafe { GetStdHandle(STD_INPUT_HANDLE) } {
        Ok(h) if !h.is_invalid() && !h.0.is_null() => h,
        _ => return Stdin::None,
    };
    match unsafe { GetFileType(handle) } {
        FILE_TYPE_PIPE => Stdin::Pipe,
        FILE_TYPE_DISK => Stdin::Disk,
        FILE_TYPE_CHAR => Stdin::Console,
        _ => Stdin::None,
    }
}

/// Prefix every spill file shares, so [`reap_orphans`] can recognise one it did not create.
const SPILL_PREFIX: &str = "tailhawk-spill-";
const SPILL_SUFFIX: &str = ".log";

/// A temp file holding a stream's bytes, with §13.2's DACL and lifetime.
pub struct Spill {
    path: PathBuf,
    file: Option<std::fs::File>,
    /// Whether the file should be removed when this is dropped. Cleared by
    /// [`keep`](Spill::keep) for the case where the user asked to save it.
    remove_on_drop: bool,
}

impl Spill {
    /// Creates an empty spill in `%TEMP%`, readable and writable **only by the calling user**.
    ///
    /// §13.2 requires the restrictive DACL and this is where it is applied, at creation, through
    /// `CreateFileW`'s `SECURITY_ATTRIBUTES` — **not** by relaxing a default and tightening it
    /// afterwards, which would leave a window in which the file exists with `%TEMP%`'s inherited
    /// permissions and customer log content already in it.
    ///
    /// The SDDL is `D:P(A;;FA;;;<user SID>)` — one access-allowed ACE for the calling user and
    /// nothing else. Naming the user's own SID rather than `CO` (CREATOR OWNER) is deliberate: `CO`
    /// is an inheritance placeholder and means nothing in an ACE on a file that will never have
    /// children.
    ///
    /// **⚠ The `P` is belt-and-braces, and a negative control is why that is said rather than
    /// assumed.** Removing it changed nothing: the created file's DACL still reads back as
    /// `D:P(A;;FA;;;<sid>)`, because supplying an explicit DACL through `SECURITY_ATTRIBUTES`
    /// already stops the parent's inheritable ACEs being merged. It is kept because it costs one
    /// character and states the intent — **not** because it is the mechanism, and the doc says so
    /// rather than leaving a later reader to conclude from a passing test that it was.
    ///
    /// What *is* load-bearing is supplying a descriptor at all. Measured, in the same directory:
    ///
    /// | | DACL |
    /// |---|---|
    /// | An ordinary file in `%TEMP%` | `D:(A;ID;FA;;;SY)(A;ID;FA;;;BA)(A;ID;FA;;;<user>)` |
    /// | A spill | `D:P(A;;FA;;;<user>)` |
    ///
    /// `a_spill_is_readable_only_by_the_user_who_made_it` asserts that **difference**, not the
    /// spill's DACL alone — the first version asserted the latter and passed with the flag it
    /// claimed to test removed.
    ///
    /// `CREATE_NEW` rather than `CREATE_ALWAYS`, so a name collision is an error instead of a
    /// silent truncation of somebody else's spill.
    pub fn create() -> Result<Self> {
        let dir = std::env::temp_dir();
        let sid = current_user_sid()?;
        let sddl = format!("D:P(A;;FA;;;{sid})");

        // Two attempts. The name carries the process id and a counter, so a collision means another
        // process reused a recycled pid within the same second — vanishingly rare, and worth one
        // retry rather than a loop that could spin.
        let mut last = None;
        for attempt in 0..2u32 {
            let counter = SPILL_COUNTER.fetch_add(1, Ordering::Relaxed);
            let path = dir.join(format!(
                "{SPILL_PREFIX}{}-{}{attempt}{SPILL_SUFFIX}",
                std::process::id(),
                counter
            ));
            match create_locked_down(&path, &sddl) {
                Ok(file) => {
                    return Ok(Self {
                        path,
                        file: Some(file),
                        remove_on_drop: true,
                    })
                }
                Err(e) => last = Some(e),
            }
        }
        Err(last.unwrap_or_else(|| Error("no spill file could be created".into())))
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Stops this spill being deleted on drop, for a user who asked to keep the stream.
    pub fn keep(&mut self) {
        self.remove_on_drop = false;
    }

    /// Appends bytes and flushes them, so a reader watching the file sees them at once.
    ///
    /// Test-only: the pump thread opens its own append handle, because a `Spill` the window owns
    /// and a thread that may outlive it cannot share one writer without a lock this does not need.
    ///
    /// **Flushed per write and not buffered**, because the whole point of the spill is that another
    /// part of this process is tailing it: bytes still in a `BufWriter` are bytes the viewport
    /// cannot show, and a slow producer would leave the last partial buffer invisible indefinitely.
    #[cfg(test)]
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<()> {
        let Some(file) = self.file.as_mut() else {
            return Ok(());
        };
        file.write_all(bytes)?;
        file.flush()
    }
}

impl Drop for Spill {
    /// §13.2: "deleted on clean exit". Dropping the handle first, because Windows will not remove a
    /// file this process still holds open for writing.
    fn drop(&mut self) {
        self.file = None;
        if self.remove_on_drop {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

static SPILL_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Names every part of a rolling spill, so [`reap_orphans`] and `pattern.rs` both recognise one.
const PART_PREFIX: &str = "part-";

/// How large one part grows before the next is started.
///
/// A roll costs a directory listing, a file creation and — on the reading side — a drain and a
/// switch, so it wants to be rare; a part is also the granularity at which history is thrown
/// away, so it wants to be small. Sixty-four megabytes is a few minutes of the owner's live
/// source, which is both.
pub const PART_BYTES: u64 = 64 * 1024 * 1024;

/// How much of a rolling spill survives before its oldest parts are deleted.
///
/// The same figure as [`crate::set::EAGER_BYTES`], and that is the argument for it: a set indexes
/// about that much when it opens, so bytes kept beyond it are bytes the reader would not have
/// indexed anyway. At the rate measured against the owner's live source it is roughly the last
/// hour, and a tail left running overnight costs half a gigabyte of `%TEMP%` rather than fifteen.
pub const SPILL_BYTES: u64 = 512 * 1024 * 1024;

/// The name of part `n`.
///
/// **Zero-padded to six digits because `pattern.rs` has to read it.** A rolling set's order is
/// inferred from the names alone: six digits clears [`crate::pattern::FIELD_MIN_DIGITS`], so the
/// parts are recognised as members at all, and padding is what makes them sort `1, 2, 10` rather
/// than `1, 10, 2` — §5.5b's silent reversal, history shown backwards with nothing to say so.
fn part_name(n: u64) -> String {
    format!("{PART_PREFIX}{n:06}{SPILL_SUFFIX}")
}

/// Every part in `dir`, oldest first, with its length.
fn parts_of(dir: &Path) -> Vec<(PathBuf, u64)> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut parts: Vec<(PathBuf, u64)> = entries
        .flatten()
        .filter(|e| {
            e.file_name()
                .to_str()
                .is_some_and(|n| n.starts_with(PART_PREFIX) && n.ends_with(SPILL_SUFFIX))
        })
        .map(|e| {
            // **`fs::metadata` on the path, not the entry's own.** A directory entry carries the
            // size the last directory update wrote, which for a file somebody holds an open write
            // handle to reads zero however much has been written and flushed to it. Nothing here
            // keeps such a handle today, so both answers agree; the day the writer keeps its part
            // open across polls, the entry's answer would silently under-count the live part and
            // every test would still pass.
            let path = e.path();
            let len = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
            (path, len)
        })
        .collect();
    parts.sort_by(|a, b| a.0.cmp(&b.0));
    parts
}

/// How many of the oldest parts must go for a spill holding `sizes` to fit in `cap`.
///
/// `sizes` is oldest-first and its last entry is the live part. **The live part is never
/// deletable**, whatever it costs: it is the one being appended to, and a bound that could delete
/// it would answer "the disk is full" by stopping the tail — which is the one thing the file
/// exists to do. So a single part over the cap is reported as nothing to delete, and the spill is
/// briefly larger than its bound rather than briefly empty.
pub fn excess(sizes: &[u64], cap: u64) -> usize {
    let mut total: u64 = sizes.iter().sum();
    let mut dropped = 0;
    while total > cap && dropped + 1 < sizes.len() {
        total -= sizes[dropped];
        dropped += 1;
    }
    dropped
}

/// A spill that rolls: a directory of numbered parts, bounded in total, read as one document.
///
/// §4.2's [`Spill`] is one file that only grows, which is right for a pipe — a pipe ends. A Loki
/// tail does not: `LOKI.md` §5's poll appends for as long as the window is open, and at the rate
/// measured against the owner's own source that is about 650 MB an hour, for ever. The three ways
/// to bound a growing file are to stop writing it, to rewrite it shorter, or to keep it in parts.
/// Stopping loses the tail; rewriting invalidates every byte offset [`crate::index::LineIndex`]
/// holds. So: parts.
///
/// **Almost nothing new is needed on the reading side, and that is the whole design.** The parts
/// are named so `pattern.rs` orders them oldest-first, which makes the directory an ordinary
/// §5.5b rolling set; [`crate::set::LogSet`] attaches a new part with the drain-then-switch it
/// already does for a rolling log, and retires a deleted one because §5.5b requires retention
/// deletions be tolerated. `file.rs`'s share mode includes `DELETE`, so the writer can delete a
/// part the reader is holding — the name goes at once under the POSIX-semantics delete Windows 10
/// 1809 gives us, which is §2.1's floor, and the bytes go when the reader retires the member a
/// listing interval later.
///
/// **The directory is also a fix, not only a container.** A remote source's spill was opened with
/// [`LogSet::open`](crate::set::LogSet::open), which infers a set from the names beside it, and
/// every spill in `%TEMP%` shares one literal skeleton — so a second remote source could splice
/// the first one's records into its scrollback as older history. That is the hazard
/// `a_spill_never_adopts_another_instances_spill_as_a_rolling_set` names and `open_single` guards
/// the pipe path against; parts in a directory of their own make the inference *correct* rather
/// than merely guarded, because the only names beside a part are that source's own.
pub struct SpillSet {
    dir: PathBuf,
    sddl: String,
    /// **The directory itself, held open for as long as this lives**, so another instance's
    /// [`reap_orphans`] can see that someone is using it. The trick [`Spill`] plays with its file,
    /// moved up a level — and it has to be up a level. Holding *part one* instead looks equivalent
    /// and is not: retention deletes part one from every spill that runs long enough, and a live
    /// tail would then be holding nothing at all. The test suite proved that within a minute of
    /// the rule existing, by reaping a spill another test was still writing to.
    ///
    /// Shared read/write/delete, so this process's own reader, writer and retention are unaffected;
    /// a reaper asks for the directory with **no** sharing, and that is the whole conversation.
    hold: Option<std::fs::File>,
    remove_on_drop: bool,
}

impl SpillSet {
    /// Creates the directory and its first part, both with §13.2's restrictive DACL.
    pub fn create() -> Result<Self> {
        let temp = std::env::temp_dir();
        let sid = current_user_sid()?;
        let sddl = format!("D:P(A;;FA;;;{sid})");

        let mut last = None;
        for attempt in 0..2u32 {
            let counter = SPILL_COUNTER.fetch_add(1, Ordering::Relaxed);
            let dir = temp.join(format!(
                "{SPILL_PREFIX}{}-{}{attempt}",
                std::process::id(),
                counter
            ));
            match create_dir_locked_down(&dir, &sddl) {
                Ok(()) => {
                    let mut set = Self {
                        dir,
                        sddl,
                        hold: None,
                        remove_on_drop: true,
                    };
                    // Built before either, so a failure below drops `set` and takes the directory
                    // with it rather than leaving an empty one in `%TEMP%`.
                    //
                    // **The hold comes before the first part, and the order is the whole guard.**
                    // A reaper skips a directory holding no parts at all, so the moment between
                    // the directory appearing and this handle existing is a moment in which there
                    // is nothing to reap. Taking the hold second leaves a window where a live
                    // spill has a part and no holder, and a concurrent launch deletes it.
                    set.hold = Some(open_dir(&set.dir, SHARE_ALL)?);
                    drop(create_locked_down(&set.first_part(), &set.sddl)?);
                    return Ok(set);
                }
                Err(e) => last = Some(e),
            }
        }
        Err(last.unwrap_or_else(|| Error("no spill directory could be created".into())))
    }

    /// The directory holding the parts. §13.2's "the spill location is displayed in source
    /// properties".
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// The part a document opens. The oldest, because a set is read oldest-first.
    pub fn first_part(&self) -> PathBuf {
        self.dir.join(part_name(1))
    }

    /// The writing half, for the thread that fills the spill.
    ///
    /// Separate because a `SpillSet` the window owns and a tail thread that may outlive it cannot
    /// share one writer without a lock neither needs — the same split [`Spill`] and [`Pump`] have.
    pub fn writer(&self) -> SpillWriter {
        SpillWriter {
            dir: self.dir.clone(),
            sddl: self.sddl.clone(),
            part: 1,
            part_bytes: PART_BYTES,
            cap: SPILL_BYTES,
        }
    }
}

impl Drop for SpillSet {
    /// §13.2: "deleted on clean exit" — the whole directory, and the handle first, for the reason
    /// [`Spill`] drops its file first.
    fn drop(&mut self) {
        self.hold = None;
        if self.remove_on_drop {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }
}

/// Appends to a [`SpillSet`], rolling and deleting as its caps require.
pub struct SpillWriter {
    dir: PathBuf,
    sddl: String,
    part: u64,
    part_bytes: u64,
    cap: u64,
}

impl SpillWriter {
    /// Smaller caps, so a test can reach a roll and a deletion without writing half a gigabyte.
    ///
    /// The production caps are the constants and nothing outside a test sets them: a spill whose
    /// bound could be configured is a bound somebody can turn off.
    #[cfg(test)]
    fn with_limits(mut self, part_bytes: u64, cap: u64) -> Self {
        self.part_bytes = part_bytes;
        self.cap = cap;
        self
    }

    /// Shrinks the retention cap mid-flight, so a test can let a reader open a part and *then*
    /// have retention take it — which is the case worth testing and cannot be reached by opening
    /// a spill that was already over its cap.
    #[cfg(test)]
    fn cap_to(&mut self, cap: u64) {
        self.cap = cap;
    }

    /// Appends `text` to the live part. Returns how many old parts this cost.
    ///
    /// **The count is returned rather than logged** because `LOKI.md` §6 wants what is missing
    /// said: a tail whose oldest records have been thrown away to stay inside its bound has lost
    /// history the user could otherwise scroll to, and the status bar is where that belongs.
    pub fn append(&mut self, text: &str) -> Result<usize> {
        let mut live = self.dir.join(part_name(self.part));
        let full = std::fs::metadata(&live).map(|m| m.len()).unwrap_or(0) >= self.part_bytes;
        if full {
            let next = self.dir.join(part_name(self.part + 1));
            create_locked_down(&next, &self.sddl)?;
            self.part += 1;
            live = next;
        }
        let mut file = std::fs::OpenOptions::new()
            .append(true)
            .open(&live)
            .map_err(|e| Error(format!("{}: {e}", live.display())))?;
        file.write_all(text.as_bytes())
            .map_err(|e| Error(format!("{}: {e}", live.display())))?;
        file.flush()
            .map_err(|e| Error(format!("{}: {e}", live.display())))?;
        Ok(self.retire())
    }

    /// Deletes the oldest parts until the spill fits its cap. Returns how many went.
    ///
    /// A delete that fails is not an error and not retried here: the next append asks the same
    /// question of a fresh listing, and a part something else is holding without `FILE_SHARE_DELETE`
    /// is a reason to stay over the bound rather than to stop tailing.
    fn retire(&self) -> usize {
        let parts = parts_of(&self.dir);
        let sizes: Vec<u64> = parts.iter().map(|(_, len)| *len).collect();
        let mut gone = 0;
        for (path, _) in parts.iter().take(excess(&sizes, self.cap)) {
            if std::fs::remove_file(path).is_ok() {
                gone += 1;
            }
        }
        gone
    }
}

/// Read, write and delete, the same sharing `file.rs` opens a log with: this process reads a spill
/// directory, appends inside it and deletes from it while holding it open.
const SHARE_ALL: FILE_SHARE_MODE =
    FILE_SHARE_MODE(FILE_SHARE_READ.0 | FILE_SHARE_WRITE.0 | FILE_SHARE_DELETE.0);

/// Opens a handle to a *directory*.
///
/// `FILE_FLAG_BACKUP_SEMANTICS` is what makes `CreateFileW` accept one at all. With `share` at
/// [`SHARE_ALL`] this is a claim — "someone is using this" — and with it at zero it is the
/// question, which is how [`reap_orphans`] tells an abandoned spill from a live one.
fn open_dir(path: &Path, share: FILE_SHARE_MODE) -> Result<std::fs::File> {
    use std::os::windows::io::FromRawHandle;

    let wide_path = wide(path);
    let handle = unsafe {
        CreateFileW(
            PCWSTR(wide_path.as_ptr()),
            windows::Win32::Foundation::GENERIC_READ.0,
            share,
            None,
            windows::Win32::Storage::FileSystem::OPEN_EXISTING,
            windows::Win32::Storage::FileSystem::FILE_FLAG_BACKUP_SEMANTICS,
            None,
        )
    }
    .map_err(|e| Error(format!("{}: {e}", path.display())))?;
    if handle == INVALID_HANDLE_VALUE {
        return Err(Error(format!("{}: invalid handle", path.display())));
    }
    Ok(unsafe { std::fs::File::from_raw_handle(handle.0) })
}

/// Creates a directory whose DACL is exactly `sddl`.
fn create_dir_locked_down(path: &Path, sddl: &str) -> Result<()> {
    let sddl_wide: Vec<u16> = sddl.encode_utf16().chain(Some(0)).collect();
    let mut descriptor = PSECURITY_DESCRIPTOR::default();
    unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            PCWSTR(sddl_wide.as_ptr()),
            SDDL_REVISION_1,
            &mut descriptor,
            None,
        )
    }
    .map_err(|e| Error(format!("spill security descriptor: {e}")))?;

    let attributes = SECURITY_ATTRIBUTES {
        nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: descriptor.0,
        bInheritHandle: false.into(),
    };
    let wide_path = wide(path);
    let created = unsafe { CreateDirectoryW(PCWSTR(wide_path.as_ptr()), Some(&attributes)) };
    unsafe { LocalFree(HLOCAL(descriptor.0)) };
    created.map_err(|e| Error(format!("{}: {e}", path.display())))
}

/// How a stream stopped.
///
/// **`PLAN.md` asks a pipe source to "distinguish *writer finished* from *writer died mid-stream*",
/// and this is as far as a pipe can answer.** Both look identical at the handle: a producer that
/// exits cleanly and one that is killed each close their end, and the reader sees the same
/// `ERROR_BROKEN_PIPE`. The distinction the plan actually wants — `az containerapp logs show
/// --follow` returning 0 on a replica restart — needs the *process* exit code, which means owning
/// the child, which is the process-spawn source and not this. Recorded in `HANDOFF.md`.
///
/// What is distinguishable is a broken or exhausted pipe from a **read that failed for some other
/// reason**, and keeping those apart costs nothing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StreamEnd {
    /// EOF, or the producer closed its end. §4.2's "stream complete".
    Complete,
    /// The read failed for a reason that is not the ordinary end of a pipe.
    Failed(String),
}

/// Copies the standard input handle into a spill, on its own thread.
///
/// §4.2 wants "a background thread with blocking `ReadFile`", and blocking is the point: a pipe has
/// no length to poll and no readiness to test without an overlapped handle we do not own. The thread
/// sits in `ReadFile` until the producer writes or closes, which costs nothing while it waits and
/// cannot stall the message loop because it is not on it.
pub struct Pump {
    path: PathBuf,
    bytes: Arc<AtomicU64>,
    finished: Arc<AtomicBool>,
    /// Why the stream stopped, once it has. See [`StreamEnd`].
    ended: Arc<Mutex<Option<StreamEnd>>>,
    /// Kept so the spill outlives the thread and is deleted when the pump is dropped. The thread
    /// holds its own handle to the same file, opened through the same restricted DACL.
    _spill: Spill,
}

impl Pump {
    /// Starts pumping the standard input handle into a fresh spill file.
    ///
    /// The returned [`path`](Pump::path) can be opened immediately — it exists and is empty, which
    /// is a state the rest of the product already handles (`Follow::after_build` seeds line 0 for a
    /// file that was empty when it was indexed, and there is a test for it).
    pub fn start() -> Result<Self> {
        let spill = Spill::create()?;
        let path = spill.path().to_path_buf();
        let bytes = Arc::new(AtomicU64::new(0));
        let finished = Arc::new(AtomicBool::new(false));
        let ended: Arc<Mutex<Option<StreamEnd>>> = Arc::new(Mutex::new(None));

        let handle = match unsafe { GetStdHandle(STD_INPUT_HANDLE) } {
            Ok(h) if !h.is_invalid() => StdinHandle(h),
            _ => return Err(Error("no standard input handle".into())),
        };

        // The thread opens its own writer rather than sharing the `Spill`'s, so `Spill` can stay a
        // plain owner with a `Drop` that deletes — no lock, and no lifetime tangle between a thread
        // that may outlive the window and a value the window owns.
        let mut writer = std::fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .map_err(|e| Error(format!("{}: {e}", path.display())))?;
        let counted = Arc::clone(&bytes);
        let done = Arc::clone(&finished);
        let ended_by = Arc::clone(&ended);
        std::thread::Builder::new()
            .name("tailhawk-stdin".into())
            .spawn(move || {
                let ended = ended_by;
                let mut outcome = StreamEnd::Complete;
                let mut buf = vec![0u8; READ_BYTES];
                loop {
                    let mut read = 0u32;
                    let ok =
                        unsafe { ReadFile(handle.get(), Some(&mut buf), Some(&mut read), None) };
                    match ok {
                        // §4.2: a 0-byte read is **stream complete**, not an error and not an exit.
                        Ok(()) if read == 0 => break,
                        Ok(()) => {}
                        Err(e) => {
                            // §4.2 names `ERROR_BROKEN_PIPE` as the ordinary end of a pipe — the
                            // producer closed its end, which is [`StreamEnd::Complete`]. Anything
                            // else is a failure and is kept as one.
                            if unsafe { GetLastError() } != ERROR_BROKEN_PIPE {
                                outcome = StreamEnd::Failed(format!("{e}"));
                            }
                            break;
                        }
                    }
                    if let Err(e) = writer
                        .write_all(&buf[..read as usize])
                        .and_then(|()| writer.flush())
                    {
                        // **The spill failed, not the pipe.** A full disk here loses log content
                        // silently otherwise: the window would show what arrived and stop, looking
                        // exactly like a producer that finished.
                        outcome = StreamEnd::Failed(format!("spill: {e}"));
                        break;
                    }
                    counted.fetch_add(read as u64, Ordering::Relaxed);
                }
                *ended.lock().unwrap_or_else(|e| e.into_inner()) = Some(outcome);
                done.store(true, Ordering::Release);
            })
            .map_err(|e| Error(format!("stdin pump: {e}")))?;

        Ok(Self {
            path,
            bytes,
            finished,
            ended,
            _spill: spill,
        })
    }

    /// The spill file to open. §13.2 wants this shown to the user, "because a user piping production
    /// logs deserves to know where they landed".
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Bytes copied out of the pipe so far.
    pub fn bytes(&self) -> u64 {
        self.bytes.load(Ordering::Relaxed)
    }

    /// Whether the producer has closed its end. §4.2: this is **not** a reason to exit.
    pub fn finished(&self) -> bool {
        self.finished.load(Ordering::Acquire)
    }

    /// Why the stream stopped, or `None` while it is still running. See [`StreamEnd`].
    pub fn outcome(&self) -> Option<StreamEnd> {
        self.ended.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }
}

/// Deletes spill files left behind by a process that did not exit cleanly. §13.2's "reaped on next
/// launch if orphaned".
///
/// **Only files this product's naming scheme produced**, and only ones nothing currently holds open:
/// the open attempt is exclusive, so a spill belonging to a *running* Tailhawk fails it and is left
/// alone. That is the whole test — there is no pid parsing, because a pid is recycled and a
/// recycled one would make this delete a live instance's stream.
///
/// Returns how many were removed.
pub fn reap_orphans() -> usize {
    let dir = std::env::temp_dir();
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return 0;
    };
    let mut reaped = 0;
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        if !name.starts_with(SPILL_PREFIX) {
            continue;
        }
        let path = entry.path();
        // A [`SpillSet`] is a directory of parts rather than a file, and it is orphaned on the
        // same evidence a spill file is: nobody has it open. The claim a live one makes is on
        // the directory, so that is what is asked about — see [`SpillSet::hold`].

        if entry.file_type().is_ok_and(|kind| kind.is_dir()) {
            // An empty one is left alone: it is zero bytes, it can only be a create that failed
            // between the directory and its first part, and skipping it is what makes the
            // [`SpillSet::create`] ordering above a guarantee rather than a narrow race.
            if !parts_of(&path).is_empty()
                && open_dir(&path, FILE_SHARE_MODE(0)).is_ok()
                && std::fs::remove_dir_all(&path).is_ok()
            {
                reaped += 1;
            }
            continue;
        }
        if !name.ends_with(SPILL_SUFFIX) {
            continue;
        }
        if open_exclusive(&path).is_ok() && std::fs::remove_file(&path).is_ok() {
            reaped += 1;
        }
    }
    reaped
}

/// Opens a path with no sharing at all, purely to find out whether anyone else holds it.
fn open_exclusive(path: &Path) -> Result<()> {
    let wide: Vec<u16> = wide(path);
    let handle = unsafe {
        CreateFileW(
            PCWSTR(wide.as_ptr()),
            windows::Win32::Foundation::GENERIC_READ.0,
            FILE_SHARE_MODE(0),
            None,
            windows::Win32::Storage::FileSystem::OPEN_EXISTING,
            FILE_ATTRIBUTE_NORMAL,
            None,
        )
    }
    .map_err(|e| Error(format!("{}: {e}", path.display())))?;
    unsafe { CloseHandle(handle) }.map_err(|e| Error(format!("{e}")))?;
    Ok(())
}

fn wide(path: &Path) -> Vec<u16> {
    use std::os::windows::ffi::OsStrExt;
    path.as_os_str().encode_wide().chain(Some(0)).collect()
}

/// Creates a file whose DACL is exactly `sddl`, and hands back a `std::fs::File` for it.
fn create_locked_down(path: &Path, sddl: &str) -> Result<std::fs::File> {
    use std::os::windows::io::FromRawHandle;

    let sddl_wide: Vec<u16> = sddl.encode_utf16().chain(Some(0)).collect();
    let mut descriptor = PSECURITY_DESCRIPTOR::default();
    unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            PCWSTR(sddl_wide.as_ptr()),
            SDDL_REVISION_1,
            &mut descriptor,
            None,
        )
    }
    .map_err(|e| Error(format!("spill security descriptor: {e}")))?;

    let attributes = SECURITY_ATTRIBUTES {
        nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: descriptor.0,
        bInheritHandle: false.into(),
    };
    let wide = wide(path);
    let handle = unsafe {
        CreateFileW(
            PCWSTR(wide.as_ptr()),
            windows::Win32::Foundation::GENERIC_READ.0
                | windows::Win32::Foundation::GENERIC_WRITE.0,
            // The rest of the process reads this file through `LogFile`, whose share mode is
            // read/write/delete — so this handle has to permit all three or our own reader is
            // locked out by our own writer.
            FILE_SHARE_MODE(FILE_SHARE_READ.0 | FILE_SHARE_WRITE.0 | FILE_SHARE_DELETE.0),
            Some(&attributes),
            CREATE_NEW,
            FILE_ATTRIBUTE_NORMAL,
            None,
        )
    };
    // Freed whether or not the create succeeded — `ConvertString…` allocates with `LocalAlloc` and
    // the caller owns it either way.
    unsafe { LocalFree(HLOCAL(descriptor.0)) };

    let handle = handle.map_err(|e| Error(format!("{}: {e}", path.display())))?;
    if handle == INVALID_HANDLE_VALUE {
        return Err(Error(format!("{}: invalid handle", path.display())));
    }
    Ok(unsafe { std::fs::File::from_raw_handle(handle.0) })
}

/// The calling user's SID, as the string SDDL wants.
fn current_user_sid() -> Result<String> {
    let mut token = HANDLE::default();
    unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) }
        .map_err(|e| Error(format!("process token: {e}")))?;

    // Asked for its size first, because a SID is variable-length and `TOKEN_USER` is a header with
    // a pointer into the same buffer — so the buffer has to be one allocation, not two.
    let mut needed = 0u32;
    let _ = unsafe { GetTokenInformation(token, TokenUser, None, 0, &mut needed) };
    let mut buf = vec![0u8; needed.max(1) as usize];
    let info = unsafe {
        GetTokenInformation(
            token,
            TokenUser,
            Some(buf.as_mut_ptr() as *mut c_void),
            needed,
            &mut needed,
        )
    };
    let closed = unsafe { CloseHandle(token) };
    info.map_err(|e| Error(format!("token user: {e}")))?;
    closed.map_err(|e| Error(format!("{e}")))?;

    let user = unsafe { &*(buf.as_ptr() as *const TOKEN_USER) };
    let mut text = PWSTR::null();
    unsafe { ConvertSidToStringSidW(user.User.Sid, &mut text) }
        .map_err(|e| Error(format!("sid: {e}")))?;
    let sid = unsafe { text.to_string() }.map_err(|e| Error(format!("sid text: {e}")))?;
    unsafe { LocalFree(HLOCAL(text.0 as *mut c_void)) };
    Ok(sid)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rows::RowSource;

    /// `cargo test` runs with a redirected or piped standard input, never a console, so the only
    /// thing this can assert without a harness is that the classification is *decided* — no panic,
    /// no invalid handle leaking through as a readable stream.
    #[test]
    fn the_standard_input_handle_is_classified_without_blocking() {
        let kind = stdin();
        assert!(matches!(
            kind,
            Stdin::Pipe | Stdin::Disk | Stdin::Console | Stdin::None
        ));
        // Whatever it is, asking must not have read a byte or waited — this test returning at all
        // is the assertion, and §4.2's "do not block" on a console is the case it guards.
        assert_eq!(kind.readable(), matches!(kind, Stdin::Pipe | Stdin::Disk));
    }

    /// The DACL of `path`, read back off the object rather than assumed from what was written.
    fn dacl_of(path: &Path) -> String {
        use windows::Win32::Security::Authorization::{
            ConvertSecurityDescriptorToStringSecurityDescriptorW, GetNamedSecurityInfoW,
            SE_FILE_OBJECT,
        };
        use windows::Win32::Security::{DACL_SECURITY_INFORMATION, OBJECT_SECURITY_INFORMATION};

        let wide_path = wide(path);
        let mut descriptor = PSECURITY_DESCRIPTOR::default();
        let status = unsafe {
            GetNamedSecurityInfoW(
                PCWSTR(wide_path.as_ptr()),
                SE_FILE_OBJECT,
                OBJECT_SECURITY_INFORMATION(DACL_SECURITY_INFORMATION.0),
                None,
                None,
                None,
                None,
                &mut descriptor,
            )
        };
        assert!(status.is_ok(), "reading the DACL of {path:?}: {status:?}");

        let mut text = PWSTR::null();
        let mut len = 0u32;
        unsafe {
            ConvertSecurityDescriptorToStringSecurityDescriptorW(
                descriptor,
                SDDL_REVISION_1,
                OBJECT_SECURITY_INFORMATION(DACL_SECURITY_INFORMATION.0),
                &mut text,
                Some(&mut len),
            )
        }
        .expect("descriptor to SDDL");
        let sddl = unsafe { text.to_string() }.expect("sddl text");
        unsafe { LocalFree(HLOCAL(text.0 as *mut c_void)) };
        unsafe { LocalFree(HLOCAL(descriptor.0)) };
        sddl
    }

    /// §13.2's restrictive DACL — **as a difference from an ordinary file in the same directory**,
    /// not as a property of the spill alone.
    ///
    /// The first version of this test asserted only the spill's own DACL, and a negative control
    /// showed why that is not enough: removing the `P` from the SDDL changed nothing, because
    /// supplying any explicit descriptor already blocks inheritance. The assertion held while the
    /// thing it named was gone — the shape of a test that cannot fail.
    ///
    /// The control file is what fixes it. If this machine's `%TEMP%` granted nobody else anything
    /// the comparison would be vacuous, so the test says so and fails rather than passing on an
    /// empty premise. Dropping the `SECURITY_ATTRIBUTES` argument entirely now fails it on `SY`.
    #[test]
    fn a_spill_is_readable_only_by_the_user_who_made_it() {
        let spill = Spill::create().expect("create a spill");
        assert!(spill.path().exists());
        let sddl = dacl_of(spill.path());

        // An ordinary file, created the ordinary way, in the same directory.
        let control = std::env::temp_dir().join("tailhawk-dacl-control.txt");
        let _ = std::fs::remove_file(&control);
        std::fs::write(&control, b"control").expect("write the control file");
        let inherited = dacl_of(&control);
        let _ = std::fs::remove_file(&control);

        let sid = current_user_sid().expect("our own sid");
        // SDDL abbreviates the built-in Administrator (RID 500) to `LA` rather than spelling its
        // SID — which is who a CI runner is.
        let as_alias = sid.ends_with("-500") && sddl.contains(";;;LA)");
        assert!(
            sddl.contains(&sid) || as_alias,
            "the owner should be granted access: {sddl}"
        );

        // Well-known principals a spill must not grant: SYSTEM (SY), BUILTIN\Administrators (BA),
        // Users (BU), Everyone (S-1-1-0), Authenticated Users (S-1-5-11).
        let others = ["SY)", "BA)", "BU)", "S-1-1-0", "S-1-5-11"];
        assert!(
            others.iter().any(|p| inherited.contains(p)),
            "this machine's %TEMP% grants nobody else anything, so the comparison proves nothing \
             — the control file's DACL is {inherited}"
        );
        for principal in others {
            assert!(
                !sddl.contains(principal),
                "{principal} is granted access to the spill but must not be.\n  spill:    {sddl}\n  \
                 ordinary: {inherited}"
            );
        }
    }

    /// §13.2: "deleted on clean exit". Dropping is what a clean exit does here.
    #[test]
    fn a_spill_deletes_itself() {
        let path = {
            let spill = Spill::create().expect("create");
            let path = spill.path().to_path_buf();
            assert!(path.exists());
            path
        };
        assert!(!path.exists(), "the spill outlived its owner");
    }

    /// …unless the user asked to keep it.
    #[test]
    fn a_kept_spill_survives() {
        let path = {
            let mut spill = Spill::create().expect("create");
            spill.keep();
            spill.path().to_path_buf()
        };
        assert!(path.exists());
        std::fs::remove_file(&path).expect("clean up");
    }

    /// §13.2's "reaped on next launch if orphaned" — and the half of it that matters, which is that
    /// a spill belonging to a *running* instance is not reaped. A pid check would get this wrong,
    /// because pids are recycled; holding the file is the fact that is actually true.
    #[test]
    fn reaping_removes_an_orphan_and_leaves_a_live_spill_alone() {
        let orphan = {
            let mut spill = Spill::create().expect("orphan");
            spill.keep();
            spill.path().to_path_buf()
        };
        let live = Spill::create().expect("live");
        let live_path = live.path().to_path_buf();

        // Not asserted on the count: the suite runs these in parallel and another test reaping
        // the same `%TEMP%` can get there first. What is asserted is what this test is about.
        reap_orphans();
        assert!(!orphan.exists(), "the orphan should have been reaped");
        assert!(
            live_path.exists(),
            "a spill still held open must survive a reap"
        );
    }

    /// The spill is a file like any other, which is the claim the whole design rests on: everything
    /// downstream — detection, indexing, following, the viewport — works on it unchanged.
    #[test]
    fn a_spill_is_an_ordinary_file_to_the_rest_of_the_product() {
        use crate::set::LogSet;

        let mut spill = Spill::create().expect("create");
        spill.write(b"first line\nsecond line\n").expect("write");

        let mut set = LogSet::open_single(spill.path()).expect("open the spill as a source");
        assert_eq!(set.total_rows(), 2);
        set.fetch(0, 2, false).expect("fetch");
        assert_eq!(set.row_text(0), Some("first line"));

        // And it follows, because nothing here knows it is not a log file.
        spill.write("third line\n".as_bytes()).expect("append");
        let polled = set.settle();
        assert_eq!(polled.lines_added, 1);
        assert_eq!(set.total_rows(), 3);
    }

    /// §4.2: "encoding is detected on the piped bytes exactly as for a file — PowerShell's
    /// native-command pipeline has historically emitted UTF-16 and OEM codepages." Nothing in this
    /// module implements that; the test exists to show the spill does not get in its way.
    #[test]
    fn a_utf16_stream_is_detected_from_the_spilled_bytes() {
        use crate::set::LogSet;

        let mut spill = Spill::create().expect("create");
        let mut bytes = vec![0xFF, 0xFE];
        for unit in "piped wide\n".encode_utf16() {
            bytes.extend_from_slice(&unit.to_le_bytes());
        }
        spill.write(&bytes).expect("write");

        let mut set = LogSet::open_single(spill.path()).expect("open");
        set.fetch(0, 1, false).expect("fetch");
        assert_eq!(set.row_text(0), Some("piped wide"));
    }

    /// **Two concurrent instances' spills look exactly like two generations of one rolling log.**
    /// `tailhawk-spill-<pid>-<n>.log` has the same literal skeleton whatever the numbers are, so
    /// `LogSet::open` would splice another process's piped stream into this one's scrollback —
    /// customer log content from the wrong source, presented as continuous history.
    #[test]
    fn a_spill_never_adopts_another_instances_spill_as_a_rolling_set() {
        use crate::pattern::RollingSet;
        use crate::set::LogSet;

        // **The hazard, on the naming scheme itself.** Two different processes' first spills differ
        // only in the pid, and `pattern.rs` matches on the literal skeleton with the numbers taken
        // out — so this is a textbook `Varying` set. Asserted on literal names rather than on live
        // spills because the counter is global and a later one would not be padded, which would
        // make the fixture pass for the wrong reason on some runs and not others.
        let ours = format!("{SPILL_PREFIX}1111-00{SPILL_SUFFIX}");
        let theirs = format!("{SPILL_PREFIX}2222-00{SPILL_SUFFIX}");
        let inferred = RollingSet::infer(&ours, &[ours.clone(), theirs.clone()]);
        assert_eq!(
            inferred.members(),
            [ours.as_str(), theirs.as_str()],
            "the hazard this guards against has to be real for the guard to mean anything"
        );

        // And the guard: a spill opened as a spill is one file, whatever is beside it.
        let mut theirs = Spill::create().expect("their spill");
        theirs.write(b"someone else's stream\n").expect("write");
        let mut mine = Spill::create().expect("our spill");
        mine.write(b"our stream\n").expect("write");

        let opened = LogSet::open_single(mine.path()).expect("open as one file");
        assert_eq!(opened.members().len(), 1);
        assert_eq!(opened.total_rows(), 1);
    }

    /// **The naming is load-bearing and this is what holds it.** `pattern.rs` decides the order of
    /// a set from its names alone, and a spill that rolled into names it ordered newest-first
    /// would present the tail's history backwards — §5.5b's silent reversal, arrived at through
    /// our own file naming rather than through someone else's.
    #[test]
    fn parts_are_a_rolling_set_that_reads_oldest_first() {
        use crate::pattern::{Order, RollingSet};

        let names: Vec<String> = (1..=11).map(part_name).collect();
        let set = RollingSet::infer(&names[0], &names);
        assert_eq!(set.order(), Order::Ascending);
        assert_eq!(set.members(), names.as_slice());
        assert_eq!(set.newest(), part_name(11));
        // Ten and two are where an unpadded sequence sorts wrong, so they are what to assert on.
        assert!(
            names[1] < names[9],
            "part two must sort before part ten: {} vs {}",
            names[1],
            names[9]
        );
    }

    #[test]
    fn a_spill_inside_its_cap_loses_nothing() {
        assert_eq!(excess(&[10, 10, 10], 100), 0);
        assert_eq!(
            excess(&[40, 40, 20], 100),
            0,
            "exactly the cap is inside it"
        );
    }

    #[test]
    fn the_oldest_parts_are_what_go() {
        assert_eq!(excess(&[40, 40, 40], 100), 1);
        assert_eq!(excess(&[40, 40, 40, 40], 100), 2);
    }

    /// The live part is never deleted, whatever it costs — see [`excess`].
    #[test]
    fn the_live_part_is_never_deletable() {
        assert_eq!(excess(&[500], 100), 0);
        assert_eq!(
            excess(&[500, 500], 100),
            1,
            "everything but the live part goes, and no further"
        );
        assert_eq!(excess(&[], 100), 0);
    }

    #[test]
    fn a_spill_set_is_readable_only_by_the_user_who_made_it() {
        let set = SpillSet::create().expect("create a rolling spill");
        let control = std::env::temp_dir().join("tailhawk-dacl-control-set.txt");
        let _ = std::fs::remove_file(&control);
        std::fs::write(&control, b"control").expect("control file");

        let ordinary = dacl_of(&control);
        assert!(
            ordinary.contains("SY") || ordinary.contains("BA"),
            "this machine's %TEMP% grants nobody else anything, so the comparison would be vacuous: {ordinary}"
        );
        for path in [set.dir().to_path_buf(), set.first_part()] {
            let sddl = dacl_of(&path);
            assert!(
                !sddl.contains("SY") && !sddl.contains("BA"),
                "{} is readable by more than its owner: {sddl}",
                path.display()
            );
        }
        let _ = std::fs::remove_file(&control);
    }

    #[test]
    fn a_rolling_spill_deletes_itself_whole() {
        let dir;
        {
            let set = SpillSet::create().expect("create");
            dir = set.dir().to_path_buf();
            set.writer().append("a line\n").expect("append");
            assert!(set.first_part().exists());
        }
        assert!(!dir.exists(), "{} outlived its SpillSet", dir.display());
    }

    /// A part fills, the next one starts, and **nothing that was written is lost in the move** —
    /// the rolling is a place to put new bytes, not a reason to drop old ones.
    #[test]
    fn a_full_part_rolls_into_the_next_one() {
        let set = SpillSet::create().expect("create");
        let mut writer = set.writer().with_limits(64, 1 << 30);

        for n in 0..20 {
            writer.append(&format!("line {n}\n")).expect("append");
        }

        let parts = parts_of(set.dir());
        assert!(
            parts.len() > 1,
            "twenty lines over a 64-byte part must have rolled: {parts:?}"
        );
        assert_eq!(parts[0].0, set.first_part(), "part one is the oldest");
        let written: String = parts
            .iter()
            .map(|(path, _)| std::fs::read_to_string(path).expect("read a part"))
            .collect();
        for n in 0..20 {
            assert!(
                written.contains(&format!("line {n}\n")),
                "line {n} was lost"
            );
        }
    }

    /// The bound itself: parts keep arriving, the oldest keep going, and the total stays inside
    /// the cap rather than growing for as long as the tail runs.
    #[test]
    fn the_oldest_parts_go_when_the_spill_is_over_its_cap() {
        let set = SpillSet::create().expect("create");
        let mut writer = set.writer().with_limits(64, 256);

        let mut dropped = 0;
        for n in 0..200 {
            dropped += writer.append(&format!("line {n}\n")).expect("append");
        }

        assert!(
            dropped > 0,
            "nothing was ever deleted, so nothing was bounded"
        );
        let parts = parts_of(set.dir());
        let total: u64 = parts.iter().map(|(_, len)| len).sum();
        assert!(
            total <= 256 + 64,
            "the spill is {total} bytes against a 256-byte cap: {parts:?}"
        );
        assert!(!set.first_part().exists(), "part one should have gone");
        let newest = std::fs::read_to_string(&parts.last().expect("a live part").0).expect("read");
        assert!(
            newest.contains("line 199\n"),
            "the newest records must be the ones kept: {newest}"
        );
    }

    /// **What the shell actually does with it**: open part one as a document and let the set
    /// follow. A roll must appear as more rows in the same document, not as a second one.
    #[test]
    fn a_rolling_spill_reads_as_one_growing_document() {
        use crate::set::LogSet;

        let set = SpillSet::create().expect("create");
        let mut writer = set.writer().with_limits(64, 1 << 30);
        for n in 0..4 {
            writer.append(&format!("line {n}\n")).expect("append");
        }

        let mut opened = LogSet::open(&set.first_part()).expect("open part one");
        opened.settle();
        let before = opened.total_rows();
        assert!(before >= 4, "the opening lines are there: {before}");

        for n in 4..40 {
            writer.append(&format!("line {n}\n")).expect("append");
        }
        assert!(
            parts_of(set.dir()).len() > 1,
            "the fixture must have rolled for this to be testing anything"
        );
        opened.rescan();
        opened.settle();

        assert_eq!(opened.total_rows(), 40, "every line, across every part");
        assert!(
            opened.members().len() > 1,
            "the roll should have attached a new member, not opened a new document"
        );
    }

    /// The hazard `a_spill_never_adopts_another_instances_spill_as_a_rolling_set` names, closed
    /// for the remote path by construction rather than by remembering to call `open_single`: the
    /// only names beside a part are that source's own parts.
    #[test]
    fn a_rolling_spill_never_adopts_another_instances_parts() {
        use crate::set::LogSet;

        let theirs = SpillSet::create().expect("their spill");
        theirs
            .writer()
            .append("someone else's records\n")
            .expect("append");
        let mine = SpillSet::create().expect("our spill");
        mine.writer().append("our records\n").expect("append");

        let opened = LogSet::open(&mine.first_part()).expect("open ours");
        assert_eq!(opened.members().len(), 1);
        assert_eq!(opened.total_rows(), 1);

        // **The assertion that can actually fail is this one.** Two sets are in two directories by
        // construction, so the open above could not splice them however the code were written; what
        // a future change could do is put the parts back in `%TEMP%` beside every other spill,
        // which is exactly the arrangement that made the splice possible. So the property under
        // test is that a part is *not* a sibling of other spills.
        assert_ne!(
            mine.first_part().parent(),
            Some(std::env::temp_dir().as_path()),
            "parts must live in a directory of their own, not beside every other spill"
        );
        assert_ne!(theirs.dir(), mine.dir());
    }

    #[test]
    fn reaping_removes_an_orphan_spill_directory_and_leaves_a_live_one_alone() {
        use crate::set::LogSet;

        let orphan = {
            let mut set = SpillSet::create().expect("orphan");
            set.writer().append("abandoned\n").expect("append");
            set.hold = None;
            set.remove_on_drop = false;
            set.dir().to_path_buf()
        };
        let live = SpillSet::create().expect("live");
        live.writer().append("in use\n").expect("append");
        let reader = LogSet::open(&live.first_part()).expect("open the live one");

        // Not asserted on the count, for the reason the file-reaping test above gives.
        reap_orphans();
        assert!(!orphan.exists(), "{} survived reaping", orphan.display());
        assert!(
            live.first_part().exists(),
            "a spill someone is reading was reaped"
        );
        assert_eq!(reader.total_rows(), 1);
    }

    /// **The two halves of the design, exercised together — which is the whole claim.** A reader
    /// holds a part; the writer deletes it; the name has to leave the directory at once, the set
    /// has to retire the member, and the row space has to stay coherent across the renumber.
    ///
    /// Each half was tested alone and neither test could have caught the other's failure: the
    /// reading test ran with retention switched off, and the retention test had no reader. The
    /// mechanism they both rest on is `file.rs`'s `FILE_SHARE_DELETE` plus `std`'s choice of
    /// POSIX-semantics delete — if either changed, the deleted name would linger as delete-pending,
    /// the member would never retire, its bytes would never be freed, and the bound would quietly
    /// stop working with the whole suite still green.
    #[test]
    fn a_part_is_deleted_under_a_reader_and_the_document_stays_coherent() {
        use crate::set::LogSet;

        let set = SpillSet::create().expect("create");
        let mut writer = set.writer().with_limits(64, 1 << 30);
        for n in 0..20 {
            writer.append(&format!("line {n}\n")).expect("append");
        }
        let parts = parts_of(set.dir());
        assert!(parts.len() > 1, "the fixture must have rolled: {parts:?}");

        let mut opened = LogSet::open(&set.first_part()).expect("open part one");
        opened.rescan();
        opened.settle();
        assert_eq!(opened.total_rows(), 20, "every line, before anything goes");
        let oldest_rows = opened.members()[0].line_count();
        assert!(oldest_rows > 0);

        // The reader is holding part one. Retention takes it anyway, which is the thing that has
        // to work: a `LogFile` shares delete, so this must succeed rather than fail as busy. The
        // cap is set from the parts on disk so that exactly the oldest is over it — a fixture that
        // deleted everything but the live part would prove less.
        let held: u64 = parts.iter().map(|(_, len)| len).sum();
        writer.cap_to(held - parts[0].1 + 32);
        writer.append("line 20\n").expect("append past the cap");
        assert!(
            !set.first_part().exists(),
            "the deleted name must leave the directory at once, not linger as delete-pending"
        );

        opened.rescan();
        let polled = opened.settle();
        assert_eq!(
            polled.retired,
            ["part-000001.log"],
            "the set must notice the part it was holding has gone"
        );
        assert_eq!(
            polled.retired_rows,
            [(0, oldest_rows)],
            "and say where those rows were, so a caller can move what it addresses"
        );
        assert_eq!(
            opened.total_rows(),
            21 - oldest_rows,
            "the row space is what survives, renumbered"
        );
        opened.fetch(0, 2, false).expect("fetch");
        assert_eq!(
            opened.row_text(0),
            Some(format!("line {oldest_rows}").as_str()),
            "row zero is now the oldest surviving line"
        );
    }
}
