//! Which editor opens a stack frame's file, and the command line that opens it at the line —
//! `docs/PROPOSAL-block-selection-and-rule-sets.md` §6.
//!
//! **Pure.** [`preferred`] picks a command from the editors the shell found installed, and
//! [`command_line`] fills a command in for one file and line. The shell only looks for the
//! executables and starts the process.
//!
//! **Nothing here goes through a shell.** The command is the reader's own setting; the path is
//! the only text that came from a log, it is substituted as one quoted argument, and the process
//! is started with `CreateProcessW`, never through `cmd.exe` and never with a `ShellExecute`
//! verb. That rules out *the file's default application* as a choice: the default for `.js`,
//! `.vbs` and `.ps1` files is to run them, and a Node frame names a `.js` file.

/// An editor this knows how to open at a line: its executable's file name, and the arguments
/// after it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Preset {
    pub name: &'static str,
    pub exe: &'static str,
    pub args: &'static str,
}

/// The editors, in the order [`preferred`] chooses between them when no command is set. Notepad
/// is last and always present, so a reader with nothing installed still gets the file.
pub const PRESETS: &[Preset] = &[
    Preset {
        name: "Visual Studio Code",
        exe: "Code.exe",
        args: r#"--goto "{path}:{line}:{column}""#,
    },
    Preset {
        name: "JetBrains Rider",
        exe: "rider64.exe",
        args: r#"--line {line} --column {column} "{path}""#,
    },
    Preset {
        name: "Visual Studio",
        exe: "devenv.exe",
        args: r#"/Edit "{path}""#,
    },
    Preset {
        name: "Notepad++",
        exe: "notepad++.exe",
        args: r#"-n{line} -c{column} "{path}""#,
    },
    Preset {
        name: "Notepad",
        exe: "notepad.exe",
        args: r#""{path}""#,
    },
];

/// The command for a preset installed at `exe_path`.
pub fn command_of(preset: &Preset, exe_path: &str) -> String {
    format!("\"{exe_path}\" {}", preset.args)
}

/// The command to use when the reader has set none: the first preset, in [`PRESETS`] order, that
/// `found` has an executable for.
pub fn preferred(found: &[(Preset, String)]) -> Option<String> {
    PRESETS.iter().find_map(|p| {
        found
            .iter()
            .find(|(f, _)| f == p)
            .map(|(f, exe)| command_of(f, exe))
    })
}

/// Characters an editor built for an ANSI code page would see as `"` or `\` after best-fit
/// mapping (WorstFit), so that one in a path could end its quoted argument or escape its quote.
/// None of the presets is such an editor; a reader's own command may name one.
const LOOKALIKES: &[char] = &[
    '\u{FF02}', '\u{201C}', '\u{201D}', '\u{2BA}', '\u{2DD}', '\u{30E}', '\u{2033}', '\u{FF3C}',
    '\u{FE68}', '\u{2216}', '\u{A5}', '\u{20A9}',
];

/// The command line that opens `path` at `line` and `column`, or `None` when `path` cannot be
/// passed safely.
///
/// `{path}`, `{line}` and `{column}` are replaced; a missing column is `1`. A `{path}` the command
/// does not already quote is quoted, and a command with no `{path}` gets one at the end, quoted.
/// **A path holding a quote is refused**: no Windows path does, and one would end the argument.
/// So is one holding a [`LOOKALIKES`] character, and one ending in a backslash, which escapes the
/// closing quote.
pub fn command_line(command: &str, path: &str, line: u32, column: Option<u32>) -> Option<String> {
    if path.contains('"')
        || path.contains(LOOKALIKES)
        || path.chars().any(char::is_control)
        || path.ends_with('\\')
    {
        return None;
    }
    let command = if command.contains("{path}") {
        command.to_string()
    } else {
        format!("{command} \"{{path}}\"")
    };
    let command = command
        .replace("{line}", &line.to_string())
        .replace("{column}", &column.unwrap_or(1).to_string());
    let mut out = String::with_capacity(command.len() + path.len());
    let mut rest = command.as_str();
    while let Some(at) = rest.find("{path}") {
        out.push_str(&rest[..at]);
        if out.matches('"').count() % 2 == 1 {
            out.push_str(path);
        } else {
            out.push('"');
            out.push_str(path);
            out.push('"');
        }
        rest = &rest[at + "{path}".len()..];
    }
    out.push_str(rest);
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn preset(name: &str) -> Preset {
        *PRESETS.iter().find(|p| p.name == name).expect(name)
    }

    #[test]
    fn each_preset_opens_its_editor_at_the_line() {
        let code = command_of(&preset("Visual Studio Code"), r"C:\Users\n\Code.exe");
        assert_eq!(
            command_line(&code, r"C:\src\B.cs", 42, Some(7)).as_deref(),
            Some(r#""C:\Users\n\Code.exe" --goto "C:\src\B.cs:42:7""#)
        );
        let npp = command_of(
            &preset("Notepad++"),
            r"C:\Program Files\Notepad++\notepad++.exe",
        );
        assert_eq!(
            command_line(&npp, r"C:\src\B.cs", 42, None).as_deref(),
            Some(r#""C:\Program Files\Notepad++\notepad++.exe" -n42 -c1 "C:\src\B.cs""#),
            "a missing column is the first"
        );
        let pad = command_of(&preset("Notepad"), r"C:\Windows\notepad.exe");
        assert_eq!(
            command_line(&pad, r"C:\My App\B.cs", 3, None).as_deref(),
            Some(r#""C:\Windows\notepad.exe" "C:\My App\B.cs""#)
        );
        let rider = command_of(&preset("JetBrains Rider"), r"C:\JB\rider64.exe");
        assert_eq!(
            command_line(&rider, r"C:\My App\B.cs", 42, Some(7)).as_deref(),
            Some(r#""C:\JB\rider64.exe" --line 42 --column 7 "C:\My App\B.cs""#)
        );
        let devenv = command_of(&preset("Visual Studio"), r"C:\VS\devenv.exe");
        assert_eq!(
            command_line(&devenv, r"C:\src\B.cs", 42, Some(7)).as_deref(),
            Some(r#""C:\VS\devenv.exe" /Edit "C:\src\B.cs""#),
            "Visual Studio opens the file but cannot be told the line"
        );
        assert_eq!(
            command_line(&code, r"C:\My App\B.cs", 1, None).as_deref(),
            Some(r#""C:\Users\n\Code.exe" --goto "C:\My App\B.cs:1:1""#),
            "a space stays inside the one argument"
        );
    }

    #[test]
    fn notepad_is_the_last_resort_and_the_order_is_the_presets() {
        assert_eq!(PRESETS.last().map(|p| p.exe), Some("notepad.exe"));
        let found = vec![
            (preset("Notepad"), r"C:\Windows\notepad.exe".to_string()),
            (preset("Notepad++"), r"C:\npp\notepad++.exe".to_string()),
            (
                preset("Visual Studio Code"),
                r"C:\code\Code.exe".to_string(),
            ),
        ];
        assert_eq!(
            preferred(&found).as_deref(),
            Some(r#""C:\code\Code.exe" --goto "{path}:{line}:{column}""#)
        );
        assert_eq!(
            preferred(&found[..2]).as_deref(),
            Some(r#""C:\npp\notepad++.exe" -n{line} -c{column} "{path}""#)
        );
        assert_eq!(preferred(&[]), None);
    }

    #[test]
    fn a_reader_s_own_command_gets_the_path_quoted_once() {
        assert_eq!(
            command_line("subl {path}:{line}", r"C:\My App\B.cs", 5, None).as_deref(),
            Some(r#"subl "C:\My App\B.cs":5"#),
            "an unquoted placeholder is quoted"
        );
        assert_eq!(
            command_line(r#"edit "{path}""#, r"C:\B.cs", 5, None).as_deref(),
            Some(r#"edit "C:\B.cs""#),
            "a quoted one is not quoted again"
        );
        assert_eq!(
            command_line("edit", r"C:\B.cs", 5, None).as_deref(),
            Some(r#"edit "C:\B.cs""#),
            "a command with no placeholder gets the path at the end"
        );
        assert_eq!(
            command_line("edit {path} {line}", r"C:\{line}\B.cs", 5, None).as_deref(),
            Some(r#"edit "C:\{line}\B.cs" 5"#),
            "the path is never rewritten"
        );
        assert_eq!(
            command_line("edit {path}", r"C:\100%\B.cs", 5, None).as_deref(),
            Some(r#"edit "C:\100%\B.cs""#),
            "no shell, so a percent is a percent"
        );
        assert_eq!(
            command_line(
                "ed {path} {column} {path}",
                r"C:\{path}\{column}",
                5,
                Some(2)
            )
            .as_deref(),
            Some(r#"ed "C:\{path}\{column}" 2 "C:\{path}\{column}""#),
            "every placeholder is filled, and none inside the path"
        );
        assert_eq!(
            command_line(
                r#""C:\Program Files\Ed\ed.exe" {path} -l {line}"#,
                r"C:\B.cs",
                5,
                None
            )
            .as_deref(),
            Some(r#""C:\Program Files\Ed\ed.exe" "C:\B.cs" -l 5"#),
            "a quoted executable leaves the path unquoted, so it is quoted"
        );
        assert_eq!(
            command_line(r#"ed --file="{path}""#, r"C:\My App\B.cs", 5, None).as_deref(),
            Some(r#"ed --file="C:\My App\B.cs""#)
        );
    }

    #[test]
    fn a_path_that_could_end_its_argument_is_refused() {
        for path in [
            r#"C:\a"b.cs"#,
            "C:\\a\u{FF02}b.cs",
            "C:\\a\u{201C}b.cs",
            "C:\\a\u{201D}b.cs",
            "C:\\a\u{2BA}b.cs",
            "C:\\a\u{2DD}b.cs",
            "C:\\a\u{30E}b.cs",
            "C:\\a\u{2033}b.cs",
            "C:\\a\u{FF3C}b.cs",
            "C:\\a\u{FE68}b.cs",
            "C:\\a\u{2216}b.cs",
            "C:\\a\u{A5}b.cs",
            "C:\\a\u{20A9}b.cs",
            "C:\\a\u{1}b.cs",
            "C:\\a\u{85}b.cs",
            "C:\\a\u{7F}b.cs",
            r"C:\src\",
        ] {
            assert_eq!(command_line("edit {path}", path, 1, None), None, "{path:?}");
        }
        assert_eq!(
            command_line("edit {path}", r"C:\Zürich\a b\ß.cs", 1, None).as_deref(),
            Some(r#"edit "C:\Zürich\a b\ß.cs""#),
            "any other character is a path's own"
        );
    }
}
