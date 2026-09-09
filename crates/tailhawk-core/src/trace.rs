//! The trace identifier a line carries — `UI-DESIGN.md` §7's correlation, at the one rung the owner
//! asked for on 2026-09-07.
//!
//! > "Follow this trace": take the `trace_id` of the line under the caret and filter the view to it.
//!
//! **The indented tree is deliberately not here.** The measurements that day found `trace_id` and
//! `span_id` on every line of forty live services and a parent id on only eight of them, so a tree
//! would have to infer most of its edges from time containment — right for a synchronous fan-out
//! and wrong for anything queued or retried. The owner's judgement was that a span tree is not what
//! a tail app is for, and that Grafana and Tempo already draw it from the same data. `LOKI.md` §8
//! carries the measurements and the decision.
//!
//! What is here is the half that costs nothing and answers the question a person actually asks at
//! the keyboard: *show me everything that was part of this call*.

/// The keys a trace identifier travels under, in the order they are looked for.
///
/// **Measured, not guessed.** Against the owner's estate on 2026-09-07, every line of forty live
/// services carried OpenTelemetry's `trace_id`; the .NET services additionally write Serilog's
/// `TraceId` for their own spans. `traceparent` is the W3C header and appears in some HTTP logs
/// carrying the id inside a longer field, so it is deliberately **not** here: half of a
/// `traceparent` is a version and a flag, and filtering on the whole string would match nothing.
pub const TRACE_KEYS: [&str; 3] = ["trace_id", "TraceId", "traceId"];

/// The trace identifier one line carries, if it carries one.
///
/// **A scan for the key rather than a parse of the line**, for the reason `lokiwire.rs` gives about
/// its own reader: there is no JSON crate in the tree, this needs one string value out of a flat
/// object, and a line the scan does not understand should yield nothing rather than an error a user
/// has to read. A line that is not JSON at all — a plain text log — simply has no trace id, which
/// is the truthful answer.
///
/// The value must look like a trace id to be believed: hexadecimal, and long enough not to be a
/// counter. That is what stops `"trace_id": "n/a"` from becoming a filter that matches every line
/// carrying those three characters.
pub fn trace_id(line: &str) -> Option<&str> {
    for key in TRACE_KEYS {
        if let Some(found) = value_of(line, key).filter(|v| looks_like_a_trace(v)) {
            return Some(found);
        }
    }
    None
}

/// The string value of `"key"` in a flat JSON object, without unescaping it.
///
/// Borrowed rather than owned, because the caller filters on the bytes as they appear in the line —
/// an unescaped copy would not match the text the filter searches.
fn value_of<'a>(line: &'a str, key: &str) -> Option<&'a str> {
    let quoted = format!("\"{key}\"");
    let at = line.find(&quoted)? + quoted.len();
    let rest = line.get(at..)?.trim_start();
    let rest = rest.strip_prefix(':')?.trim_start();
    let rest = rest.strip_prefix('"')?;
    let end = rest.find('"')?;
    rest.get(..end)
}

/// Whether a value is plausibly a trace identifier: hexadecimal and at least eight digits of it.
///
/// W3C makes a trace id sixteen bytes, written as thirty-two hex characters, and a span id half
/// that. Eight is the floor rather than thirty-two so a shorter house format still works, and it is
/// enough to refuse `"unknown"`, `"0"` and `"n/a"`.
fn looks_like_a_trace(value: &str) -> bool {
    value.len() >= 8
        && value.len() <= 64
        && value.chars().all(|c| c.is_ascii_hexdigit())
        && value.chars().any(|c| c != '0')
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The shape the estate actually writes, from a real spilled line.
    #[test]
    fn a_clef_line_gives_up_its_trace_id() {
        let line = r#"{"@t":"2026-09-07T12:45:09.7325995Z","@l":"Information","app":"nurtur-messaging-ui-api","span_id":"fe54a6e74f81bab4","trace_id":"d213cb0ae9cc2cc8cab31b9cd15d0854","@m":"End processing HTTP request"}"#;
        assert_eq!(trace_id(line), Some("d213cb0ae9cc2cc8cab31b9cd15d0854"));
    }

    /// Serilog's own spelling, which the .NET services write beside the OpenTelemetry one.
    #[test]
    fn the_serilog_spelling_is_read_too() {
        let line = r#"{"@t":"x","TraceId":"32b7381983d5dd2758f26ac5db572743","@m":"y"}"#;
        assert_eq!(trace_id(line), Some("32b7381983d5dd2758f26ac5db572743"));
        let camel = r#"{"traceId":"32b7381983d5dd2758f26ac5db572743"}"#;
        assert_eq!(trace_id(camel), Some("32b7381983d5dd2758f26ac5db572743"));
    }

    /// **A line with no trace has no trace**, and a plain text log is the commonest such line.
    #[test]
    fn a_line_without_one_says_so_rather_than_guessing() {
        assert_eq!(trace_id("2026-09-07 12:45:09 INF Started"), None);
        assert_eq!(trace_id(r#"{"@m":"no ids here"}"#), None);
        assert_eq!(trace_id(""), None);
    }

    /// **A placeholder is not an identifier.** `"trace_id":"n/a"` filtered on three characters
    /// would keep every line that happened to contain them, which is a filter that looks like it
    /// worked and is not.
    #[test]
    fn a_value_that_is_not_a_trace_is_refused() {
        assert_eq!(trace_id(r#"{"trace_id":"n/a"}"#), None);
        assert_eq!(trace_id(r#"{"trace_id":""}"#), None);
        assert_eq!(
            trace_id(r#"{"trace_id":"0000000000000000"}"#),
            None,
            "all zeroes is no trace"
        );
        assert_eq!(trace_id(r#"{"trace_id":"not-hex-at-all-really"}"#), None);
        assert_eq!(
            trace_id(r#"{"trace_id":123}"#),
            None,
            "a number is not the string this filters on"
        );
    }

    /// The key must be the key, not a substring of a longer one — `parent_trace_id` names a
    /// different thing and a filter built from it would be about a different call.
    #[test]
    fn a_key_that_merely_ends_with_the_name_is_not_it() {
        let line = r#"{"outer_trace_id":"aaaaaaaaaaaaaaaa","span_id":"bbbbbbbbbbbbbbbb"}"#;
        assert_eq!(trace_id(line), None);
    }

    /// Whitespace between the key, the colon and the value is JSON's to have.
    #[test]
    fn spacing_around_the_value_is_tolerated() {
        let line = r#"{ "trace_id" :  "d213cb0ae9cc2cc8cab31b9cd15d0854" }"#;
        assert_eq!(trace_id(line), Some("d213cb0ae9cc2cc8cab31b9cd15d0854"));
    }
}
