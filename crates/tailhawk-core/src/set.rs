//! A rolling set as one logical source — `SPEC.md` §5.5b.
//!
//! [`crate::pattern`] decides which files are members and what order they are in. This opens them,
//! gives them a single row space, and serves any row in it — so scrolling up past the head of the
//! current file continues into the previous one, which is the behaviour §5.5b exists for and which
//! §5.5b notes no incumbent has ("Hoo WinTail's folder monitoring opens new files as *separate
//! documents*, which is not the same thing").
//!
//! ## Each member keeps its own index, and the set holds a prefix sum
//!
//! **The alternative was one index spanning the set, and `index.rs` is why it is not.** A
//! [`LineIndex`] stores byte offsets, and [`offset_of_line`](crate::indexer::offset_of_line) reads
//! through one [`ChunkReader`]; a single index across several files would have to invent a synthetic
//! byte space and translate on every lookup, in the one code path that runs per frame.
//!
//! Per-member indices cost a binary search over a handful of `first_row` values instead, and they
//! make the common case free: §5.5b orders members **oldest first**, so the live member is last, and
//! its growth appends at the end of the row space without moving anything. Only a *new* member or a
//! retention deletion renumbers, and both are rare enough to walk the list.
//!
//! ## The eager window always contains the file the user opened
//!
//! §5.5b: "A set with 400 daily files is not fully indexed on open. The **newest N members (default
//! 10, configurable) or M bytes** are indexed eagerly."
//!
//! Applied naively that rule drops the anchor — a user who opens `log-20260101.txt` in a year-long
//! set would get the *December* files and not the one they double-clicked. So the window runs from
//! the anchor (or the N-th newest, whichever is older) to the newest, and [`EAGER_BYTES`] can shrink
//! it but never past the anchor. Opening an old file is an explicit request for that file.
//!
//! ## ⚠ What this does **not** do
//!
//! - **On-demand backfill.** §5.5b wants members past the eager bound "indexed on demand as the user
//!   scrolls into them, with a progress marker". The bound is enforced; the backfill is not. Older
//!   members are excluded from the row space and counted in [`LogSet::describe`], so the omission is
//!   visible rather than silent.
//! - **The separator row and the per-member gutter.** §5.5b wants each boundary marked and line
//!   numbers shown per member. [`LogSet::locate`] returns exactly what a painter needs for both;
//!   drawing them is a rendering feature and is not attempted here.
//! - **Archived members.** A `.gz` or `.zip` member stays out of the set until §4.3 exists to read
//!   it — see `pattern.rs`, where the name is rejected.

use std::path::{Path, PathBuf};

use crate::cell::ColumnAnchors;
use crate::encoding::Charset;
use crate::file::{FileSource, LogFile};
use crate::index::{Extent, LineIndex};
use crate::indexer::{build_index, IndexOptions};
use crate::pattern::RollingSet;
use crate::rows::{RowSource, Rows};
use crate::{Error, Result};

/// Members indexed when a set is opened. §5.5b's "default 10, configurable".
pub const EAGER_MEMBERS: usize = 10;

/// Bytes indexed when a set is opened, across all members. §5.5b's "or M bytes".
///
/// Indexing runs at gigabytes a second (`docs/HANDOFF.md` records 5.24 GB in 2.35 s), so this is not
/// a time bound — it is a *memory* bound. §11.2 budgets the index at roughly 8 bytes per anchor, and
/// half a gigabyte of log at 100-byte lines is about 80,000 anchors, which is nothing. The figure is
/// deliberately generous because the member count is the bound that will actually bind on a daily
/// set, and the byte bound is here for the pathological case of ten 10 GB files.
pub const EAGER_BYTES: u64 = 512 * 1024 * 1024;

/// One file of a set, with its own index, decoder state and viewport window.
pub struct Member {
    name: String,
    path: PathBuf,
    file: LogFile,
    charset: Charset,
    index: LineIndex,
    rows: Rows,
    /// The set-wide row number of this member's line 0.
    first_row: u64,
    /// First byte after any BOM. §5.6 consumes it; the index starts after it so offsets stay exact.
    start: u64,
    /// How far this member has been indexed. Only the newest one ever moves.
    scanned_to: u64,
}

impl Member {
    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn file(&self) -> &LogFile {
        &self.file
    }

    pub fn charset(&self) -> Charset {
        self.charset
    }

    pub fn index(&self) -> &LineIndex {
        &self.index
    }

    pub fn line_count(&self) -> u64 {
        self.index.line_count()
    }

    /// The set-wide row number of this member's first line.
    pub fn first_row(&self) -> u64 {
        self.first_row
    }

    pub fn scanned_to(&self) -> u64 {
        self.scanned_to
    }

    /// First byte after any byte-order mark. §5.6 consumes the BOM without rendering it, and the
    /// index starts here rather than at zero so every later offset stays byte-exact.
    pub fn start(&self) -> u64 {
        self.start
    }

    fn open(path: &Path) -> Result<Self> {
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        // One handle, opened once and kept: two opens could land on different files if the writer
        // rolls between them, and the index would then describe bytes the reads no longer return.
        let source = FileSource::open(path).map_err(|e| Error(format!("{name}: {e}")))?;
        let detection = *source.detection();
        let file = source.into_file();
        let charset = detection.charset;
        let start = detection.bom_len as u64;
        let end = file.len().map_err(|e| Error(format!("{name}: {e}")))?;
        let index = build_index(&file, charset, start, end, &IndexOptions::default())
            .map_err(|e| Error(format!("{name}: {e}")))?;
        Ok(Self {
            name,
            path: path.to_path_buf(),
            file,
            charset,
            index,
            rows: Rows::new(charset),
            first_row: 0,
            start,
            scanned_to: end,
        })
    }
}

/// Where a set-wide row lives.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct Located {
    /// Index into [`LogSet::members`], which is oldest-first.
    pub member: usize,
    /// The row's number **within that member**, which is what §5.5b wants in the gutter.
    pub line: u64,
    /// True when this row is the member's line 0 — where §5.5b wants a separator row drawn.
    pub starts_member: bool,
}

/// A set of files presented as one continuous log.
pub struct LogSet {
    dir: PathBuf,
    anchor: String,
    set: RollingSet,
    /// Oldest first, matching [`RollingSet::members`].
    members: Vec<Member>,
    /// Members the eager bound left out. §5.5b wants these backfilled on demand; see the module note.
    omitted: usize,
    total_rows: u64,
    last_error: Option<String>,
}

impl LogSet {
    /// Opens the set containing `path`, indexing as much of it as the eager bound allows.
    ///
    /// A file with no siblings is a set of one, so a caller never has to ask which it has.
    pub fn open(path: &Path) -> Result<Self> {
        let anchor = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .ok_or_else(|| Error(format!("{} names no file", path.display())))?;
        let dir = path.parent().unwrap_or(Path::new(".")).to_path_buf();
        let set = RollingSet::infer(&anchor, &siblings(&dir));
        Self::open_set(dir, anchor, set)
    }

    fn open_set(dir: PathBuf, anchor: String, set: RollingSet) -> Result<Self> {
        let names = set.members().to_vec();
        let anchor_at = names.iter().position(|n| *n == anchor).unwrap_or(0);
        let eager = eager_window(&dir, &names, anchor_at);

        let mut members = Vec::new();
        let mut last_error = None;
        for name in &names[eager..] {
            match Member::open(&dir.join(name)) {
                Ok(member) => members.push(member),
                // §5.5b: "Retention deletions are tolerated. A member disappearing … is never an
                // error." The listing was taken a moment ago; a member can be gone by now, and the
                // rest of the set is still perfectly readable.
                Err(e) => last_error = Some(e.0),
            }
        }
        if members.is_empty() {
            return Err(Error(last_error.unwrap_or_else(|| {
                format!("no member of {} could be opened", dir.display())
            })));
        }

        let mut this = Self {
            dir,
            anchor,
            set,
            members,
            omitted: eager,
            total_rows: 0,
            last_error,
        };
        this.renumber();
        Ok(this)
    }

    /// Recomputes the row space after any member's line count changes.
    ///
    /// Cheap by construction: growth only ever touches the last member, so the sum it recomputes is
    /// over a handful of `u64`s. It is called unconditionally rather than incrementally because
    /// getting a prefix sum subtly wrong shows up as text from the wrong file, which is exactly the
    /// class of bug §5.5b's ordering warning is about.
    fn renumber(&mut self) {
        let mut row = 0;
        for member in &mut self.members {
            member.first_row = row;
            row += member.index.line_count();
        }
        self.total_rows = row;
    }

    pub fn members(&self) -> &[Member] {
        &self.members
    }

    /// The member being written to — the last one, because members are oldest-first.
    pub fn newest(&self) -> &Member {
        self.members.last().expect("a set has at least one member")
    }

    pub fn total_rows(&self) -> u64 {
        self.total_rows
    }

    /// How many older members the eager bound left out of the row space.
    pub fn omitted(&self) -> usize {
        self.omitted
    }

    pub fn last_error(&self) -> Option<&str> {
        self.last_error.as_deref()
    }

    pub fn anchor(&self) -> &str {
        &self.anchor
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    pub fn pattern(&self) -> &RollingSet {
        &self.set
    }

    /// Which member a set-wide row belongs to, and its line number within it.
    ///
    /// A binary search over `first_row`, using `partition_point` with `<=` so the answer is the last
    /// member starting at or before the row. **A member with no lines must not swallow a row**: an
    /// empty file that has just been rolled to has `first_row` equal to its predecessor's end, so a
    /// naive search lands on it and reports a line it does not have.
    pub fn locate(&self, row: u64) -> Option<Located> {
        if row >= self.total_rows {
            return None;
        }
        let after = self
            .members
            .partition_point(|m| m.first_row <= row)
            .saturating_sub(1);
        // Walk back over any empty members sharing this first_row. There are at most a handful and
        // usually none, and the alternative — excluding them from the search — would make the index
        // returned here disagree with `members()`.
        let mut at = after;
        while self.members[at].index.line_count() == 0 && at > 0 {
            at -= 1;
        }
        let member = &self.members[at];
        if row < member.first_row || row >= member.first_row + member.index.line_count() {
            return None;
        }
        Some(Located {
            member: at,
            line: row - member.first_row,
            starts_member: row == member.first_row,
        })
    }

    /// The horizontal extent of the whole set — the widest line in any member.
    ///
    /// [`Extent::merge`] is defined in file order for a single file's chunks; merging across members
    /// is the same operation over a coarser sequence, and the fields it joins — max line bytes, all
    /// ASCII — are order-independent maxima and conjunctions.
    pub fn extent(&self) -> Extent {
        self.members
            .iter()
            .fold(Extent::EMPTY, |acc, m| acc.merge(m.index.extent()))
    }

    /// The set's charset for display. The newest member's, because §5.6 re-runs detection on
    /// rotation and the live file is the one whose encoding the user is watching.
    pub fn charset(&self) -> Charset {
        self.newest().charset
    }

    /// Fills every member overlapping `[first, first + count)` with its slice of the viewport.
    ///
    /// **The window is split, not the read.** Each member fetches through its own [`Rows`], which
    /// keeps `Rows` a single-file type and keeps its cache key — which carries that member's line
    /// count — meaningful. A viewport wholly inside one member touches exactly one member, which is
    /// every frame except the handful that straddle a boundary.
    pub fn fetch(&mut self, first: u64, count: usize, anchored: bool) -> Result<()> {
        if count == 0 {
            return Ok(());
        }
        let end = first.saturating_add(count as u64).min(self.total_rows);
        for member in &mut self.members {
            let lines = member.index.line_count();
            let member_end = member.first_row + lines;
            let lo = first.max(member.first_row);
            let hi = end.min(member_end);
            if lo >= hi {
                // Not on screen. Left holding whatever it last served; a `Rows` window is a few
                // hundred kilobytes at most and re-fetching costs a warm sequential read.
                continue;
            }
            member.rows.fetch(
                &member.file,
                &member.index,
                lo - member.first_row,
                (hi - lo) as usize,
                anchored,
            )?;
        }
        Ok(())
    }

    /// One line the UI can show. §5.5b requires the inference be confirmable, not assumed.
    pub fn describe(&self) -> String {
        let base = self.set.describe();
        if self.omitted == 0 {
            base
        } else {
            format!("{base}; {} older not indexed", self.omitted)
        }
    }
}

impl RowSource for LogSet {
    fn row_text(&self, row: u64) -> Option<&str> {
        let at = self.locate(row)?;
        let member = &self.members[at.member];
        member.rows.row_text(at.line)
    }

    fn row_anchors(&self, row: u64) -> &ColumnAnchors {
        match self.locate(row) {
            Some(at) => self.members[at.member].rows.row_anchors(at.line),
            None => ColumnAnchors::none_ref(),
        }
    }
}

/// The names in a directory, as a sorted list of plain filenames.
///
/// Directories are skipped — §5.5b's NLog row puts members in a subfolder and that is not supported;
/// see the module note. A name that is not valid Unicode is skipped too: `pattern.rs` works on
/// `&str`, and a lossy conversion would produce a name that cannot be joined back onto the directory
/// to open it.
fn siblings(dir: &Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut names: Vec<String> = entries
        .flatten()
        .filter(|e| e.file_type().map(|t| t.is_file()).unwrap_or(false))
        .filter_map(|e| e.file_name().to_str().map(str::to_string))
        .collect();
    names.sort();
    names
}

/// The index of the oldest member to index eagerly. See the module note on the anchor.
fn eager_window(dir: &Path, names: &[String], anchor_at: usize) -> usize {
    let by_count = names.len().saturating_sub(EAGER_MEMBERS);
    let mut start = by_count.min(anchor_at);
    // Walk forward while the window is over budget, but never past the file the user opened.
    let mut total: u64 = names[start..]
        .iter()
        .map(|n| std::fs::metadata(dir.join(n)).map(|m| m.len()).unwrap_or(0))
        .sum();
    while start < anchor_at && total > EAGER_BYTES {
        total -= std::fs::metadata(dir.join(&names[start]))
            .map(|m| m.len())
            .unwrap_or(0);
        start += 1;
    }
    start
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join("tailhawk-set").join(name);
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("scratch dir");
        dir
    }

    fn write(dir: &Path, name: &str, lines: &[&str]) -> PathBuf {
        let path = dir.join(name);
        let mut file = std::fs::File::create(&path).expect("create");
        for line in lines {
            writeln!(file, "{line}").expect("write");
        }
        path
    }

    fn texts(set: &LogSet) -> Vec<String> {
        (0..set.total_rows())
            .map(|r| set.row_text(r).unwrap_or("<missing>").to_string())
            .collect()
    }

    /// The point of the whole module: three files, one row space, in writing order.
    #[test]
    fn a_dated_set_reads_as_one_log_oldest_first() {
        let dir = scratch("dated");
        write(&dir, "log-20260727.txt", &["mon a", "mon b"]);
        write(&dir, "log-20260728.txt", &["tue a"]);
        let anchor = write(&dir, "log-20260729.txt", &["wed a", "wed b", "wed c"]);

        let mut set = LogSet::open(&anchor).expect("open");
        assert_eq!(set.total_rows(), 6);
        set.fetch(0, 6, false).expect("fetch");
        assert_eq!(
            texts(&set),
            ["mon a", "mon b", "tue a", "wed a", "wed b", "wed c"]
        );
    }

    /// §5.5b's trap, end to end. `app.log.2` is the *oldest* text, and a set that read it as newest
    /// would still return six plausible rows — which is why this asserts the text and not the count.
    #[test]
    fn a_log4net_backup_set_reads_oldest_backup_first() {
        let dir = scratch("log4net");
        write(&dir, "app.log.2", &["oldest"]);
        write(&dir, "app.log.1", &["middle"]);
        let anchor = write(&dir, "app.log", &["newest"]);

        let mut set = LogSet::open(&anchor).expect("open");
        set.fetch(0, 3, false).expect("fetch");
        assert_eq!(texts(&set), ["oldest", "middle", "newest"]);
        assert_eq!(set.newest().name(), "app.log");
    }

    /// §5.5b: "line numbers are per-member with the member named in the gutter", and "a separator
    /// row marks each boundary". This is the data both need.
    #[test]
    fn a_row_knows_which_member_and_which_line_of_it_it_is() {
        let dir = scratch("locate");
        write(&dir, "log_001.txt", &["a", "b"]);
        let anchor = write(&dir, "log_002.txt", &["c"]);

        let set = LogSet::open(&anchor).expect("open");
        assert_eq!(
            set.locate(0),
            Some(Located {
                member: 0,
                line: 0,
                starts_member: true
            })
        );
        assert_eq!(
            set.locate(1),
            Some(Located {
                member: 0,
                line: 1,
                starts_member: false
            })
        );
        assert_eq!(
            set.locate(2),
            Some(Located {
                member: 1,
                line: 0,
                starts_member: true
            })
        );
        assert_eq!(set.locate(3), None);
    }

    /// A viewport that straddles a boundary is the case the whole design exists for, and it is the
    /// one a single-file viewer gets wrong by showing nothing above the current file's first line.
    #[test]
    fn a_viewport_spanning_a_boundary_is_filled_from_both_members() {
        let dir = scratch("straddle");
        write(&dir, "log_001.txt", &["a1", "a2", "a3"]);
        let anchor = write(&dir, "log_002.txt", &["b1", "b2", "b3"]);

        let mut set = LogSet::open(&anchor).expect("open");
        set.fetch(2, 2, false).expect("fetch");
        assert_eq!(set.row_text(2), Some("a3"));
        assert_eq!(set.row_text(3), Some("b1"));
    }

    /// An empty member is routine — a writer rolls to a new file and has not written to it yet. It
    /// must occupy no rows and must not swallow the row after it.
    #[test]
    fn an_empty_member_occupies_no_rows() {
        let dir = scratch("empty");
        write(&dir, "log_001.txt", &["a"]);
        write(&dir, "log_002.txt", &[]);
        let anchor = write(&dir, "log_003.txt", &["c"]);

        let mut set = LogSet::open(&anchor).expect("open");
        assert_eq!(set.total_rows(), 2);
        set.fetch(0, 2, false).expect("fetch");
        assert_eq!(texts(&set), ["a", "c"]);
        assert_eq!(set.locate(1).map(|l| l.member), Some(2));
    }

    /// Every member is decoded with its own charset. §5.6 re-runs detection on rotation because "the
    /// new file may differ", and in a set the old file differing is the same fact seen earlier.
    #[test]
    fn members_may_disagree_about_their_encoding() {
        let dir = scratch("charsets");
        let utf16: Vec<u8> = {
            let mut bytes = vec![0xFF, 0xFE];
            for unit in "wide\n".encode_utf16() {
                bytes.extend_from_slice(&unit.to_le_bytes());
            }
            bytes
        };
        std::fs::write(dir.join("log_001.txt"), &utf16).expect("write utf16");
        let anchor = write(&dir, "log_002.txt", &["plain"]);

        let mut set = LogSet::open(&anchor).expect("open");
        assert_eq!(set.total_rows(), 2);
        set.fetch(0, 2, false).expect("fetch");
        assert_eq!(texts(&set), ["wide", "plain"]);
    }

    /// §5.5b's bounded history, and the amendment the module argues for: the newest N, but never
    /// without the file the user actually opened.
    #[test]
    fn the_eager_window_keeps_the_newest_members_and_the_one_that_was_opened() {
        let dir = scratch("bounded");
        for day in 1..=20u32 {
            write(&dir, &format!("log-2026{day:04}.txt"), &["x"]);
        }
        let newest = dir.join("log-20260020.txt");
        let set = LogSet::open(&newest).expect("open");
        assert_eq!(set.members().len(), EAGER_MEMBERS);
        assert_eq!(set.omitted(), 10);
        assert_eq!(set.members()[0].name(), "log-20260011.txt");

        let old = dir.join("log-20260003.txt");
        let set = LogSet::open(&old).expect("open");
        assert_eq!(set.members()[0].name(), "log-20260003.txt");
        assert_eq!(set.newest().name(), "log-20260020.txt");
        assert_eq!(set.omitted(), 2);
    }

    /// §5.5b: a member disappearing "is never an error". The listing is a moment old by the time the
    /// members are opened, and retention runs on its own schedule.
    #[test]
    fn a_member_deleted_between_listing_and_opening_is_not_an_error() {
        let dir = scratch("retention");
        write(&dir, "log_001.txt", &["gone soon"]);
        let anchor = write(&dir, "log_002.txt", &["still here"]);
        let set = RollingSet::infer(
            "log_002.txt",
            &["log_001.txt".to_string(), "log_002.txt".to_string()],
        );
        std::fs::remove_file(dir.join("log_001.txt")).expect("remove");

        let mut opened =
            LogSet::open_set(dir, "log_002.txt".to_string(), set).expect("open despite the gap");
        assert_eq!(opened.total_rows(), 1);
        opened.fetch(0, 1, false).expect("fetch");
        assert_eq!(opened.row_text(0), Some("still here"));
        assert!(opened.last_error().is_some());
        let _ = anchor;
    }

    /// A file with nothing beside it must behave exactly like a set of one, so the shell has one
    /// path rather than two.
    #[test]
    fn a_lone_file_is_a_set_of_one() {
        let dir = scratch("lone");
        let anchor = write(&dir, "app.log", &["only"]);
        let mut set = LogSet::open(&anchor).expect("open");
        assert_eq!(set.members().len(), 1);
        assert_eq!(set.total_rows(), 1);
        set.fetch(0, 1, false).expect("fetch");
        assert_eq!(set.row_text(0), Some("only"));
        assert_eq!(set.describe(), "1 file — no rolling set found beside it");
    }

    /// The extent drives the horizontal scroll range, and a set whose widest line is in an *older*
    /// member would otherwise clip it.
    #[test]
    fn the_extent_covers_every_member_not_just_the_live_one() {
        let dir = scratch("extent");
        write(
            &dir,
            "log_001.txt",
            &["a very much longer line than the other"],
        );
        let anchor = write(&dir, "log_002.txt", &["short"]);
        let set = LogSet::open(&anchor).expect("open");
        // 37 characters and the terminator, which `Extent` counts — the point here is that the
        // widest line came from the member that is *not* live.
        assert_eq!(set.extent().max_line_bytes(), 38);
    }
}
