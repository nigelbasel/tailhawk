//! Column sort and top-N — E22, `SPEC.md` §11.4.
//!
//! A pass on a worker, shaped like [`crate::export`]: over a snapshot of the set, walking
//! `[from, to)` with the same [`crate::search::each_line`] the search, the filter and the export
//! use, taking one **key** from each row the view shows and reporting the rows in key order.
//! "The view shows" is [`Keep`] — every row, or a filter's survivors — exactly as the export reads
//! it, with the same moving cursor.
//!
//! ## Two shapes, one pass
//!
//! §11.4 caps a **sort** at a filtered set of 2 M rows, because a sort holds a key per row and
//! *N* keys of a 100-million-line file is a machine's memory. The cap is the caller's to enforce —
//! it knows the row count before it starts — and this module trusts it. **Top-N** has no cap: it
//! is "a heap over a scan", holding *N* keys however many rows go by, which is what serves "show me
//! the slowest requests" on a file that a sort could not. Both come out of the same walk; the
//! only difference is whether every key is kept or the heap decides.
//!
//! ## What a key is
//!
//! The column's text, read through the format: a level column keys on its severity number so
//! `Warning` sorts above `Information` rather than after it alphabetically; a timestamp column
//! keys on the parsed instant so `dd/MM/yyyy` dates order by date (a level word or a stamp the
//! format cannot read is *missing*, not text — so an unknown spelling never floats to the top);
//! anything that reads as a number is a number; the rest is text, compared case-insensitively.
//! Numbers come before text ascending (after it descending), and a row whose column did not
//! participate — a continuation line, a record the format could not read — comes **last in either
//! direction**, so the rows worth seeing are at the top whichever
//! way the user asked for them. Ties keep file order.

use std::cmp::Ordering;
use std::collections::BinaryHeap;
use std::sync::mpsc::{Receiver, Sender};

pub use crate::export::Keep;
use crate::find::{Excerpt, Outcome};
use crate::format::{Format, LEVEL, TS};
use crate::search::{each_line, Cancel};
use crate::Result;

/// What the worker reports.
#[derive(Debug)]
pub enum Update {
    /// Rows examined so far.
    Progress { scanned: u64 },
    /// The last message: how it ended, and the rows in order — every kept row for a sort, at most
    /// *N* for a top-N. Empty when the pass failed before it read anything.
    Finished(Outcome, Vec<u64>),
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

/// What order to put the rows in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Order {
    /// An index into [`Format::columns`].
    pub column: usize,
    pub descending: bool,
    /// Keep only the first *N* rows of the order — the heap. `None` is a whole sort.
    pub top: Option<usize>,
}

/// One row's sort key. See the module note for what each column becomes.
#[derive(Clone, Debug, PartialEq)]
pub enum Key {
    /// An instant or a severity: exact, so two stamps 100 ns apart do not tie.
    Int(i64),
    Number(f64),
    Text(String),
    /// The column did not participate in this row.
    Missing,
}

/// The key of `column` in `line` under `format`.
pub fn key_of(format: &Format, column: usize, line: &str) -> Key {
    let Some(name) = format.columns.get(column) else {
        return Key::Missing;
    };
    let Some(fields) = format.fields(line) else {
        return Key::Missing;
    };
    let Some(Some(range)) = fields.get(column) else {
        return Key::Missing;
    };
    let text = line[range.clone()].trim();
    if text.is_empty() {
        return Key::Missing;
    }
    if *name == LEVEL {
        return match format.level.parse(text) {
            Some(severity) => Key::Int(i64::from(severity.get())),
            None => Key::Missing,
        };
    }
    if *name == TS {
        return match format.stamp.parse(text) {
            Some(stamp) => Key::Int(stamp.unix_nanos),
            None => Key::Missing,
        };
    }
    if let Ok(n) = text.parse::<f64>() {
        if n.is_finite() {
            return Key::Number(n);
        }
    }
    Key::Text(text.to_lowercase())
}

/// Where `a` goes relative to `b` in the requested direction: numbers before text, a missing key
/// last either way, ties left to the caller (who breaks them by row).
pub fn compare(a: &Key, b: &Key, descending: bool) -> Ordering {
    match (a, b) {
        (Key::Missing, Key::Missing) => Ordering::Equal,
        (Key::Missing, _) => Ordering::Greater,
        (_, Key::Missing) => Ordering::Less,
        _ => {
            let natural = match (a, b) {
                (Key::Int(x), Key::Int(y)) => x.cmp(y),
                (Key::Number(x), Key::Number(y)) => x.total_cmp(y),
                (Key::Int(x), Key::Number(y)) => (*x as f64).total_cmp(y),
                (Key::Number(x), Key::Int(y)) => x.total_cmp(&(*y as f64)),
                (Key::Int(_) | Key::Number(_), Key::Text(_)) => Ordering::Less,
                (Key::Text(_), Key::Int(_) | Key::Number(_)) => Ordering::Greater,
                (Key::Text(x), Key::Text(y)) => x.cmp(y),
                _ => Ordering::Equal,
            };
            if descending {
                natural.reverse()
            } else {
                natural
            }
        }
    }
}

/// Starts ordering the rows `[from, to)` of the snapshot that `keep` keeps by `order`, on a
/// worker. The caller has checked §11.4's cap for a whole sort.
pub fn start(
    members: Vec<Excerpt>,
    keep: Keep,
    from: u64,
    to: u64,
    format: &'static Format,
    order: Order,
) -> Result<Running> {
    let cancel = Cancel::new();
    let (tx, updates) = std::sync::mpsc::channel();
    let worker = {
        let cancel = cancel.clone();
        std::thread::Builder::new()
            .name("tailhawk-sort".to_owned())
            .spawn(move || run(members, keep, from, to, format, order, &cancel, &tx))
            .map_err(|e| crate::Error(format!("starting the sort worker: {e}")))?
    };
    Ok(Running {
        cancel,
        updates,
        worker: Some(worker),
    })
}

/// How often progress is reported, in rows examined.
const REPORT_EVERY: u64 = 20_000;

/// One keyed row. Its `Ord` is **its place in the output** — earlier is less — so a max-heap's top
/// is the row that would come out last, which is the one a better row displaces.
struct Item {
    key: Key,
    row: u64,
    descending: bool,
}

impl PartialEq for Item {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}

impl Eq for Item {}

impl PartialOrd for Item {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Item {
    fn cmp(&self, other: &Self) -> Ordering {
        compare(&self.key, &other.key, self.descending).then(self.row.cmp(&other.row))
    }
}

/// The keys as they are gathered: all of them, or the heap.
enum Gather {
    All(Vec<Item>),
    Top(usize, BinaryHeap<Item>),
}

impl Gather {
    fn push(&mut self, item: Item) {
        match self {
            Gather::All(items) => items.push(item),
            Gather::Top(n, heap) => {
                if heap.len() < *n {
                    heap.push(item);
                } else if heap.peek().is_some_and(|worst| item < *worst) {
                    heap.pop();
                    heap.push(item);
                }
            }
        }
    }

    fn into_order(self) -> Vec<u64> {
        let mut items = match self {
            Gather::All(items) => items,
            Gather::Top(_, heap) => heap.into_vec(),
        };
        items.sort();
        items.into_iter().map(|item| item.row).collect()
    }
}

#[allow(clippy::too_many_arguments)]
fn run(
    members: Vec<Excerpt>,
    keep: Keep,
    from: u64,
    to: u64,
    format: &'static Format,
    order: Order,
    cancel: &Cancel,
    tx: &Sender<Update>,
) {
    let mut gather = match order.top {
        Some(n) => Gather::Top(n.max(1), BinaryHeap::with_capacity(n.max(1) + 1)),
        None => Gather::All(Vec::new()),
    };
    let mut scanned = 0u64;
    let mut outcome = Outcome::Complete;
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
        let mut since_report = 0u64;
        let walked = each_line(
            &*member.file,
            member.charset,
            &member.index,
            member_from,
            member_to,
            cancel.flag(),
            |local, text| {
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
                    gather.push(Item {
                        key: key_of(format, order.column, text),
                        row,
                        descending: order.descending,
                    });
                }
                if since_report >= REPORT_EVERY {
                    since_report = 0;
                    let _ = tx.send(Update::Progress { scanned });
                }
            },
        );
        if let Err(e) = walked {
            outcome = Outcome::Failed(e.to_string());
            break;
        }
        if cancel.cancelled() {
            outcome = Outcome::Cancelled;
            break;
        }
    }
    let rows = if matches!(outcome, Outcome::Complete) {
        gather.into_order()
    } else {
        Vec::new()
    };
    let _ = tx.send(Update::Finished(outcome, rows));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::encoding::Charset;
    use crate::file::LogFile;
    use crate::format::by_id;
    use crate::indexer::{build_index, IndexOptions};
    use std::path::Path;
    use std::sync::Arc;

    fn excerpt(path: &Path) -> Excerpt {
        let file = LogFile::open(path).expect("open");
        let end = file.len().expect("len");
        let index =
            build_index(&file, Charset::UTF_8, 0, end, &IndexOptions::default()).expect("index");
        Excerpt {
            file: Arc::new(file),
            charset: Charset::UTF_8,
            index,
            first_row: 0,
        }
    }

    fn finish(running: &Running) -> (Outcome, Vec<u64>) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        loop {
            for update in running.drain() {
                if let Update::Finished(outcome, rows) = update {
                    return (outcome, rows);
                }
            }
            assert!(
                std::time::Instant::now() < deadline,
                "the sort did not finish"
            );
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }

    fn serilog() -> &'static Format {
        by_id("serilog-file").expect("the Serilog file format is in the catalogue")
    }

    const LOG: &str = "2026-08-17 10:00:00.000 +01:00 [INF] started\r\n\
2026-08-17 10:00:01.000 +01:00 [ERR] failed\r\n\
   at Some.Frame()\r\n\
2026-08-17 10:00:02.000 +01:00 [WRN] slow\r\n\
2026-08-17 10:00:03.000 +01:00 [DBG] detail\r\n\
2026-08-17 10:00:04.000 +01:00 [ERR] failed again\r\n";

    fn write(name: &str) -> std::path::PathBuf {
        let path =
            std::env::temp_dir().join(format!("tailhawk-sort-{name}-{}.log", std::process::id()));
        std::fs::write(&path, LOG).expect("write");
        path
    }

    #[test]
    fn keys_read_through_the_format() {
        let f = serilog();
        let level = f.columns.iter().position(|c| *c == LEVEL).expect("level");
        let ts = f.columns.iter().position(|c| *c == TS).expect("ts");
        let err = key_of(f, level, "2026-08-17 10:00:01.000 +01:00 [ERR] failed");
        let inf = key_of(f, level, "2026-08-17 10:00:00.000 +01:00 [INF] started");
        assert!(matches!(err, Key::Int(_)));
        assert_eq!(
            compare(&inf, &err, false),
            Ordering::Less,
            "INF sorts below ERR"
        );
        assert!(matches!(
            key_of(f, ts, "2026-08-17 10:00:00.000 +01:00 [INF] started"),
            Key::Int(_)
        ));
        assert_eq!(key_of(f, level, "   at Some.Frame()"), Key::Missing);
        assert_eq!(
            compare(&Key::Missing, &inf, true),
            Ordering::Greater,
            "missing is last either way"
        );
        assert_eq!(
            compare(&Key::Number(1.0), &Key::Text("a".into()), true),
            Ordering::Greater,
            "descending puts text first"
        );
    }

    #[test]
    fn sorts_by_level_both_ways_with_continuations_last() {
        let path = write("level");
        let f = serilog();
        let level = f.columns.iter().position(|c| *c == LEVEL).expect("level");
        let asc = start(
            vec![excerpt(&path)],
            Keep::All,
            0,
            6,
            f,
            Order {
                column: level,
                descending: false,
                top: None,
            },
        )
        .expect("start");
        let (outcome, rows) = finish(&asc);
        assert_eq!(outcome, Outcome::Complete);
        // DBG(4) INF(0) WRN(3) ERR(1) ERR(5), then the frame line.
        assert_eq!(rows, [4, 0, 3, 1, 5, 2]);
        let desc = start(
            vec![excerpt(&path)],
            Keep::All,
            0,
            6,
            f,
            Order {
                column: level,
                descending: true,
                top: None,
            },
        )
        .expect("start");
        let (_, rows) = finish(&desc);
        assert_eq!(
            rows,
            [1, 5, 3, 0, 4, 2],
            "ties keep file order; missing still last"
        );
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn top_n_over_a_filter_is_the_head_of_the_sort() {
        let path = write("top");
        let f = serilog();
        let level = f.columns.iter().position(|c| *c == LEVEL).expect("level");
        let running = start(
            vec![excerpt(&path)],
            Keep::Rows(vec![0, 1, 3, 4, 5]),
            0,
            6,
            f,
            Order {
                column: level,
                descending: true,
                top: Some(2),
            },
        )
        .expect("start");
        let (outcome, rows) = finish(&running);
        assert_eq!(outcome, Outcome::Complete);
        assert_eq!(rows, [1, 5]);
        let _ = std::fs::remove_file(path);
    }
}
