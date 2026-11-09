//! Stack frames in a record, and whether the file each one names can be opened here —
//! `docs/PROPOSAL-block-selection-and-rule-sets.md` §6, increment 1.
//!
//! Two pure functions. [`frames_of`] reads a record's text, continuation lines included, and
//! returns every frame that names a file and a line. [`resolve`] decides whether one frame's file
//! may be opened on this machine, given a probe for existence the caller supplies.
//!
//! **Log text is somebody else's.** A Loki tail is production output, so nothing here trusts a
//! path because it looks like one. [`resolve`] refuses a network, device or URL path on the text
//! alone, before the probe runs: testing a UNC path for existence makes Windows authenticate to
//! that host, which is a gift to whoever wrote the log.

use std::collections::BTreeMap;
use std::ops::Range;
use std::sync::OnceLock;

use regex::{Captures, Regex};

/// One frame: where it sits in the record, and what it names.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Frame {
    /// The bytes of the record the location came from: the path through the last number, or for a
    /// Java frame the file name through the line, since its path is not in the text.
    pub span: Range<usize>,
    /// The path exactly as the record writes it, except a Java frame's, which is built from its
    /// package because the record names only the file.
    pub path: String,
    /// The line, one-based, as the runtime wrote it.
    pub line: u32,
    /// The column, one-based, when the shape carries one.
    pub column: Option<u32>,
    /// The function, when the shape names one.
    pub function: Option<String>,
}

/// Whether a frame's file can be opened here.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Resolution {
    /// The local file to open.
    Found(String),
    /// Nothing at that path, or a path that names no place on this machine yet.
    NotFound,
    /// A path that is never probed or opened, and why.
    Refused(&'static str),
}

/// Every frame in `text`, in the order they appear.
///
/// **One location is read once.** The shapes are tried most specific first, and each match a
/// specific shape makes *claims* its text whether or not it yields a frame: a Node frame
/// `(C:\a.js:80:8)` is also a bare `path:line:col` and must not be listed twice, and a Node
/// frame the reader rejected, `(https://cdn.example/app.js:10:5)`, must not come back as a bare
/// `//cdn.example/app.js`. The claims are kept ordered, so a record of a hundred thousand
/// candidates costs a logarithm per candidate rather than a scan.
pub fn frames_of(text: &str) -> Vec<Frame> {
    let mut claimed: BTreeMap<usize, usize> = BTreeMap::new();
    let mut frames: Vec<Frame> = Vec::new();
    for shape in shapes() {
        for caps in shape.regex.captures_iter(text) {
            let whole = caps.get(0).map_or(0..0, |m| m.range());
            let frame = (shape.read)(text, &caps);
            let claim = match &frame {
                Some(_) | None if shape.claims => whole,
                Some(f) => f.span.clone(),
                None => continue,
            };
            let overlaps = claimed
                .range(..claim.end)
                .next_back()
                .is_some_and(|(_, &end)| end > claim.start);
            if overlaps {
                continue;
            }
            claimed.insert(claim.start, claim.end);
            if let Some(frame) = frame {
                frames.push(frame);
            }
        }
    }
    frames.sort_by_key(|f| f.span.start);
    frames
}

/// Whether `frame`'s file can be opened here. `exists` is asked only about a local absolute path,
/// with its dots collapsed, and that path is what a [`Resolution::Found`] carries.
///
/// **Refusal is decided on the text.** A network path (`\\host\share`, `//host/share`), a device
/// path (`\\?\`, `\\.\`), a URL other than a local `file:///C:/`, a reserved device name in any
/// component (`COM1.cs`, `NUL.js`), or a path holding a quote, a control character or a colon past
/// its drive never reaches `exists`, because probing a UNC path makes Windows authenticate to its
/// host and opening a device can block. A path that is relative, or rooted on no drive, names no
/// place here until increment 2's source roots say where it is, so it is not probed either.
pub fn resolve(frame: &Frame, exists: impl Fn(&str) -> bool) -> Resolution {
    let path = frame.path.as_str();
    if path.chars().any(|c| c == '"' || c.is_control()) {
        return Resolution::Refused("a character no Windows path holds");
    }
    let local = if let Some(rest) = path.strip_prefix("file://") {
        let Some(on_drive) = rest.strip_prefix('/').filter(|r| has_drive(r)) else {
            return Resolution::Refused("a file URL on another host");
        };
        match percent_decoded(on_drive) {
            Some(decoded) if !decoded.chars().any(|c| c == '"' || c.is_control()) => {
                decoded.replace('/', "\\")
            }
            _ => return Resolution::Refused("a file URL that does not decode to a path"),
        }
    } else if is_url(path) {
        return Resolution::Refused("a URL");
    } else {
        path.to_string()
    };
    if local.starts_with("\\\\")
        || local.starts_with("//")
        || local.starts_with("\\/")
        || local.starts_with("/\\")
    {
        return Resolution::Refused("a network or device path");
    }
    if !has_drive(&local) {
        return Resolution::NotFound;
    }
    if local[2..].contains(':') {
        return Resolution::Refused("a character no Windows path holds");
    }
    let local = collapsed(&local);
    if local.split('\\').any(is_device_name) {
        return Resolution::Refused("a device, not a file");
    }
    if exists(&local) {
        Resolution::Found(local)
    } else {
        Resolution::NotFound
    }
}

/// A drive path with `.`, `..` and empty segments taken out and every separator a `\`.
///
/// **Windows does the same to the text before it touches the disk**, so `C:\B.cs\anything\..`
/// exists whenever `C:\B.cs` does. Probing and opening the collapsed path means a segment the log
/// wrote and then took away never reaches an editor's command line.
fn collapsed(path: &str) -> String {
    let mut segments: Vec<&str> = Vec::new();
    for segment in path[3..].split(['\\', '/']) {
        match segment {
            "" | "." => {}
            ".." => {
                segments.pop();
            }
            _ => segments.push(segment),
        }
    }
    format!("{}\\{}", &path[..2], segments.join("\\"))
}

/// Whether `path` starts with a drive and a separator, `C:\` or `C:/`.
fn has_drive(path: &str) -> bool {
    let b = path.as_bytes();
    b.len() >= 3 && b[0].is_ascii_alphabetic() && b[1] == b':' && (b[2] == b'\\' || b[2] == b'/')
}

/// Whether a path component names one of Windows' reserved devices, which it does in any
/// directory and with any extension: `COM1.cs` is the serial port. Trailing dots and spaces are
/// ignored as Windows ignores them, and the superscript digits count, as Windows counts them.
fn is_device_name(component: &str) -> bool {
    let stem = component.split('.').next().unwrap_or("");
    let stem = stem.trim_end_matches([' ', '.']).to_uppercase();
    if matches!(
        stem.as_str(),
        "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$"
    ) {
        return true;
    }
    let mut chars = stem.chars();
    let prefix: String = chars.by_ref().take(3).collect();
    let digit: Vec<char> = chars.collect();
    (prefix == "COM" || prefix == "LPT")
        && digit.len() == 1
        && (digit[0].is_ascii_digit() || matches!(digit[0], '¹' | '²' | '³'))
}

/// Whether `path` starts with a URL scheme and `://`. A drive letter is not a scheme: `C:\` has
/// one letter and no `//`.
fn is_url(path: &str) -> bool {
    match path.find("://") {
        Some(at) if at >= 2 => {
            let scheme = &path[..at];
            scheme.starts_with(|c: char| c.is_ascii_alphabetic())
                && scheme
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
        }
        _ => false,
    }
}

/// A URL path with its `%XX` escapes decoded, or `None` when an escape is malformed or the bytes
/// are not UTF-8.
fn percent_decoded(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            out.push(u8::from_str_radix(text.get(i + 1..i + 3)?, 16).ok()?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

/// One way a runtime writes a location: the pattern, how its captures become a frame, and whether
/// a match claims its text even when the reader turns it down.
struct Shape {
    regex: Regex,
    read: fn(&str, &Captures<'_>) -> Option<Frame>,
    claims: bool,
}

/// The extensions the bare `path:line[:col]` shape accepts. **Without this list a timestamp's
/// `12:05:33` and a host's `example.com:443` both read as frames**; the other shapes carry their
/// own evidence and do not consult it. It is not enough on its own, because `.pl`, `.rs` and
/// `.py` are also countries: [`read_bare`] adds the rest.
const SOURCE_EXTENSIONS: &[&str] = &[
    "c", "cc", "cjs", "clj", "cpp", "cs", "cshtml", "cts", "cxx", "dart", "erl", "ex", "exs", "fs",
    "fsx", "go", "groovy", "h", "hh", "hpp", "hs", "java", "jl", "js", "jsx", "kt", "kts", "lua",
    "m", "mjs", "mm", "mts", "nim", "php", "pl", "pm", "ps1", "psm1", "py", "razor", "rb", "rs",
    "scala", "sh", "sql", "svelte", "swift", "ts", "tsx", "vb", "vue", "zig",
];

/// The shapes, most specific first.
fn shapes() -> &'static [Shape] {
    static SHAPES: OnceLock<Vec<Shape>> = OnceLock::new();
    SHAPES.get_or_init(|| {
        let shape = |source: &str, read, claims| Shape {
            regex: Regex::new(source).unwrap_or_else(|e| panic!("frame pattern {source:?}: {e}")),
            read,
            claims,
        };
        vec![
            shape(
                r"(?m)^[ \t]*at (?P<func>.+?) in (?P<path>.+?):line (?P<line>\d+)[ \t]*\r?$",
                read_plain,
                true,
            ),
            shape(
                r#"File "(?P<path>[^"<\r\n][^"\r\n]*)", line (?P<line>\d+)(?:, in (?P<func>[^\r\n]*[^\s]))?"#,
                read_plain,
                true,
            ),
            shape(
                r"\bat (?:(?:[\w.$-]+(?:@[\w.+-]+)?)?/){0,2}(?P<func>[\w$]+(?:\.[\w$<>]+)+)\((?P<file>[\w$-]+\.(?:java|kt|kts|scala|groovy)):(?P<line>\d+)\)",
                read_java,
                true,
            ),
            shape(
                r"(?m)\bat (?P<func>[^\r\n]+?) \((?P<path>[^\r\n]+?):(?P<line>\d+):(?P<col>\d+)\)[ \t]*\r?$",
                read_node,
                true,
            ),
            shape(
                r"(?m)\bat (?P<path>(?:[A-Za-z]:[\\/]|/|\\\\|\.{1,2}[\\/]|file://)[^\r\n]*?):(?P<line>\d+):(?P<col>\d+)[ \t]*\r?$",
                read_node,
                true,
            ),
            shape(
                r#"(?m)(?:^|[\s>\]])(?P<path>[A-Za-z]:[\\/][^\r\n:*?"<>|]*?\.\w+|[^\s():*?"<>|\[\]]+\.\w+)\((?P<line>\d+)(?:,(?P<col>\d+))?(?:,\d+,\d+)?\)\s*:\s*(?:fatal\s+)?(?i:error|warning|info|message|note)\b"#,
                read_plain,
                true,
            ),
            shape(
                r#"(?P<path>[A-Za-z]:[\\/][^\r\n:*?"<>|]*?\.(?P<dext>[A-Za-z0-9]+)|[\w.\\/-]*\.(?P<ext>[A-Za-z0-9]+)):(?P<line>\d+)(?::(?P<col>\d+))?"#,
                read_bare,
                false,
            ),
        ]
    })
}

/// A frame from the `path`, `line`, `col` and `func` captures, spanning path through the last
/// number.
fn read_plain(_text: &str, caps: &Captures<'_>) -> Option<Frame> {
    let path = caps.name("path")?;
    let line = caps.name("line")?;
    let column = caps.name("col");
    let end = column.map_or(line.end(), |c| c.end());
    Some(Frame {
        span: path.start()..end,
        path: path.as_str().to_string(),
        line: line.as_str().parse().ok()?,
        column: column.and_then(|c| c.as_str().parse().ok()),
        function: caps.name("func").map(|f| f.as_str().to_string()),
    })
}

/// A Node frame, whose path must look like a file: `node:internal/…`, Node 14's bare
/// `internal/modules/…`, a bundler's `webpack:///` and an `https://` script do not.
fn read_node(text: &str, caps: &Captures<'_>) -> Option<Frame> {
    let path = caps.name("path")?.as_str();
    let file_like = has_drive(path)
        || (path.starts_with('/') && !path.starts_with("//"))
        || path.starts_with("\\\\")
        || path.starts_with("./")
        || path.starts_with(".\\")
        || path.starts_with("../")
        || path.starts_with("..\\")
        || path.starts_with("file://");
    if file_like {
        read_plain(text, caps)
    } else {
        None
    }
}

/// A bare `path:line[:col]`, counted only when it reads as a source file and not as a host.
///
/// Three conditions, because `.pl`, `.rs` and `.py` are countries as well as languages: the
/// extension is a source file's; the text before it is not a scheme's `:`, an address's `@` or the
/// middle of a word, so the `k:/` of `webpack:///` is not a drive;
/// and a path with no directory in it must carry a column too, so `smtp.firma.pl:587` is a mail
/// server and `x.py:12:5` is a file.
fn read_bare(text: &str, caps: &Captures<'_>) -> Option<Frame> {
    let path = caps.name("path")?;
    let ext = caps.name("ext").or_else(|| caps.name("dext"))?;
    if !SOURCE_EXTENSIONS.contains(&ext.as_str().to_ascii_lowercase().as_str()) {
        return None;
    }
    let before = &text[..path.start()];
    if before.ends_with([':', '@'])
        || before.ends_with(|c: char| c.is_alphanumeric() || c == '_')
        || path.as_str().starts_with("//")
        || path.as_str().contains("://")
    {
        return None;
    }
    let has_directory = path.as_str().contains(['\\', '/']);
    if !has_directory && caps.name("col").is_none() {
        return None;
    }
    read_plain(text, caps)
}

/// A Java frame, which names only its file: the path is the package's directories and the file,
/// `com.acme.Foo.bar` with `Foo.java` giving `com/acme/Foo.java`.
fn read_java(_text: &str, caps: &Captures<'_>) -> Option<Frame> {
    let func = caps.name("func")?;
    let file = caps.name("file")?;
    let line = caps.name("line")?;
    let segments: Vec<&str> = func.as_str().split('.').collect();
    let package = &segments[..segments.len().saturating_sub(2)];
    let path = package.iter().map(|s| format!("{s}/")).collect::<String>() + file.as_str();
    Some(Frame {
        span: file.start()..line.end(),
        path,
        line: line.as_str().parse().ok()?,
        column: None,
        function: Some(func.as_str().to_string()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn one(text: &str) -> Frame {
        let frames = frames_of(text);
        assert_eq!(frames.len(), 1, "one frame in {text:?}, got {frames:?}");
        frames.into_iter().next().unwrap()
    }

    fn located(text: &str) -> (String, u32, Option<u32>) {
        let f = one(text);
        (f.path, f.line, f.column)
    }

    #[test]
    fn a_dotnet_frame_names_its_path_line_and_method() {
        let f = one(r"   at Nurtur.Api.Foo.Bar(Int32 id) in C:\src\Nurtur.Api\Foo.cs:line 42");
        assert_eq!(f.path, r"C:\src\Nurtur.Api\Foo.cs");
        assert_eq!((f.line, f.column), (42, None));
        assert_eq!(f.function.as_deref(), Some("Nurtur.Api.Foo.Bar(Int32 id)"));
        assert_eq!(
            located(r"   at A.B() in C:\Program Files\App\B.cs:line 7"),
            (r"C:\Program Files\App\B.cs".to_string(), 7, None),
            "a path with a space in it"
        );
        assert_eq!(
            located("   at A.B() in /src/Api/Controllers/B.cs:line 9"),
            ("/src/Api/Controllers/B.cs".to_string(), 9, None),
            "a Linux container's path"
        );
    }

    #[test]
    fn a_node_frame_names_its_path_line_and_column() {
        let f = one(r"    at merge (C:\dev\m\harness\tsc-merge.js:80:8)");
        assert_eq!(
            (f.path.as_str(), f.line, f.column),
            (r"C:\dev\m\harness\tsc-merge.js", 80, Some(8))
        );
        assert_eq!(f.function.as_deref(), Some("merge"));
        let f = one(r"    at C:\dev\m\harness\tsc-merge.js:129:21");
        assert_eq!(
            (f.path.as_str(), f.line, f.column),
            (r"C:\dev\m\harness\tsc-merge.js", 129, Some(21))
        );
        assert_eq!(f.function, None);
        assert_eq!(
            located("    at load (file:///C:/dev/app/a.mjs:3:9)"),
            ("file:///C:/dev/app/a.mjs".to_string(), 3, Some(9)),
            "an ES module's frame keeps its URL; resolve decides what it means"
        );
    }

    #[test]
    fn a_python_frame_names_its_path_line_and_function() {
        let f = one(r#"  File "C:\dev\x\app.py", line 10, in main"#);
        assert_eq!(
            (f.path.as_str(), f.line, f.column),
            (r"C:\dev\x\app.py", 10, None)
        );
        assert_eq!(f.function.as_deref(), Some("main"));
        assert_eq!(
            located(r#"  File "/srv/app/x.py", line 3"#),
            ("/srv/app/x.py".to_string(), 3, None)
        );
    }

    #[test]
    fn a_java_frame_takes_its_path_from_the_package() {
        let f = one("\tat com.acme.billing.Invoice.total(Invoice.java:42)");
        assert_eq!(
            (f.path.as_str(), f.line),
            ("com/acme/billing/Invoice.java", 42)
        );
        assert_eq!(
            f.function.as_deref(),
            Some("com.acme.billing.Invoice.total")
        );
        assert_eq!(
            located("\tat com.acme.Outer$Inner.run(Outer.kt:7)"),
            ("com/acme/Outer.kt".to_string(), 7, None),
            "a nested class's file is its own name's, not the outer type's segment"
        );
    }

    #[test]
    fn a_compiler_diagnostic_names_its_path_line_and_column() {
        assert_eq!(
            located("  --> src/main.rs:12:5"),
            ("src/main.rs".to_string(), 12, Some(5))
        );
        assert_eq!(
            located("./main.go:42: undefined: x"),
            ("./main.go".to_string(), 42, None)
        );
        assert_eq!(
            located("/usr/src/app/x.c:3:1: error: expected ';'"),
            ("/usr/src/app/x.c".to_string(), 3, Some(1))
        );
        assert_eq!(
            located(r"C:\dev\Api\Foo.cs(42,7): error CS1002: ; expected"),
            (r"C:\dev\Api\Foo.cs".to_string(), 42, Some(7)),
            "MSBuild's form"
        );
        assert_eq!(
            located(r"C:\dev\Api\Foo.cs(42): warning CS0168: unused"),
            (r"C:\dev\Api\Foo.cs".to_string(), 42, None)
        );
    }

    #[test]
    fn the_near_misses_are_not_frames() {
        for text in [
            "2026-11-02 12:05:33.123 [Information] started",
            "GET https://example.com:443/x 200",
            "listening on http://localhost:5000",
            "retried at line 42",
            "rotated report.log:12",
            "    at async Promise.all (index 0)",
            "    at node:internal/modules/cjs/loader:1105:14",
            "    at Module._compile (node:internal/modules/cjs/loader:1105:14)",
            "\tat java.base/jdk.internal.misc.Unsafe.park(Native Method)",
            "\tat com.acme.Foo.bar(Unknown Source)",
            r#"  File "<stdin>", line 1, in <module>"#,
            "version 1.2.3:4",
            "    at Module._compile (internal/modules/cjs/loader.js:1063:30)",
            "    at f (https://cdn.example.com/app.js:10:5)",
            "    at f (webpack:///./src/x.js:1:2)",
            "webpack:///./src/x.js:1:2",
            "connecting to smtp.firma.pl:587",
            "mail from user@mail.example.rs:25",
            "Processed batch.Size(500): done",
            "rotated logs/app.log:12:3",
            r"build42C:\src\a.cs:12:3",
        ] {
            assert_eq!(frames_of(text), Vec::new(), "{text:?}");
        }
    }

    #[test]
    fn the_forms_a_build_server_and_a_module_system_add_are_still_frames() {
        assert_eq!(
            located("\tat app//com.acme.Foo.bar(Foo.java:12)"),
            ("com/acme/Foo.java".to_string(), 12, None),
            "a named class loader in an unnamed module"
        );
        assert_eq!(
            located("\tat java.base@21-ea/java.lang.Thread.run(Thread.java:1583)"),
            ("java/lang/Thread.java".to_string(), 1583, None),
            "a module with a version"
        );
        assert_eq!(
            located(r"1>C:\a\Foo.cs(4,2): error CS1002: ; expected"),
            (r"C:\a\Foo.cs".to_string(), 4, Some(2)),
            "MSBuild's project-numbered prefix"
        );
        assert_eq!(
            located("##[error]src/x.ts(4,2): error TS2322: no"),
            ("src/x.ts".to_string(), 4, Some(2)),
            "a pipeline's annotation"
        );
        assert_eq!(
            located(r"2026-11-02 10:41:07 C:\Program Files\App\Foo.cs(9): warning CS0168: x"),
            (r"C:\Program Files\App\Foo.cs".to_string(), 9, None),
            "a timestamp before it, and a space in the path"
        );
    }

    #[test]
    fn a_path_with_spaces_or_parentheses_is_read_whole() {
        assert_eq!(
            located(r"    at f (C:\Program Files (x86)\app\a.js:1:2)"),
            (r"C:\Program Files (x86)\app\a.js".to_string(), 1, Some(2))
        );
        assert_eq!(
            located(r"    at C:\Program Files\app\a.js:1:2"),
            (r"C:\Program Files\app\a.js".to_string(), 1, Some(2))
        );
        assert_eq!(
            located(r"error at C:\My Projects\x.rs:3:4"),
            (r"C:\My Projects\x.rs".to_string(), 3, Some(4))
        );
        assert_eq!(
            located("x.py:12:5: E501 line too long"),
            ("x.py".to_string(), 12, Some(5)),
            "no directory, but a column, so a file"
        );
    }

    #[test]
    fn one_location_is_one_frame_whichever_shapes_could_read_it() {
        assert_eq!(frames_of(r"    at merge (C:\dev\a.js:80:8)").len(), 1);
        assert_eq!(frames_of(r"    at C:\dev\a.js:80:8").len(), 1);
    }

    #[test]
    fn each_shape_spans_exactly_its_location() {
        for (text, span) in [
            (r"   at A.B() in C:\src\B.cs:line 3", r"C:\src\B.cs:line 3"),
            (
                r#"  File "C:\x\é.py", line 10, in main"#,
                r#"C:\x\é.py", line 10"#,
            ),
            ("\tat com.acme.Foo.bar(Foo.java:42)", "Foo.java:42"),
            (r"    at merge (C:\dév\a.js:80:8)", r"C:\dév\a.js:80:8"),
            (r"    at C:\dev\a.js:129:21", r"C:\dev\a.js:129:21"),
            (
                r"C:\dev\Foo.cs(42,7): error CS1002: x",
                r"C:\dev\Foo.cs(42,7",
            ),
            ("été → src/main.rs:12:5", "src/main.rs:12:5"),
        ] {
            let f = one(text);
            assert_eq!(&text[f.span.clone()], span, "{text:?}");
        }
    }

    #[test]
    fn every_frame_of_a_record_is_found_with_its_span() {
        let text = "System.InvalidOperationException: nope\r\n   at A.B() in C:\\src\\B.cs:line 3\r\n   at A.C() in C:\\src\\C.cs:line 14\r\n";
        let frames = frames_of(text);
        assert_eq!(
            frames.iter().map(|f| f.line).collect::<Vec<_>>(),
            vec![3, 14]
        );
        for f in &frames {
            assert!(
                text[f.span.clone()].starts_with(&f.path),
                "{:?}",
                &text[f.span.clone()]
            );
            assert!(
                text[f.span.clone()].ends_with(&f.line.to_string()),
                "{:?}",
                &text[f.span.clone()]
            );
        }
    }

    fn frame(path: &str) -> Frame {
        Frame {
            span: 0..path.len(),
            path: path.to_string(),
            line: 1,
            column: None,
            function: None,
        }
    }

    fn never(path: &str) -> bool {
        panic!("probed {path:?}, which must be refused on its text")
    }

    #[test]
    fn a_local_absolute_path_is_found_when_it_exists() {
        assert_eq!(
            resolve(&frame(r"C:\src\B.cs"), |p| p == r"C:\src\B.cs"),
            Resolution::Found(r"C:\src\B.cs".into())
        );
        assert_eq!(
            resolve(&frame("C:/src/B.cs"), |p| p == r"C:\src\B.cs"),
            Resolution::Found(r"C:\src\B.cs".into())
        );
        assert_eq!(
            resolve(&frame(r"C:\src\gone.cs"), |_| false),
            Resolution::NotFound
        );
    }

    #[test]
    fn a_local_file_url_is_its_path() {
        assert_eq!(
            resolve(&frame("file:///C:/dev/app/a.mjs"), |p| p
                == r"C:\dev\app\a.mjs"),
            Resolution::Found(r"C:\dev\app\a.mjs".into())
        );
        assert_eq!(
            resolve(&frame("file:///C:/My%20App/a.mjs"), |p| p
                == r"C:\My App\a.mjs"),
            Resolution::Found(r"C:\My App\a.mjs".into()),
            "its escapes decoded"
        );
        assert!(
            matches!(
                resolve(&frame("file:///C:/a%2"), never),
                Resolution::Refused(_)
            ),
            "a broken escape"
        );
    }

    #[test]
    fn a_path_naming_no_place_here_yet_is_not_probed() {
        for path in [
            "/src/Api/B.cs",
            "src/main.rs",
            "./main.go",
            "com/acme/Foo.java",
            r"C:src\a.cs",
        ] {
            assert_eq!(
                resolve(&frame(path), never),
                Resolution::NotFound,
                "{path:?}"
            );
        }
    }

    #[test]
    fn network_device_and_url_paths_are_refused_without_a_probe() {
        for path in [
            r"\\evil\share\a.cs",
            "//evil/share/a.cs",
            r"\\?\C:\src\a.cs",
            r"\\.\pipe\x.cs",
            "file://evil/share/a.js",
            "https://evil.example/a.js",
            r#"C:\src\a".cs"#,
            "C:\\src\\a\u{1}.cs",
            "file:///C:/a%22.cs",
            "file:///C:/a%00.cs",
            "file:////evil/share/a.js",
            "file:///C:/a%+1.cs",
            r"C:\src\a.cs:evil",
        ] {
            assert!(
                matches!(resolve(&frame(path), never), Resolution::Refused(_)),
                "{path:?}"
            );
        }
    }

    #[test]
    fn a_reserved_device_name_is_refused_in_any_directory() {
        for path in [
            r"C:\src\COM1.cs",
            r"C:\x\NUL.js",
            r"C:\x\con .txt",
            r"C:\x\LPT¹.c",
            r"C:\x\CONIN$",
            r"C:\aux\B.cs",
        ] {
            assert!(
                matches!(resolve(&frame(path), never), Resolution::Refused(_)),
                "{path:?}"
            );
        }
        assert_eq!(
            resolve(&frame(r"C:\src\COM10.cs"), |_| true),
            Resolution::Found(r"C:\src\COM10.cs".into()),
            "COM10 is a file"
        );
        assert_eq!(
            resolve(&frame("C://x.cs"), |p| p == r"C:\x.cs"),
            Resolution::Found(r"C:\x.cs".into()),
            "a doubled separator after a drive is still on the drive"
        );
    }

    #[test]
    fn what_is_probed_and_opened_is_the_path_with_its_dots_collapsed() {
        let only = |want: &'static str| move |p: &str| p == want;
        assert_eq!(
            resolve(&frame(r"C:\src\B.cs\a b ＂ c\.."), only(r"C:\src\B.cs")),
            Resolution::Found(r"C:\src\B.cs".into()),
            "a segment that .. takes away never reaches the editor"
        );
        assert_eq!(
            resolve(&frame(r"C:\proj\..\lib\.\x.cs"), only(r"C:\lib\x.cs")),
            Resolution::Found(r"C:\lib\x.cs".into())
        );
        assert_eq!(
            resolve(&frame("C:/../../x.cs"), only(r"C:\x.cs")),
            Resolution::Found(r"C:\x.cs".into()),
            "nothing climbs above the drive"
        );
        assert_eq!(
            resolve(&frame(r"C:\src\NUL\.."), |_| true),
            Resolution::Found(r"C:\src".into()),
            "a device name taken away is no longer a device"
        );
    }
}
