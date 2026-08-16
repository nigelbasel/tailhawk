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
use std::sync::Arc;

use crate::cell::ColumnAnchors;
use crate::encoding::Charset;
use crate::file::{FileSource, LogFile};
use crate::follow::Follow;
use crate::index::{Extent, LineIndex};
use crate::indexer::{build_index, IndexOptions};
use crate::pattern::RollingSet;
use crate::rotation::{Rotation, Watch};
use crate::rows::{RowSource, Rows};
use crate::scanner::Scanner;
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

/// How often the directory is re-listed looking for a new member.
///
/// **⚠ §5.5b asks for something better and this is not it.** "A new member is detected by directory
/// watch — this is the one case where `FILE_NOTIFY_CHANGE_FILE_NAME` fires reliably (§5.4), so it is
/// genuinely event-driven rather than polled." A `ReadDirectoryChangesW` watch needs a thread and an
/// overlapped completion, which is shell machinery this portable module cannot own; a re-listing
/// stands in until it exists, and the gap is in `HANDOFF.md`.
///
/// A second is chosen against what the delay actually costs: a roll that goes unnoticed for up to a
/// second is a second of tail that arrives late, not lost — the drain reads the old member to EOF
/// whenever it happens, so no bytes go missing either way. Listing a directory of ten files every
/// tick, by contrast, is a syscall per tick for a change that happens daily.
pub const LIST_INTERVAL: std::time::Duration = std::time::Duration::from_secs(1);

/// One file of a set, with its own index, decoder state and viewport window.
pub struct Member {
    name: String,
    path: PathBuf,
    /// **Shared, because the scan runs on another thread.** `SPEC.md` §5.2's positional reads are
    /// what make that sound — every read carries its own offset and nothing moves a shared file
    /// pointer — and reopening by path instead would risk landing on a different file across a
    /// rotation. See [`crate::scanner`].
    file: Arc<LogFile>,
    charset: Charset,
    index: LineIndex,
    rows: Rows,
    /// The set-wide row number of this member's line 0.
    first_row: u64,
    /// First byte after any BOM. §5.6 consumes it; the index starts after it so offsets stay exact.
    start: u64,
    /// How far this member has been indexed. Only the newest one ever moves.
    scanned_to: u64,
    /// §5.6: the head and tail samples reached different conclusions about the encoding. The tail
    /// won, because it is what is live, and the file is flagged rather than silently decided.
    disagreed: bool,
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

    /// §5.6: the head and tail samples disagreed about this member's encoding, so it is flagged.
    pub fn disagreed(&self) -> bool {
        self.disagreed
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
            file: Arc::new(file),
            charset,
            index,
            rows: Rows::new(charset),
            first_row: 0,
            start,
            scanned_to: end,
            disagreed: detection.disagreed,
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
    /// The growth scan for the **live** member, on its own thread. Every older member is finished
    /// and never scanned again, so there is exactly one worker however many members there are.
    scanner: Option<Scanner>,
    /// The live member's path and identity, so §5.5's in-place rotations are noticed.
    watch: Watch,
    /// When the directory was last re-listed. See [`LIST_INTERVAL`].
    listed: Option<std::time::Instant>,
}

/// What one [`LogSet::poll`] found. Several of these can be true in one tick, which is why it is a
/// record rather than an enum — a roll that also grew the new member is the ordinary case.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Polled {
    /// Lines appended to the row space, across every member touched.
    pub lines_added: u64,
    /// The name of the member now being written to, if the set rolled onto a new one.
    pub rolled_to: Option<String>,
    /// **The live member's contents were replaced under us** — §5.5's copy-truncate. Its index was
    /// rebuilt from zero, so anything addressing its old rows (a selection, a bookmark) is stale.
    pub reset: bool,
    /// Members that left the set — §5.5b's retention deletions. Their rows are gone.
    pub retired: Vec<String>,
}

impl Polled {
    /// Nothing happened, so a caller can skip the work a change implies.
    pub fn is_quiet(&self) -> bool {
        *self == Self::default()
    }
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

    /// Opens exactly one file, **without looking at its siblings**.
    ///
    /// For a source that is a file by accident rather than by nature — §4.2's stdin spill is the
    /// case that forced this. Spill names share a shape (`tailhawk-spill-<pid>-<n>.log`), and
    /// `pattern.rs` matches on the literal skeleton with the numbers taken out, so two *concurrent*
    /// Tailhawk instances' spills look exactly like two generations of one rolling log. Inferring a
    /// set there would splice another process's piped stream into this one's scrollback.
    ///
    /// The general rule this is an instance of: **inference is for files a user pointed at**, and a
    /// spill is a file this program made.
    pub fn open_single(path: &Path) -> Result<Self> {
        let anchor = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .ok_or_else(|| Error(format!("{} names no file", path.display())))?;
        let dir = path.parent().unwrap_or(Path::new(".")).to_path_buf();
        let set = RollingSet::infer(&anchor, &[]);
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

        let live = members.last().expect("checked non-empty");
        let watch = Watch::new(
            &live.path,
            live.file
                .identity()
                .map_err(|e| Error(format!("{}: {e}", live.name)))?,
        );

        let mut this = Self {
            dir,
            anchor,
            set,
            members,
            omitted: eager,
            total_rows: 0,
            last_error,
            scanner: None,
            watch,
            listed: None,
        };
        this.reseat_scanner();
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

    /// Bytes indexed across the whole set.
    ///
    /// Across, not just the live member: reporting the live member's alone made the byte count in
    /// the title *fall* at every roll, which reads as data lost when nothing was.
    pub fn bytes(&self) -> u64 {
        self.members.iter().map(|m| m.scanned_to - m.start).sum()
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

    /// The set as a worker can hold it — one [`Excerpt`] per member, oldest first.
    ///
    /// **This is a snapshot and it is meant to be.** The handles are shared, so the worker reads the
    /// same files rather than reopening names that may have rolled; the indexes are *cloned*, so the
    /// row space the worker reports against cannot move while it works. A search therefore answers
    /// for the log as it was when it started, which is the only answer a followed file can give
    /// without either a lock on the per-frame path or line numbers that shift under the result list.
    ///
    /// See [`crate::find`], which is the only caller and argues the rest of it.
    #[cfg(windows)]
    pub fn snapshot(&self) -> Vec<crate::find::Excerpt> {
        self.members
            .iter()
            .map(|m| crate::find::Excerpt {
                file: Arc::clone(&m.file),
                charset: m.charset,
                index: m.index.clone(),
                first_row: m.first_row,
            })
            .collect()
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
                &*member.file,
                &member.index,
                lo - member.first_row,
                (hi - lo) as usize,
                anchored,
            )?;
        }
        Ok(())
    }

    /// Advances the set: growth on the live member, and every way it can stop being live.
    ///
    /// **This does no scanning.** [`crate::scanner`] runs it on a worker; what happens here is
    /// folding the worker's findings into the index and deciding what the file has become. There is
    /// no per-tick time budget to set, which is the point — the budget that satisfied §11.3's frame
    /// rule starved the throughput criterion and the one that met the throughput criterion broke the
    /// frame rule. The only blocking wait left is the drain before a switch, which §5.5 requires.
    ///
    /// ## The order is the design
    ///
    /// 1. **Rotation in place** — §5.5's copy-truncate and rename-and-recreate, asked of the live
    ///    member's own handle. Tested first because applying a *new* file's length to the *old*
    ///    file's scan position reads bytes that are not the ones the index describes.
    /// 2. **A new member in the directory** — §5.5b's roll-to-new-name, which no property of the
    ///    live file reveals: it "never changes identity and never shrinks".
    /// 3. **Growth**, of whichever member is live once 1 and 2 have settled.
    ///
    /// A switch in either 1 or 2 drains the outgoing member to EOF first. §5.5: "This is where naive
    /// tools lose the last KB", and unlike the single-file case those drained lines are now *kept* —
    /// the old member stays in the scrollback, which is what made the drain worth writing.
    pub fn poll(&mut self) -> Polled {
        let mut polled = Polled::default();

        let live_len = self.members.last().and_then(|m| m.file.len().ok());
        if let Some(len) = live_len {
            let scanned_to = self.members.last().map_or(0, |m| m.scanned_to);
            match self.watch.check(len, scanned_to) {
                Rotation::Stable => {}
                // `tail -F`: the path is gone but the handle is not. Keep reading what we hold.
                Rotation::Missing => {}
                Rotation::Truncated => {
                    polled.reset = self.reseat_live();
                }
                Rotation::Replaced => {
                    // log4net's roll. The file we hold has been renamed out from under the path and
                    // a new one has taken its place — so the outgoing member is not leaving the set,
                    // it is becoming its own predecessor.
                    polled.lines_added += self.drain_live();
                    if let Some(name) = self.adopt_path_as_new_member() {
                        polled.rolled_to = Some(name);
                    }
                }
            }
        }

        if polled.rolled_to.is_none() {
            self.reconcile_directory(&mut polled);
        }

        polled.lines_added += self.grow_live();
        self.renumber();
        polled
    }

    /// Reads the live member to EOF. Returns the lines that arrived.
    ///
    /// **Blocks until the worker is caught up**, which is the one place that is correct: §5.5 wants
    /// the old handle read to EOF *before* a switch, and a drain that gave up would lose exactly the
    /// last KB the requirement exists to keep.
    fn drain_live(&mut self) -> u64 {
        let Self {
            members, scanner, ..
        } = self;
        let (Some(live), Some(scanner)) = (members.last_mut(), scanner.as_mut()) else {
            return 0;
        };
        let Ok(len) = live.file.len() else {
            return 0;
        };
        let collected = scanner.drain(&mut live.index, len);
        live.scanned_to = collected.applied_to.max(live.scanned_to);
        collected.lines
    }

    /// Folds in whatever the worker has scanned since the last tick.
    ///
    /// **`budget_ms` is gone and that is the point.** The scan no longer runs here — `scanner.rs`
    /// owns it — so there is no per-tick time budget to set, and §11.3's "the UI never blocks on
    /// indexing" is met by construction rather than by choosing a number that satisfied neither the
    /// throughput criterion nor the frame budget.
    fn grow_live(&mut self) -> u64 {
        let Self {
            members, scanner, ..
        } = self;
        let (Some(live), Some(scanner)) = (members.last_mut(), scanner.as_mut()) else {
            return 0;
        };
        let Ok(len) = live.file.len() else {
            return 0;
        };
        // Woken first, then collected: the worker starts on the new length while this tick folds in
        // whatever it produced from the last one. Collecting first would leave the worker idle for
        // the duration of the fold, which is the wrong way round when the fold is the cheap half.
        scanner.look(len);
        let collected = scanner.collect(&mut live.index);
        live.scanned_to = collected.applied_to.max(live.scanned_to);
        collected.lines
    }

    /// [`poll`](Self::poll), then **wait for the worker to catch up**.
    ///
    /// `poll` is deliberately asynchronous: it wakes the worker and folds in what is ready, so lines
    /// written in the last few milliseconds land on the next tick — invisible at 100 ms, and the
    /// whole reason the scan is off this thread.
    ///
    /// This is for the callers that cannot accept that. Tests are the main one: "append a line, then
    /// assert the row count" is otherwise a race, and a test that sleeps to fix a race is a test that
    /// will flake on a loaded machine.
    pub fn settle(&mut self) -> Polled {
        let mut polled = self.poll();
        polled.lines_added += self.drain_live();
        self.renumber();
        polled
    }

    /// Points a fresh worker at whichever member is now live.
    fn reseat_scanner(&mut self) {
        self.scanner = None;
        let Some(live) = self.members.last() else {
            return;
        };
        let follow = Follow::after_build(live.charset, &live.index, live.scanned_to);
        self.scanner = Scanner::start(Arc::clone(&live.file), follow).ok();
    }

    /// Rebuilds the live member from a path whose contents were replaced in place.
    ///
    /// §5.5's copy-truncate: the identity is unchanged, so this is the *same* member with different
    /// bytes — it keeps its position in the set rather than becoming a new one. Everything derived
    /// from the old contents goes: the index, the scan position, the row window.
    fn reseat_live(&mut self) -> bool {
        let Some(live) = self.members.last_mut() else {
            return false;
        };
        let Ok(fresh) = Member::open(&live.path) else {
            return false;
        };
        let Ok(identity) = fresh.file.identity() else {
            return false;
        };
        self.watch = Watch::new(&fresh.path, identity);
        *live = fresh;
        // **The old worker is dropped here, holding the old file's `Arc`.** It cannot leak a delta
        // into the new index: `reseat_scanner` replaces the whole `Scanner`, and its receiver goes
        // with it, so anything the old worker had queued is discarded along with the bytes it
        // described.
        self.reseat_scanner();
        true
    }

    /// Attaches whatever now sits at the watched path as the new live member.
    ///
    /// The member we were holding stays where it is in the set — its handle still reads, because
    /// §5.1's share mode includes `DELETE` and a rename does not invalidate an open handle. Only its
    /// *name* is now wrong, and [`rename_rolled_member`](Self::rename_rolled_member) repairs that by
    /// identity rather than by guessing which sibling it became.
    fn adopt_path_as_new_member(&mut self) -> Option<String> {
        let path = self.watch.path().to_path_buf();
        let fresh = Member::open(&path).ok()?;
        let identity = fresh.file.identity().ok()?;
        let name = fresh.name.clone();
        self.rename_rolled_member();
        self.watch = Watch::new(&path, identity);
        self.members.push(fresh);
        self.reseat_scanner();
        Some(name)
    }

    /// Finds the new name of the member that was just renamed out from under the watched path.
    ///
    /// **By identity, which is the only thing a rename preserves.** §5.5: "detection is keyed on file
    /// identity, never on the path string." The candidates are the names in the directory that the
    /// pattern accepts and that no member already holds — for a log4net set that is one file, so this
    /// costs one open. A member whose new name cannot be established keeps its old one rather than
    /// being given a guessed one; a wrong name in the gutter is worse than a stale one, because a
    /// stale one is at least the name the user opened.
    fn rename_rolled_member(&mut self) {
        let Some(rolled) = self.members.last() else {
            return;
        };
        let Ok(identity) = rolled.file.identity() else {
            return;
        };
        let held: Vec<&str> = self.members.iter().map(|m| m.name.as_str()).collect();
        let listing = siblings(&self.dir);
        let set = RollingSet::infer(&self.anchor, &listing);
        self.set = set.clone();
        let candidates: Vec<String> = set
            .members()
            .iter()
            .filter(|n| !held.contains(&n.as_str()))
            .cloned()
            .collect();
        for name in candidates {
            let path = self.dir.join(&name);
            let Ok(candidate) = LogFile::open(&path) else {
                continue;
            };
            if candidate.identity().ok() == Some(identity) {
                let rolled = self.members.last_mut().expect("checked above");
                rolled.name = name;
                rolled.path = path;
                return;
            }
        }
    }

    /// Re-lists the directory and reconciles the set against it. §5.5b's roll-to-new-name and its
    /// retention deletions are both here, because both are facts about the *directory* and neither
    /// is visible from any handle we hold.
    fn reconcile_directory(&mut self, polled: &mut Polled) {
        let now = std::time::Instant::now();
        if self.listed.is_some_and(|at| now - at < LIST_INTERVAL) {
            return;
        }
        self.listed = Some(now);

        let listing = siblings(&self.dir);
        let set = RollingSet::infer(&self.anchor, &listing);
        // **Kept, not just consulted.** A log4net set that has rolled once was a `Single` when it
        // was opened, and `pattern()` reporting that for ever would describe a set that no longer
        // exists.
        self.set = set.clone();
        let held: Vec<String> = self.members.iter().map(|m| m.name.clone()).collect();
        let live_name = held.last().cloned().unwrap_or_default();

        // §5.5b: "A member disappearing from the middle or tail of the set removes it from the
        // scrollback with a marker; it is never an error." Held handles keep reading a deleted file,
        // so this is a decision to follow the spec rather than a limitation — the alternative is a
        // scrollback that disagrees with the directory it claims to be showing.
        //
        // The live member is exempt: a path that vanished is §5.5's `Missing`, which wants `tail -F`
        // semantics and not the loss of the tail we are watching.
        let mut retired = Vec::new();
        self.members.retain(|m| {
            let gone = m.name != live_name && !listing.contains(&m.name);
            if gone {
                retired.push(m.name.clone());
            }
            !gone
        });
        polled.retired = retired;

        // **Newer means later in the *set's* order, not later in the alphabet.** Comparing names
        // directly gets log4net backwards — `app.log.1` sorts after `app.log` by any string rule and
        // is older than it — which is the reversal §5.5b calls the trap, reached from a third
        // direction. `pattern.rs` already decided the direction; asking it is the only safe move.
        let arrivals: Vec<String> = match set.members().iter().position(|n| *n == live_name) {
            Some(at) => set.members()[at + 1..]
                .iter()
                .filter(|n| !held.contains(n))
                .cloned()
                .collect(),
            // The live member is not in the re-inferred set at all — its path was renamed away, or
            // the directory changed shape under us. Attaching anything on that basis is a guess.
            None => Vec::new(),
        };
        if arrivals.is_empty() {
            return;
        }

        // §5.5b: "Drain-then-switch, exactly as for rename rotation: the old member is read to EOF
        // before the new one is attached, so the last lines before a roll are never lost."
        polled.lines_added += self.drain_live();

        let mut attached = None;
        for name in arrivals {
            match Member::open(&self.dir.join(&name)) {
                Ok(member) => {
                    attached = Some(member.name.clone());
                    self.members.push(member);
                }
                Err(e) => self.last_error = Some(e.0),
            }
        }
        let Some(name) = attached else {
            return;
        };
        let live = self.members.last().expect("just pushed");
        if let Ok(identity) = live.file.identity() {
            self.watch = Watch::new(&live.path, identity);
        }
        self.reseat_scanner();
        polled.rolled_to = Some(name);
    }

    /// Forces the next [`poll`](Self::poll) to re-list the directory rather than wait out
    /// [`LIST_INTERVAL`]. What a "refresh" command drives, and what makes a roll testable without
    /// sleeping for a second.
    pub fn rescan(&mut self) {
        self.listed = None;
    }

    /// One line the UI can show. §5.5b requires the inference be confirmable, not assumed.
    ///
    /// **Built from the members actually held, not from the inference made at open.** A set that has
    /// rolled since is a different set; a title that still names the file it opened with is telling
    /// the user something that has stopped being true, which is worse than telling them nothing —
    /// they would be confirming an order against a file list that no longer matches. Caught by
    /// screenshot rather than by test: nothing here could see that the string was stale.
    ///
    /// It names files rather than directions for the same reason [`RollingSet::describe`] does. A
    /// user can check "oldest is `app.log.2`" against the folder in front of them; nobody checks
    /// "descending".
    pub fn describe(&self) -> String {
        let oldest = self.members[0].name();
        let newest = self.newest().name();
        let mut out = if self.members.len() == 1 {
            format!("1 file — {oldest}")
        } else {
            format!(
                "{} files — oldest is {oldest}, newest is {newest}",
                self.members.len()
            )
        };
        if self.omitted > 0 {
            out.push_str(&format!("; {} older not indexed", self.omitted));
        }
        out
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
        assert_eq!(set.describe(), "1 file — app.log");
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

    fn append(dir: &Path, name: &str, lines: &[&str]) {
        let mut file = std::fs::OpenOptions::new()
            .append(true)
            .open(dir.join(name))
            .expect("append");
        for line in lines {
            writeln!(file, "{line}").expect("write");
        }
    }

    /// Read the whole set, whatever it now is. Fetches first so `row_text` has something to serve.
    fn read_all(set: &mut LogSet) -> Vec<String> {
        let total = set.total_rows();
        set.fetch(0, total as usize, false).expect("fetch");
        texts(set)
    }

    #[test]
    fn the_live_member_grows_and_the_older_ones_do_not_move() {
        let dir = scratch("growth");
        write(&dir, "log_001.txt", &["a1"]);
        let anchor = write(&dir, "log_002.txt", &["b1"]);
        let mut set = LogSet::open(&anchor).expect("open");
        assert_eq!(set.total_rows(), 2);

        append(&dir, "log_002.txt", &["b2", "b3"]);
        let polled = set.settle();
        assert_eq!(polled.lines_added, 2);
        assert!(polled.rolled_to.is_none());
        assert_eq!(read_all(&mut set), ["a1", "b1", "b2", "b3"]);
        assert_eq!(set.members()[0].first_row(), 0);
    }

    /// §5.5b's roll-to-new-name — Serilog's and NLog's *default*, and the case §5.5 explicitly could
    /// not detect because the live file "never changes identity and never shrinks".
    ///
    /// **The lines written to the old member after the new one appeared are the whole test.** Follow
    /// re-seats onto the new member, so anything not drained first is never scanned again — not
    /// delayed, lost. Deleting the `drain_live` call above leaves this the only failing test.
    #[test]
    fn a_roll_to_a_new_name_drains_the_old_member_before_switching() {
        let dir = scratch("roll");
        write(&dir, "log_001.txt", &["a1"]);
        let anchor = write(&dir, "log_002.txt", &["b1"]);
        let mut set = LogSet::open(&anchor).expect("open");

        // The writer's last words before it rolled, then the file it rolled onto.
        append(&dir, "log_002.txt", &["b2 last words"]);
        write(&dir, "log_003.txt", &["c1"]);

        let polled = set.settle();
        assert_eq!(polled.rolled_to.as_deref(), Some("log_003.txt"));
        assert_eq!(set.newest().name(), "log_003.txt");
        assert_eq!(read_all(&mut set), ["a1", "b1", "b2 last words", "c1"]);

        // And it keeps following the file it rolled onto.
        append(&dir, "log_003.txt", &["c2"]);
        set.settle();
        assert_eq!(
            read_all(&mut set),
            ["a1", "b1", "b2 last words", "c1", "c2"]
        );
    }

    /// log4net's rename-and-recreate, which `rotation.rs` could report and a single-file view could
    /// only survive by throwing the old file away. Here the renamed file **stays in the scrollback**,
    /// which is what §5.5's "never lose the last KB" was always for.
    #[test]
    fn a_renamed_live_file_becomes_its_own_predecessor() {
        let dir = scratch("rename");
        let anchor = write(&dir, "app.log", &["gen1 a"]);
        let mut set = LogSet::open(&anchor).expect("open");
        assert_eq!(set.members().len(), 1);

        std::fs::rename(dir.join("app.log"), dir.join("app.log.1")).expect("rename");
        append(&dir, "app.log.1", &["gen1 last words"]);
        write(&dir, "app.log", &["gen2 a"]);

        let polled = set.settle();
        assert_eq!(polled.rolled_to.as_deref(), Some("app.log"));
        assert_eq!(set.members().len(), 2);
        assert_eq!(read_all(&mut set), ["gen1 a", "gen1 last words", "gen2 a"]);
    }

    /// §5.5: "detection is keyed on file identity, never on the path string." After a rename the
    /// member we hold is reachable only through its handle, and the name in the gutter has to be
    /// repaired from the directory — by identity, because the name is exactly what changed.
    #[test]
    fn a_member_renamed_out_from_under_us_learns_its_new_name() {
        let dir = scratch("renamed-gutter");
        let anchor = write(&dir, "app.log", &["gen1"]);
        let mut set = LogSet::open(&anchor).expect("open");

        std::fs::rename(dir.join("app.log"), dir.join("app.log.1")).expect("rename");
        write(&dir, "app.log", &["gen2"]);
        set.settle();

        assert_eq!(set.members()[0].name(), "app.log.1");
        assert_eq!(set.members()[0].path(), dir.join("app.log.1"));
        assert_eq!(set.members()[1].name(), "app.log");
    }

    /// §5.5's copy-truncate on the live member: same identity, different bytes. It stays the same
    /// member — it does not become a new generation — and `reset` tells the caller its old rows are
    /// gone, which is the fact a selection or a bookmark needs.
    #[test]
    fn a_truncated_live_member_is_rebuilt_in_place() {
        let dir = scratch("truncate");
        write(&dir, "log_001.txt", &["a1"]);
        let anchor = write(&dir, "log_002.txt", &["b1", "b2", "b3", "b4"]);
        let mut set = LogSet::open(&anchor).expect("open");
        assert_eq!(set.total_rows(), 5);

        std::fs::write(dir.join("log_002.txt"), b"fresh\n").expect("truncate");
        let polled = set.settle();
        assert!(polled.reset);
        assert!(polled.rolled_to.is_none());
        assert_eq!(set.members().len(), 2);
        assert_eq!(read_all(&mut set), ["a1", "fresh"]);
    }

    /// §5.5b: "Retention deletions are tolerated. A member disappearing from the middle or tail of
    /// the set removes it from the scrollback with a marker; it is never an error."
    #[test]
    fn a_member_deleted_by_retention_leaves_the_scrollback() {
        let dir = scratch("retired");
        write(&dir, "log_001.txt", &["a1"]);
        write(&dir, "log_002.txt", &["b1"]);
        let anchor = write(&dir, "log_003.txt", &["c1"]);
        let mut set = LogSet::open(&anchor).expect("open");
        assert_eq!(set.total_rows(), 3);

        std::fs::remove_file(dir.join("log_001.txt")).expect("remove");
        set.rescan();
        let polled = set.settle();
        assert_eq!(polled.retired, ["log_001.txt"]);
        assert_eq!(set.total_rows(), 2);
        assert_eq!(read_all(&mut set), ["b1", "c1"]);
    }

    /// The live member is exempt from retirement. Its path vanishing is §5.5's `Missing`, which wants
    /// `tail -F` semantics — keep the handle, keep reading — and not the loss of the tail on screen.
    #[test]
    fn the_live_member_vanishing_from_the_directory_is_not_a_retirement() {
        let dir = scratch("missing-live");
        write(&dir, "log_001.txt", &["a1"]);
        let anchor = write(&dir, "log_002.txt", &["b1"]);
        let mut set = LogSet::open(&anchor).expect("open");

        std::fs::remove_file(dir.join("log_002.txt")).expect("remove");
        set.rescan();
        let polled = set.settle();
        assert!(polled.retired.is_empty());
        assert_eq!(set.total_rows(), 2);
        assert_eq!(read_all(&mut set), ["a1", "b1"]);
    }

    /// **The defect a screenshot found and no test here could have.** `describe` was built from the
    /// inference made at open, so a window showing three files went on saying "2 files … newest is
    /// `log_002.txt`" — a confirmation prompt inviting a check against a list that had moved on.
    #[test]
    fn the_description_follows_the_set_across_a_roll() {
        let dir = scratch("describe-roll");
        write(&dir, "log_001.txt", &["a"]);
        let anchor = write(&dir, "log_002.txt", &["b"]);
        let mut set = LogSet::open(&anchor).expect("open");
        assert_eq!(
            set.describe(),
            "2 files — oldest is log_001.txt, newest is log_002.txt"
        );

        write(&dir, "log_003.txt", &["c"]);
        set.settle();
        assert_eq!(
            set.describe(),
            "3 files — oldest is log_001.txt, newest is log_003.txt"
        );
    }

    /// The same for log4net, where the shape itself changes: a set that was one file when it was
    /// opened becomes two the moment the writer renames it.
    #[test]
    fn a_set_that_was_one_file_says_two_after_the_rename() {
        let dir = scratch("describe-rename");
        let anchor = write(&dir, "app.log", &["gen1"]);
        let mut set = LogSet::open(&anchor).expect("open");
        assert_eq!(set.describe(), "1 file — app.log");

        std::fs::rename(dir.join("app.log"), dir.join("app.log.1")).expect("rename");
        write(&dir, "app.log", &["gen2"]);
        set.settle();
        assert_eq!(
            set.describe(),
            "2 files — oldest is app.log.1, newest is app.log"
        );
    }

    /// A quiet tick has to be recognisable, or the shell repaints and re-fetches every 100 ms
    /// against a file nobody is writing to.
    #[test]
    fn a_tick_with_nothing_happening_says_so() {
        let dir = scratch("quiet");
        let anchor = write(&dir, "app.log", &["a"]);
        let mut set = LogSet::open(&anchor).expect("open");
        set.settle();
        assert!(set.settle().is_quiet());
    }
}
