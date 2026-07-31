//! The file engine — `SPEC.md` §5.1 and §5.2.
//!
//! > **Product guarantee: Tailhawk never prevents your application from writing, rotating or
//! > deleting its own logs.**
//!
//! That guarantee is one `CreateFileW` call and two things this module must never do. It is
//! asserted nowhere and tested in `writer_safety` below, because it is the kind of promise that
//! stays true right up until somebody adds a convenience.
//!
//! The two prohibitions, both from §5.2:
//! - **No memory mapping of a followed file, ever.** A section handle blocks `DeleteFile`, so a
//!   log4net `RollingFileAppender` would fail to delete its oldest backup, swallow the error and
//!   silently stop rolling — filling the customer's disk with Tailhawk as the cause. "Tear the
//!   section down when rotation is suspected" is unimplementable: the failure *is* the detection
//!   event.
//! - **Never `LockFile`/`LockFileEx`, never `FILE_FLAG_DELETE_ON_CLOSE`.**

use std::ffi::c_void;
use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};

use windows::core::PCWSTR;
use windows::Win32::Foundation::{
    CloseHandle, ERROR_HANDLE_EOF, ERROR_IO_PENDING, HANDLE, WAIT_OBJECT_0,
};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, FileIdInfo, FileStandardInfo, GetFileInformationByHandleEx, ReadFile,
    FILE_FLAG_OVERLAPPED, FILE_FLAG_SEQUENTIAL_SCAN, FILE_ID_INFO, FILE_SHARE_DELETE,
    FILE_SHARE_MODE, FILE_SHARE_READ, FILE_SHARE_WRITE, FILE_STANDARD_INFO, OPEN_EXISTING,
};
use windows::Win32::System::Threading::{CreateEventW, WaitForSingleObject, INFINITE};
use windows::Win32::System::IO::{GetOverlappedResult, OVERLAPPED, OVERLAPPED_0, OVERLAPPED_0_0};

use crate::encoding::{detect, Detection, Sample, SAMPLE_BYTES};
use crate::lines::LineDecoder;

/// The share mode §5.1 makes mandatory. All three, and the reason for each:
///
/// - `READ` — another reader (or the writer re-opening to read) must not be locked out.
/// - `WRITE` — without it the writer cannot append, and a log viewer that stops the log is absurd.
/// - `DELETE` — without it `rename` and `DeleteFile` fail with `ERROR_SHARING_VIOLATION`, which is
///   every rotation scheme at once. This is the one that is easy to omit and hard to notice, since
///   nothing goes wrong until the writer rolls.
const SHARE_ALL: FILE_SHARE_MODE =
    FILE_SHARE_MODE(FILE_SHARE_READ.0 | FILE_SHARE_WRITE.0 | FILE_SHARE_DELETE.0);

/// `ERROR_SHARING_VIOLATION`. Named here because §5.1 requires it to be distinguished from every
/// other open failure — it is unfixable from the reader side, so the remedy is writer-side advice
/// rather than a retry.
const WIN32_SHARING_VIOLATION: u32 = 32;
const WIN32_FILE_NOT_FOUND: u32 = 2;
const WIN32_PATH_NOT_FOUND: u32 = 3;

/// Read size for the streaming path. §5.2 specifies 1–4 MB per request.
const READ_CHUNK: usize = 1024 * 1024;

#[derive(Debug)]
pub enum FileError {
    /// The **writer** holds an exclusive handle. Nothing the reader does can fix this, so §5.1
    /// specifies presenting the writer-side remedy by name — `shared: true` for a Serilog file
    /// sink, `keepFileOpen="false"` for NLog, `MinimalLock` for log4net.
    ///
    /// No handle-stealing, no `NtDuplicateObject`.
    SharingViolation,
    /// §5.5's "path missing" case is `tail -F` semantics — keep waiting, never an error dialog —
    /// so the caller needs this separable from a real failure.
    NotFound,
    Io(String),
}

impl std::fmt::Display for FileError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FileError::SharingViolation => write!(
                f,
                "the application writing this file holds it exclusively, so it cannot be read \
                 while it is open"
            ),
            FileError::NotFound => write!(f, "the file does not exist"),
            FileError::Io(m) => write!(f, "{m}"),
        }
    }
}

impl std::error::Error for FileError {}

impl From<FileError> for crate::Error {
    fn from(e: FileError) -> Self {
        crate::Error(e.to_string())
    }
}

type Result<T> = std::result::Result<T, FileError>;

/// File identity, from `GetFileInformationByHandleEx(FileIdInfo)`.
///
/// §5.5: **tab identity, dedup and rotation detection are keyed on this, never on the path
/// string.** Windows has had per-directory case sensitivity since 1803 and WSL-created directories
/// cannot be made insensitive, so comparing paths produces both false merges and false splits.
///
/// The legacy 64-bit `nFileIndex` is deliberately not used — it is not unique on ReFS.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct FileIdentity {
    pub volume_serial: u64,
    pub file_id: [u8; 16],
}

/// An open, followed file.
#[derive(Debug)]
pub struct LogFile {
    handle: HANDLE,
    path: PathBuf,
}

// The handle is owned, and every read passes its own `OVERLAPPED` with an explicit offset rather
// than moving a shared file pointer — which is what makes concurrent reads on one handle sound
// (§5.2).
unsafe impl Send for LogFile {}
unsafe impl Sync for LogFile {}

impl LogFile {
    /// Opens exactly as `SPEC.md` §5.1 specifies. There is no other open in the product.
    pub fn open(path: &Path) -> Result<Self> {
        Self::open_with_share_mode(path, SHARE_ALL)
    }

    fn open_with_share_mode(path: &Path, share: FILE_SHARE_MODE) -> Result<Self> {
        let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();

        let handle = unsafe {
            CreateFileW(
                PCWSTR(wide.as_ptr()),
                // Never `GENERIC_WRITE`. Tailhawk has no reason to write to a log and asking for
                // the access would make an exclusive-writer conflict our fault.
                windows::Win32::Foundation::GENERIC_READ.0,
                share,
                None,
                OPEN_EXISTING,
                FILE_FLAG_SEQUENTIAL_SCAN | FILE_FLAG_OVERLAPPED,
                None,
            )
        }
        .map_err(map_open_error)?;

        Ok(Self {
            handle,
            path: path.to_owned(),
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The current length, read from **our own handle**.
    ///
    /// §5.4 forbids the path-based alternatives (`GetFileAttributesEx`, `FindFirstFile`): NTFS
    /// replicates size into the directory entry only when the last handle closes, so a path stat
    /// on an actively-written log returns a frozen size forever. This is the single most
    /// destructive way to get following wrong, because it looks like the writer stopped.
    pub fn len(&self) -> Result<u64> {
        let mut info = FILE_STANDARD_INFO::default();
        unsafe {
            GetFileInformationByHandleEx(
                self.handle,
                FileStandardInfo,
                &mut info as *mut _ as *mut c_void,
                std::mem::size_of::<FILE_STANDARD_INFO>() as u32,
            )
        }
        .map_err(|e| FileError::Io(format!("querying the size of {}: {e}", self.path.display())))?;
        Ok(info.EndOfFile.max(0) as u64)
    }

    pub fn is_empty(&self) -> Result<bool> {
        Ok(self.len()? == 0)
    }

    pub fn identity(&self) -> Result<FileIdentity> {
        let mut info = FILE_ID_INFO::default();
        unsafe {
            GetFileInformationByHandleEx(
                self.handle,
                FileIdInfo,
                &mut info as *mut _ as *mut c_void,
                std::mem::size_of::<FILE_ID_INFO>() as u32,
            )
        }
        .map_err(|e| FileError::Io(format!("identifying {}: {e}", self.path.display())))?;
        Ok(FileIdentity {
            volume_serial: info.VolumeSerialNumber,
            file_id: info.FileId.Identifier,
        })
    }

    /// Reads at an explicit offset, leaving the file pointer alone.
    ///
    /// §5.2: this is what makes several reads on one handle safe — there is no shared pointer to
    /// contend for. Returns the byte count, which is short at end of file and zero past it.
    pub fn read_at(&self, offset: u64, buf: &mut [u8]) -> Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }

        // The handle is `FILE_FLAG_OVERLAPPED`, so every read needs its own `OVERLAPPED` whether
        // or not the caller wants to wait. A manual-reset event, initially unsignalled, is the
        // completion mechanism for this synchronous shape.
        let event = unsafe { CreateEventW(None, true, false, PCWSTR::null()) }
            .map_err(|e| FileError::Io(format!("creating a read event: {e}")))?;
        let _event = OwnedHandle(event);

        let mut overlapped = OVERLAPPED {
            Anonymous: OVERLAPPED_0 {
                Anonymous: OVERLAPPED_0_0 {
                    Offset: offset as u32,
                    OffsetHigh: (offset >> 32) as u32,
                },
            },
            hEvent: event,
            ..Default::default()
        };

        let started = unsafe { ReadFile(self.handle, Some(buf), None, Some(&mut overlapped)) };
        match started {
            Ok(()) => {}
            Err(e) if win32_code(&e) == Some(ERROR_IO_PENDING.0) => {
                // Waiting on the event rather than passing `bWait` to `GetOverlappedResult` keeps
                // the two paths identical below.
                let wait = unsafe { WaitForSingleObject(event, INFINITE) };
                if wait != WAIT_OBJECT_0 {
                    return Err(FileError::Io(format!(
                        "waiting for a read of {} to complete: {wait:?}",
                        self.path.display()
                    )));
                }
            }
            // Reading at or past end of file is not a failure; it is how the caller learns where
            // the end is.
            Err(e) if win32_code(&e) == Some(ERROR_HANDLE_EOF.0) => return Ok(0),
            Err(e) => {
                return Err(FileError::Io(format!(
                    "reading {} at offset {offset}: {e}",
                    self.path.display()
                )))
            }
        }

        let mut transferred = 0u32;
        match unsafe { GetOverlappedResult(self.handle, &overlapped, &mut transferred, false) } {
            Ok(()) => Ok(transferred as usize),
            Err(e) if win32_code(&e) == Some(ERROR_HANDLE_EOF.0) => Ok(0),
            Err(e) => Err(FileError::Io(format!(
                "completing a read of {} at offset {offset}: {e}",
                self.path.display()
            ))),
        }
    }
}

impl Drop for LogFile {
    fn drop(&mut self) {
        let _ = unsafe { CloseHandle(self.handle) };
    }
}

/// Closes a raw handle on the way out of a scope. Only the read event needs this; `LogFile` owns
/// its own.
struct OwnedHandle(HANDLE);

impl Drop for OwnedHandle {
    fn drop(&mut self) {
        let _ = unsafe { CloseHandle(self.0) };
    }
}

/// A file opened, sniffed for encoding, and ready to stream decoded lines.
///
/// **Decode before index.** The detection happens in `open`, before a single byte offset has been
/// recorded, because §5.3's chunk-assignment invariant needs the code-unit width and an indexer
/// that has already run cannot be given one retroactively.
pub struct FileSource {
    file: LogFile,
    detection: Detection,
    decoder: LineDecoder,
    /// How far into the file the decoder has consumed. Byte offsets, never character counts.
    offset: u64,
    buffer: Vec<u8>,
}

impl FileSource {
    pub fn open(path: &Path) -> Result<Self> {
        Self::from_file(LogFile::open(path)?)
    }

    pub fn from_file(file: LogFile) -> Result<Self> {
        let len = file.len()?;

        let mut head = vec![0u8; SAMPLE_BYTES.min(len as usize)];
        let read = file.read_at(0, &mut head)?;
        head.truncate(read);

        // Only worth a separate tail sample if it would not overlap the head. §5.6 wants both ends
        // because a file can change character partway through — a UTF-8 header written by one tool
        // and UTF-16 body appended by another is a real shape.
        let tail_bytes = if len > 2 * SAMPLE_BYTES as u64 {
            // Aligned down to 4, so the sample's own parity matches the file's for every code-unit
            // width. The absolute offset is carried anyway, but an aligned sample makes the
            // plausibility decode meaningful as well as the parity count.
            let start = (len - SAMPLE_BYTES as u64) & !3;
            let mut buf = vec![0u8; SAMPLE_BYTES];
            let read = file.read_at(start, &mut buf)?;
            buf.truncate(read);
            Some((buf, start))
        } else {
            None
        };

        let detection = detect(
            &head,
            tail_bytes.as_ref().map(|(bytes, offset)| Sample {
                bytes,
                offset: *offset,
            }),
            fallback_encoding(),
        );

        Ok(Self {
            decoder: LineDecoder::from_detection(&detection),
            file,
            detection,
            offset: 0,
            buffer: vec![0u8; READ_CHUNK],
        })
    }

    pub fn detection(&self) -> &Detection {
        &self.detection
    }

    pub fn file(&self) -> &LogFile {
        &self.file
    }

    /// Byte offset the decoder has consumed to. This is what the index is built from, and what
    /// makes the BOM's bytes count even though they are never rendered (§5.6).
    pub fn offset(&self) -> u64 {
        self.offset
    }

    /// Reads everything appended since the last call, emitting each complete line.
    ///
    /// A trailing partial line is **held**, not emitted — a writer that has flushed half a line is
    /// the normal state of a live log, and showing the half is worse than showing nothing. Returns
    /// the number of bytes consumed.
    pub fn pump(&mut self, mut on_line: impl FnMut(&str)) -> Result<u64> {
        let start = self.offset;
        loop {
            let read = self.file.read_at(self.offset, &mut self.buffer)?;
            if read == 0 {
                break;
            }
            self.decoder.push(&self.buffer[..read], &mut on_line);
            self.offset += read as u64;
        }
        Ok(self.offset - start)
    }

    /// Reads to end of file and closes the stream, emitting a final unterminated line if there is
    /// one. For a file that is still being written, use [`pump`](Self::pump) instead — this
    /// commits to there being no more bytes.
    pub fn read_to_end(&mut self, mut on_line: impl FnMut(&str)) -> Result<u64> {
        let consumed = self.pump(&mut on_line)?;
        self.decoder.finish(on_line);
        Ok(consumed)
    }
}

fn fallback_encoding() -> &'static encoding_rs::Encoding {
    crate::encoding::system_codepage()
}

fn map_open_error(e: windows::core::Error) -> FileError {
    match win32_code(&e) {
        Some(WIN32_SHARING_VIOLATION) => FileError::SharingViolation,
        Some(WIN32_FILE_NOT_FOUND) | Some(WIN32_PATH_NOT_FOUND) => FileError::NotFound,
        _ => FileError::Io(e.to_string()),
    }
}

/// The Win32 code inside an `HRESULT`, if it is a `HRESULT_FROM_WIN32` one.
fn win32_code(e: &windows::core::Error) -> Option<u32> {
    let hr = e.code().0 as u32;
    (hr & 0xFFFF_0000 == 0x8007_0000).then_some(hr & 0xFFFF)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::encoding::{Charset, Confidence};
    use std::fs;
    use std::io::Write;
    use std::sync::atomic::{AtomicU32, Ordering};

    /// A scratch directory that removes itself. Not `tempfile` — the dependency surface of a
    /// copy-and-run binary is a design constraint (`SPEC.md` §2), and this is eight lines.
    struct Scratch(PathBuf);

    impl Scratch {
        fn new(tag: &str) -> Self {
            static COUNTER: AtomicU32 = AtomicU32::new(0);
            let n = COUNTER.fetch_add(1, Ordering::Relaxed);
            let dir = std::env::temp_dir()
                .join(format!("tailhawk-test-{tag}-{}-{n}", std::process::id()));
            fs::create_dir_all(&dir).expect("scratch directory");
            Scratch(dir)
        }
        fn join(&self, name: &str) -> PathBuf {
            self.0.join(name)
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn write(path: &Path, bytes: &[u8]) {
        fs::write(path, bytes).expect("writing a fixture");
    }

    fn lines_of(path: &Path) -> Vec<String> {
        let mut source = FileSource::open(path).expect("opening the fixture");
        let mut out = Vec::new();
        source
            .read_to_end(|line| out.push(line.to_owned()))
            .expect("reading the fixture");
        out
    }

    fn utf16le(s: &str) -> Vec<u8> {
        s.encode_utf16().flat_map(|u| u.to_le_bytes()).collect()
    }

    // --- The guarantee ------------------------------------------------------------------------

    /// **This is the M1 done-criterion**, and the product promise in one test: with Tailhawk
    /// attached, the writer can append, rename, truncate and delete its own log.
    ///
    /// It is written against the *writer's* return codes rather than ours, because the failure
    /// mode being guarded against is one where Tailhawk is perfectly happy and the customer's
    /// application is the thing that breaks — silently, since log4net swallows the roll failure
    /// and just stops rolling.
    #[test]
    fn writer_safety_through_all_three_rotation_modes() {
        let scratch = Scratch::new("writer-safety");
        let log = scratch.join("app.log");
        write(&log, b"line one\r\nline two\r\n");

        let held = LogFile::open(&log).expect("Tailhawk opens the log");
        assert!(held.len().expect("size") > 0);

        // Append while we hold it. Needs FILE_SHARE_WRITE on our side.
        {
            let mut w = fs::OpenOptions::new()
                .append(true)
                .open(&log)
                .expect("the writer must be able to re-open its own log for append");
            w.write_all(b"line three\r\n").expect("append");
        }

        // Rename-and-recreate, log4net's default. Needs FILE_SHARE_DELETE — a rename of an open
        // file is a delete-class operation as far as sharing is concerned.
        let rolled = scratch.join("app.log.1");
        fs::rename(&log, &rolled).expect("rename-and-recreate must not be blocked");
        write(&log, b"after the roll\r\n");

        // Copy-truncate. Needs FILE_SHARE_WRITE: the truncating open asks for write access.
        fs::copy(&log, scratch.join("app.log.bak")).expect("copy");
        fs::OpenOptions::new()
            .write(true)
            .truncate(true)
            .open(&log)
            .expect("copy-truncate must not be blocked");

        // Retention deleting an old member. This is the one §5.2 is really about: a section handle
        // would block it, log4net would swallow the error, and rolling would silently stop.
        fs::remove_file(&rolled).expect("deleting a rolled member must not be blocked");

        // And the file we are actually holding open.
        fs::remove_file(&log).expect("deleting the followed file must not be blocked");

        drop(held);
    }

    /// The negative control. Without it the test above proves only that the writer happened to
    /// succeed, not that our share mode is why — and a share mode is exactly the kind of constant
    /// that gets "tidied" by someone who does not know what it costs.
    #[test]
    fn the_guarantee_fails_without_the_share_mode() {
        let scratch = Scratch::new("negative-control");
        let log = scratch.join("app.log");
        write(&log, b"line one\r\n");

        let held =
            LogFile::open_with_share_mode(&log, FILE_SHARE_READ).expect("opening read-shared only");

        let append = fs::OpenOptions::new().append(true).open(&log);
        assert!(
            append.is_err(),
            "without FILE_SHARE_WRITE the writer is locked out — which is the bug the real share \
             mode exists to prevent, so if this ever passes the test above has stopped meaning \
             anything"
        );
        assert!(
            fs::remove_file(&log).is_err(),
            "without FILE_SHARE_DELETE rotation is blocked"
        );

        drop(held);
    }

    #[test]
    fn a_sharing_violation_is_distinguished_from_every_other_failure() {
        let scratch = Scratch::new("sharing-violation");
        let log = scratch.join("exclusive.log");
        write(&log, b"held exclusively\r\n");

        // Stand in for a writer configured without shared access — log4net's default
        // `ExclusiveLock`, or a Serilog sink without `shared: true`.
        let exclusive = LogFile::open_with_share_mode(&log, FILE_SHARE_MODE(0))
            .expect("the stand-in writer opens it");

        match LogFile::open(&log) {
            Err(FileError::SharingViolation) => {}
            other => panic!("expected a sharing violation, got {other:?}"),
        }

        drop(exclusive);
        assert!(LogFile::open(&log).is_ok(), "and it opens once released");
    }

    #[test]
    fn a_missing_path_is_not_an_error_class_of_its_own_making() {
        let scratch = Scratch::new("missing");
        match LogFile::open(&scratch.join("never-existed.log")) {
            // §5.5's "waiting for <path>" state needs this separable, so it can keep retrying
            // rather than showing a dialog.
            Err(FileError::NotFound) => {}
            other => panic!("expected NotFound, got {other:?}"),
        }
    }

    // --- Identity and size ---------------------------------------------------------------------

    #[test]
    fn identity_survives_a_rename_and_distinguishes_a_recreated_file() {
        let scratch = Scratch::new("identity");
        let log = scratch.join("app.log");
        write(&log, b"original\r\n");

        let held = LogFile::open(&log).expect("open");
        let before = held.identity().expect("identity");

        fs::rename(&log, scratch.join("app.log.1")).expect("rename");
        assert_eq!(
            held.identity().expect("identity"),
            before,
            "identity is a property of the file, not of the path it currently answers to — which \
             is what makes it usable for rotation detection (§5.5)"
        );

        write(&log, b"recreated\r\n");
        let recreated = LogFile::open(&log).expect("open the new file");
        assert_ne!(
            recreated.identity().expect("identity"),
            before,
            "the recreated file at the same path is a different file"
        );
    }

    /// §5.4's trap, as a test: the size must come from our handle. A path-based stat on a file
    /// with an open write handle returns a stale figure, and the failure looks exactly like the
    /// writer having stopped.
    #[test]
    fn size_comes_from_our_handle_and_tracks_appends() {
        let scratch = Scratch::new("size");
        let log = scratch.join("app.log");
        write(&log, b"aaaa\r\n");

        let held = LogFile::open(&log).expect("open");
        assert_eq!(held.len().expect("size"), 6);

        let mut w = fs::OpenOptions::new()
            .append(true)
            .open(&log)
            .expect("append");
        w.write_all(b"bbbb\r\n").expect("write");
        w.flush().expect("flush");

        assert_eq!(
            held.len().expect("size"),
            12,
            "the appended bytes are visible on the handle we already hold"
        );
    }

    #[test]
    fn reading_past_the_end_yields_nothing_rather_than_failing() {
        let scratch = Scratch::new("eof");
        let log = scratch.join("app.log");
        write(&log, b"short\r\n");

        let held = LogFile::open(&log).expect("open");
        let mut buf = [0u8; 64];
        assert_eq!(held.read_at(0, &mut buf).expect("read"), 7);
        assert_eq!(held.read_at(7, &mut buf).expect("read at eof"), 0);
        assert_eq!(held.read_at(9999, &mut buf).expect("read past eof"), 0);
    }

    // --- Detect-then-decode, end to end ---------------------------------------------------------

    #[test]
    fn corpus_a_shape_log4net() {
        let scratch = Scratch::new("corpus-a");
        let log = scratch.join("app.log");
        write(
            &log,
            b"2026-07-31 09:14:14,123 INFO   Worker.Run  started\r\n\
              2026-07-31 09:14:15,004 ERROR  Worker.Run  failed\r\n",
        );
        assert_eq!(
            lines_of(&log),
            [
                "2026-07-31 09:14:14,123 INFO   Worker.Run  started",
                "2026-07-31 09:14:15,004 ERROR  Worker.Run  failed"
            ]
        );
    }

    /// Corpus B's known-answer property, through the real file path: BOM-less UTF-8 whose em
    /// dashes PowerShell 5.1 renders as `â€"`. The correct answer was known before this code was
    /// written, which is what makes it a regression test rather than a snapshot.
    #[test]
    fn corpus_b_shape_bomless_utf8_with_em_dashes() {
        let scratch = Scratch::new("corpus-b");
        let log = scratch.join("run.log");
        let text = "Batch run starting\n\
                    TIME\tINSTANCE\tSTAGE\n\
                    17:00:02\tINSTANCE-1\tstage — complete\n\
                    2026-07-31 17:00:09  [dispatch]  finished — 3 items\n";
        write(&log, text.as_bytes());

        let mut source = FileSource::open(&log).expect("open");
        assert_eq!(source.detection().charset, Charset::UTF_8);
        assert_eq!(source.detection().confidence, Confidence::High);

        let mut out = Vec::new();
        source
            .read_to_end(|l| out.push(l.to_owned()))
            .expect("read");
        assert_eq!(out.len(), 4);
        assert!(out[2].contains('—'), "{:?} lost the em dash", out[2]);
        assert!(!out[2].contains('â'), "{:?} was decoded as CP1252", out[2]);
    }

    #[test]
    fn utf16le_with_a_bom_through_the_file_path() {
        let scratch = Scratch::new("utf16");
        let log = scratch.join("ps.log");
        let mut bytes = vec![0xFF, 0xFE];
        bytes.extend_from_slice(&utf16le("first — line\r\nsecond line\r\n"));
        write(&log, &bytes);

        let mut source = FileSource::open(&log).expect("open");
        assert_eq!(source.detection().charset, Charset::UTF_16LE);
        assert_eq!(source.detection().bom_len, 2);

        let mut out = Vec::new();
        source
            .read_to_end(|l| out.push(l.to_owned()))
            .expect("read");
        assert_eq!(out, ["first — line", "second line"]);
    }

    /// A file long enough to be read in more than one 1 MB request, so the decoder's carry is
    /// exercised by the real read path rather than only by hand-split fixtures.
    #[test]
    fn a_multi_chunk_file_decodes_identically_to_a_small_one() {
        let scratch = Scratch::new("multi-chunk");
        let log = scratch.join("big.log");

        let mut text = String::new();
        // Non-ASCII in every line, so read boundaries land inside multi-byte sequences.
        for i in 0..60_000 {
            text.push_str(&format!("2026-07-31 09:14:14 INFO  line {i} — ok\r\n"));
        }
        write(&log, text.as_bytes());
        assert!(text.len() > 2 * READ_CHUNK, "the fixture must span reads");

        let mut count = 0usize;
        let mut last = String::new();
        let mut source = FileSource::open(&log).expect("open");
        source
            .read_to_end(|line| {
                count += 1;
                last.clear();
                last.push_str(line);
            })
            .expect("read");

        assert_eq!(count, 60_000);
        assert_eq!(last, "2026-07-31 09:14:14 INFO  line 59999 — ok");
        assert_eq!(
            source.offset(),
            text.len() as u64,
            "every byte is accounted for, which is what the index will be built on"
        );
    }

    #[test]
    fn a_partial_final_line_is_held_until_it_completes() {
        let scratch = Scratch::new("partial");
        let log = scratch.join("app.log");
        write(&log, b"complete\r\nhalf a li");

        let mut source = FileSource::open(&log).expect("open");
        let mut out = Vec::new();
        source.pump(|l| out.push(l.to_owned())).expect("first pump");
        assert_eq!(
            out,
            ["complete"],
            "a writer that has flushed half a line is the normal state of a live log"
        );

        let mut w = fs::OpenOptions::new()
            .append(true)
            .open(&log)
            .expect("append");
        w.write_all(b"ne\r\n").expect("write");
        w.flush().expect("flush");

        source
            .pump(|l| out.push(l.to_owned()))
            .expect("second pump");
        assert_eq!(out, ["complete", "half a line"]);
    }

    #[test]
    fn an_empty_file_opens_and_yields_nothing() {
        let scratch = Scratch::new("empty");
        let log = scratch.join("empty.log");
        write(&log, b"");
        assert!(lines_of(&log).is_empty());
    }
}
