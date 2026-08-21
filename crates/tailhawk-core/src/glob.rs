//! Shell-style glob matching — `*` and `?`, and nothing else.
//!
//! `SPEC.md` §8.1 calls for "the simple kind a shell offers: `*` and `?` in a file name", and
//! §6.5.1 needs the same thing to decide whether a remembered format definition claims the file
//! being opened. Two callers, and they must not disagree — a watched folder that adopts
//! `app-2026.log` while the definition that should columnise it decides the same pattern does not
//! match would be maddening to diagnose, because each half is individually correct.
//!
//! **Purely textual, and deliberately ignorant of paths.** The two callers want different subjects:
//! a watched folder matches a *file name*, while a definition may claim a bare pattern (`*.log`) or
//! a rooted one (`C:\logs\ndc\*.log`). Choosing the subject is the caller's business. A matcher
//! that quietly picked one would be right for one caller and wrong for the other, and the wrongness
//! would show up on somebody's real log rather than in a test.
//!
//! **Case-insensitive**, because Windows paths are, and both callers are matching Windows paths.

/// Whether `text` matches `pattern`, where `*` stands for any run of characters (including none)
/// and `?` for exactly one.
///
/// Everything else is a literal, compared without regard to case. A pattern with no wildcard is
/// therefore just a case-insensitive equality test, which is what a caller passing a plain file
/// name should get.
pub fn matches(pattern: &str, text: &str) -> bool {
    // Recursive rather than iterative with backtracking indices: patterns here are file globs of a
    // few dozen characters, so the depth is bounded by the input a person typed, and the two-line
    // `*` case is far easier to see the correctness of than a hand-rolled restart pointer.
    fn go(p: &[char], n: &[char]) -> bool {
        match (p.first(), n.first()) {
            (None, None) => true,
            // `*` either stops here, or eats one character and stays. The `!n.is_empty()` guard is
            // what stops it recursing past the end of the text.
            (Some('*'), _) => go(&p[1..], n) || (!n.is_empty() && go(p, &n[1..])),
            (Some('?'), Some(_)) => go(&p[1..], &n[1..]),
            (Some(a), Some(b)) if a.eq_ignore_ascii_case(b) => go(&p[1..], &n[1..]),
            _ => false,
        }
    }
    let p: Vec<char> = pattern.chars().collect();
    let n: Vec<char> = text.chars().collect();
    go(&p, &n)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pattern_with_no_wildcard_is_a_case_insensitive_equality() {
        assert!(matches("app.log", "app.log"));
        assert!(matches("APP.LOG", "app.log"));
        assert!(!matches("app.log", "app.log.1"));
        assert!(!matches("app.log", "pp.log"));
    }

    #[test]
    fn star_spans_any_run_including_none() {
        assert!(matches("*.log", "app.log"));
        assert!(matches("*.log", ".log"), "a star may match nothing at all");
        assert!(matches("app*", "app"));
        assert!(matches("app*.log", "app-2026-08-21.log"));
        assert!(!matches("*.log", "app.txt"));
    }

    #[test]
    fn question_takes_exactly_one() {
        assert!(matches("log_00?.txt", "log_001.txt"));
        assert!(!matches("log_00?.txt", "log_0012.txt"));
        assert!(!matches("log_00?.txt", "log_00.txt"), "one, not zero");
    }

    /// Serilog's `rollOnFileSizeLimit` and date-rolled names are the shapes §5.5's rolling sets and
    /// §8.1's watched folders actually meet.
    #[test]
    fn the_rolled_file_shapes_the_spec_names_all_match() {
        assert!(matches("log_*.txt", "log_001.txt"));
        assert!(matches("log-*_*.txt", "log-20260728_001.txt"));
        assert!(matches("*.log", "jobdispatcher.log"));
    }

    /// Several stars in one pattern is where a naive matcher goes wrong, and where the recursion
    /// earns its place.
    #[test]
    fn several_stars_backtrack_correctly() {
        assert!(matches("*a*b*c*", "xxaxxbxxcxx"));
        assert!(!matches("*a*b*c*", "xxaxxcxxbxx"), "order still matters");
        assert!(matches("**", "anything"));
        assert!(matches("*", ""));
    }

    /// A rooted pattern is just text to this function — which is the point of the module note. The
    /// caller decides whether it is handing over a name or a whole path.
    #[test]
    fn a_rooted_pattern_is_matched_as_plain_text() {
        assert!(matches(r"C:\logs\ndc\*.log", r"C:\logs\ndc\api.log"));
        assert!(matches(r"c:\LOGS\ndc\*.log", r"C:\logs\NDC\api.log"));
        assert!(
            !matches(r"C:\logs\ndc\*.log", "api.log"),
            "a rooted pattern does not match a bare name; the caller chooses the subject"
        );
    }

    #[test]
    fn the_empty_pattern_matches_only_the_empty_text() {
        assert!(matches("", ""));
        assert!(!matches("", "a"));
        assert!(!matches("a", ""));
    }
}
