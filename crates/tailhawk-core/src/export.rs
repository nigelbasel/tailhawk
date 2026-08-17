//! Export and tee — E21, `SPEC.md` §11.1's "export the filtered view", Hoo WinTail's tee.
//!
//! A pass on a worker, shaped like [`crate::find`] and [`crate::sieve`]: over a snapshot of the
//! set, walking `[from, to)` with the same [`crate::search::each_line`] the search and the filter
//! use, writing every row the view shows to a file. "The view shows" is a sorted list of surviving
//! set-wide rows when a filter is on, or every row when it is not; a moving cursor over that list
//! keeps a filtered export to one pass and no per-row search.
//!
//! ## A tee is the export, continued
//!
//! Hoo WinTail's tee writes matching lines to a file *as they arrive*. Here that is the same job
//! started again over the growth: the caller keeps the row the last export covered up to and, once
//! the filter pass has judged the rows beyond it, starts another `[covered, total)` export that
//! **appends**. Nothing is teed that the filter had not yet seen, and no frame waits on a write.
//!
//! ## What is written
//!
//! The decoded text of each line, as UTF-8, ending in `\r\n`. That is a transformation of the
//! bytes in the log — an export is for reading elsewhere, and the raw bytes are in the log where
//! §5.6 keeps them. It is said here so nobody takes an export for a copy.

use std::io::Write;
use std::path::Path;
use std::sync::mpsc::{Receiver, Sender};

use crate::find::{Excerpt, Outcome};
use crate::search::{each_line, Cancel};
use crate::Result;

/// What the worker reports.
#[derive(Debug)]
pub enum Update {
    /// Rows written so far, and rows examined.
    Progress { written: u64, scanned: u64 },
    /// The last message: how it ended, and how many rows were written.
    Finished(Outcome, u64),
}

/// A pass in flight. Dropping it cancels and joins the worker, as [`crate::find::Running`] does.
pub struct Running {
    cancel: Cancel,
    updates: Receiver<Update>,
    worker: Option<std::thread::JoinHandle<()>>,
}

impl Running {
    /// Everything reported since the last call. Never blocks.
    pub fn drain(&self) -> impl Iterator<Item = Update> + '_ {
        self.updates.try_iter()
    }

    pub fn cancel(&self) {
        self.cancel.cancel();
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        self.cancel.cancel();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

/// Which rows to write.
#[derive(Clone, Debug)]
pub enum Keep {
    /// Every row in the range.
    All,
    /// Only these set-wide rows, **sorted ascending** — a filter's survivors.
    Rows(Vec<u64>),
}

/// Starts writing the rows `[from, to)` of the snapshot that `keep` keeps to `path` — created or
/// truncated, or appended to when `append` — on a worker. Errors opening the file come back here;
/// errors during the pass come back as [`Outcome::Failed`].
pub fn start(
    members: Vec<Excerpt>,
    keep: Keep,
    from: u64,
    to: u64,
    path: &Path,
    append: bool,
) -> Result<Running> {
    let file = std::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .append(append)
        .truncate(!append)
        .open(path)
        .map_err(|e| crate::Error(format!("opening {}: {e}", path.display())))?;
    let cancel = Cancel::new();
    let (tx, updates) = std::sync::mpsc::channel();
    let worker = {
        let cancel = cancel.clone();
        std::thread::Builder::new()
            .name("tailhawk-export".to_owned())
            .spawn(move || run(members, keep, from, to, file, &cancel, &tx))
            .map_err(|e| crate::Error(format!("starting the export worker: {e}")))?
    };
    Ok(Running {
        cancel,
        updates,
        worker: Some(worker),
    })
}

/// How often progress is reported, in rows examined.
const REPORT_EVERY: u64 = 20_000;

fn run(
    members: Vec<Excerpt>,
    keep: Keep,
    from: u64,
    to: u64,
    file: std::fs::File,
    cancel: &Cancel,
    tx: &Sender<Update>,
) {
    let mut out = std::io::BufWriter::new(file);
    let mut written = 0u64;
    let mut scanned = 0u64;
    let mut outcome = Outcome::Complete;
    // The moving cursor over the survivor list: rows arrive ascending, so it only moves forward.
    let mut cursor = match &keep {
        Keep::All => 0,
        Keep::Rows(rows) => rows.partition_point(|&r| r < from),
    };
    for member in &members {
        if cancel.cancelled() {
            outcome = Outcome::Cancelled;
            break;
        }
        let member_rows = member.index.line_count();
        let member_from = from.saturating_sub(member.first_row).min(member_rows);
        let member_to = to.saturating_sub(member.first_row).min(member_rows);
        if member_from >= member_to {
            continue;
        }
        let mut write_error: Option<std::io::Error> = None;
        let mut since_report = 0u64;
        let walked = each_line(
            &*member.file,
            member.charset,
            &member.index,
            member_from,
            member_to,
            cancel.flag(),
            |local, text| {
                if write_error.is_some() {
                    return;
                }
                let row = member.first_row + local;
                scanned += 1;
                since_report += 1;
                let wanted = match &keep {
                    Keep::All => true,
                    Keep::Rows(rows) => {
                        while cursor < rows.len() && rows[cursor] < row {
                            cursor += 1;
                        }
                        cursor < rows.len() && rows[cursor] == row
                    }
                };
                if wanted {
                    if let Err(e) = out
                        .write_all(text.as_bytes())
                        .and_then(|()| out.write_all(b"\r\n"))
                    {
                        write_error = Some(e);
                        return;
                    }
                    written += 1;
                }
                if since_report >= REPORT_EVERY {
                    since_report = 0;
                    let _ = tx.send(Update::Progress { written, scanned });
                }
            },
        );
        if let Some(e) = write_error {
            outcome = Outcome::Failed(format!("writing the export: {e}"));
            break;
        }
        if let Err(e) = walked {
            outcome = Outcome::Failed(e.to_string());
            break;
        }
        if cancel.cancelled() {
            outcome = Outcome::Cancelled;
            break;
        }
    }
    if let Err(e) = out.flush() {
        if matches!(outcome, Outcome::Complete) {
            outcome = Outcome::Failed(format!("writing the export: {e}"));
        }
    }
    let _ = tx.send(Update::Finished(outcome, written));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::encoding::Charset;
    use crate::file::LogFile;
    use crate::indexer::{build_index, IndexOptions};
    use std::sync::Arc;

    fn excerpt(path: &Path, first_row: u64) -> Excerpt {
        let file = LogFile::open(path).expect("open");
        let end = file.len().expect("len");
        let index = build_index(&file, Charset::UTF_8, 0, end, &IndexOptions::default())
            .expect("index");
        Excerpt {
            file: Arc::new(file),
            charset: Charset::UTF_8,
            index,
            first_row,
        }
    }

    fn finish(running: &Running) -> (Outcome, u64) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        loop {
            for update in running.drain() {
                if let Update::Finished(outcome, written) = update {
                    return (outcome, written);
                }
            }
            assert!(std::time::Instant::now() < deadline, "the export did not finish");
            std::thread::yield_now();
        }
    }

    #[test]
    fn every_row_or_only_the_kept_ones_and_appending_continues_the_file() {
        let dir = std::env::temp_dir().join("tailhawk_export_test");
        let _ = std::fs::create_dir_all(&dir);
        let log = dir.join("in.log");
        let out = dir.join("out.txt");
        let text: String = (0..1000).map(|i| format!("line {i}\n")).collect();
        std::fs::write(&log, &text).expect("write");

        let (outcome, written) =
            finish(&start(vec![excerpt(&log, 0)], Keep::All, 0, 1000, &out, false).expect("start"));
        assert!(matches!(outcome, Outcome::Complete), "{outcome:?}");
        assert_eq!(written, 1000);
        let got = std::fs::read_to_string(&out).expect("read");
        assert_eq!(got.lines().count(), 1000);
        assert!(got.starts_with("line 0\r\nline 1\r\n"), "CRLF, decoded text");

        let kept = vec![3, 4, 500, 999];
        let (outcome, written) = finish(
            &start(vec![excerpt(&log, 0)], Keep::Rows(kept), 0, 1000, &out, false).expect("start"),
        );
        assert!(matches!(outcome, Outcome::Complete));
        assert_eq!(written, 4);
        assert_eq!(
            std::fs::read_to_string(&out).expect("read"),
            "line 3\r\nline 4\r\nline 500\r\nline 999\r\n"
        );

        // A tee's second leg: the growth [990, 1000), appended, only the kept rows in it.
        let (_, written) = finish(
            &start(
                vec![excerpt(&log, 0)],
                Keep::Rows(vec![3, 991, 992]),
                990,
                1000,
                &out,
                true,
            )
            .expect("start"),
        );
        assert_eq!(written, 2, "the survivor before the range is not written again");
        assert!(std::fs::read_to_string(&out)
            .expect("read")
            .ends_with("line 999\r\nline 991\r\nline 992\r\n"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_set_of_two_members_is_one_row_space() {
        let dir = std::env::temp_dir().join("tailhawk_export_set_test");
        let _ = std::fs::create_dir_all(&dir);
        let a = dir.join("a.log");
        let b = dir.join("b.log");
        std::fs::write(&a, "a0\na1\na2\n").expect("write");
        std::fs::write(&b, "b0\nb1\n").expect("write");
        let out = dir.join("out.txt");
        let (outcome, written) = finish(
            &start(
                vec![excerpt(&a, 0), excerpt(&b, 3)],
                Keep::Rows(vec![1, 3, 4]),
                0,
                5,
                &out,
                false,
            )
            .expect("start"),
        );
        assert!(matches!(outcome, Outcome::Complete));
        assert_eq!(written, 3);
        assert_eq!(std::fs::read_to_string(&out).expect("read"), "a1\r\nb0\r\nb1\r\n");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
