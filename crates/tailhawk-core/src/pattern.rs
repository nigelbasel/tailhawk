//! Rolling-set recognition — `SPEC.md` §5.5b, the naming half.
//!
//! §5.5b: "A source may be a *rolling set*: a directory plus a pattern, presented as one continuous
//! logical log." This module answers the two questions that come before any of that: given the file
//! the user pointed at and the names sitting beside it, **which of them are members**, and **what
//! order are they in**.
//!
//! It touches no file contents and opens nothing. Its whole input is one anchor name and a list of
//! sibling names, which is what makes it exhaustively testable — the shapes below are all reachable
//! from a `&[&str]` in a unit test, and none of them needs a directory on disk.
//!
//! ## Ordering is the trap, and §5.5b says so
//!
//! > "Date and ascending-sequence sets sort newest-*last*; log4net's rename-based backups sort
//! > newest-*first* (`app.log` is current, `app.log.1` is the previous one). Getting this backwards
//! > silently presents history in reverse."
//!
//! Silently is the word that matters. A viewer that reverses history does not crash, does not warn,
//! and looks exactly like a viewer that did it right; the user finds out when they draw the wrong
//! conclusion from it. So the two families are recognised by *different rules* rather than by one
//! rule with a guess attached, and [`RollingSet::describe`] exists because §5.5b requires the
//! inference be "shown in the UI for confirmation rather than silently assumed".
//!
//! The newest-first family has **two spellings**, not one. log4net appends to the whole name —
//! `app.log.1` — and Serilog's `preserveLogFilename` inserts before the extension —
//! `Api_001.log`. §5.5b names only the first, and the second went unrecognised until it turned up
//! in the owner's own logs: opening `Nurtur.Contact.Api.log` reported one file with two
//! generations sitting beside it.
//!
//! ## Numbers compare as numbers
//!
//! **Ours, not §5.5b's.** §5.5b's own example is `log_<seq:000>` — zero-padded, where byte order and
//! numeric order agree. An *unpadded* sequence does not: `log_1`, `log_10`, `log_2` is the order a
//! string sort gives, and it is precisely the silent reversal §5.5b calls the trap, arrived at from
//! the other direction. [`natural_key`] therefore splits a name into literal and numeric runs and
//! compares numeric runs by value. A date is just a wide number and falls out of the same rule.
//!
//! ## What is deliberately not recognised
//!
//! - **`by-mtime`.** §5.5b names it as "the fallback for unrecognised shapes". Inference never
//!   produces it: an unrecognised shape means we do not know which siblings are members either, and
//!   ordering files we cannot identify by a timestamp a copy or a restore rewrites is a worse answer
//!   than admitting the set is one file. [`Order::ByMtime`] exists for a caller to *choose*.
//! - **NLog's unpadded mid-name sequence** (`log.1.txt`), for the reason in [`FIELD_MIN_DIGITS`].
//! - **A sibling directory.** §5.5b's NLog row is `archive/log.2026-07-28.txt` — members in a
//!   subfolder. Everything here works within one directory listing.

use std::cmp::Ordering;

/// Smallest unpadded numeric field that will be believed as a rolling field.
///
/// **Ours, and it is a trade.** `service2.log` beside `service3.log` is two services, not two
/// generations of one; nothing in the names distinguishes that case from a rolling set except how
/// wide the number is. Serilog's defaults are 8 digits (a date) and 3 zero-padded (a sequence), so
/// requiring three digits *or* a leading zero admits both and rejects the single-digit name.
///
/// The cost is stated rather than hidden: **NLog's `log.1.txt` is not recognised as a set**, and
/// neither is any other one-digit unpadded sequence. That is recorded in `HANDOFF.md`. A false
/// negative leaves the user with the single file they asked for; a false positive splices an
/// unrelated service into their scrollback, and §5.5b's whole concern is history that reads wrong
/// without saying so.
pub const FIELD_MIN_DIGITS: usize = 3;

/// How the members of a set sort from oldest to newest.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Order {
    /// The varying field counts **up** with time. Serilog's date and sequence sets, NLog's dates.
    Ascending,
    /// The varying field counts **down** with time — log4net's `app.log.1`, `app.log.2`, where the
    /// live file carries no number at all and a *bigger* number is *older*.
    Descending,
    /// §5.5b's fallback for shapes nothing here recognises. Never inferred; see the module note.
    ByMtime,
}

/// The rule that decides membership, and the evidence for the order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Shape {
    /// One file. Either there were no siblings, or the anchor's name has nothing varying in it.
    Single,
    /// Every member shares the anchor's literal text and differs only in its numeric fields.
    ///
    /// `log-<20260728>.txt`, `log_<001>.txt`, `log-<20260728>_<001>.txt`. Newest sorts last.
    Varying {
        /// The anchor's literal runs, in order, with the numeric runs removed. Two names are members
        /// of the same set exactly when these agree.
        literals: Vec<String>,
    },
    /// The live file is the bare name and the history is `<name>.1`, `<name>.2`, …
    ///
    /// log4net's default. **Newest sorts first**, which is the reversal §5.5b warns about, and the
    /// reason this is a separate shape rather than [`Varying`](Shape::Varying) with a flag.
    Backup {
        /// The live member's whole filename — `app.log`.
        live: String,
    },
}

/// A rolling set as inferred from one directory listing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RollingSet {
    shape: Shape,
    order: Order,
    /// Members **oldest first**, which is the order a scrollback reads in.
    members: Vec<String>,
}

impl RollingSet {
    /// Infers the set containing `anchor` from the names beside it.
    ///
    /// `siblings` is every name in the directory, the anchor included or not — it is filtered either
    /// way. Nothing is opened and no metadata is read, so this is a decision about names only.
    ///
    /// The three recognisers are tried in a fixed order, and the order carries meaning. Both backup
    /// shapes go first, because their anchor has no numeric field at all and [`Shape::Varying`]
    /// therefore cannot see the set that is plainly there: `app.log` beside `app.log.1`, or
    /// `Api.log` beside `Api_001.log`. `Varying` then takes everything whose *live* name itself
    /// counts — a date or a sequence in the anchor — which is why
    /// [`infer_preserved`](Self::infer_preserved) declines an anchor that has one.
    pub fn infer(anchor: &str, siblings: &[String]) -> Self {
        if let Some(set) = Self::infer_backup(anchor, siblings) {
            return set;
        }
        if let Some(set) = Self::infer_preserved(anchor, siblings) {
            return set;
        }
        if let Some(set) = Self::infer_varying(anchor, siblings) {
            return set;
        }
        Self {
            shape: Shape::Single,
            order: Order::Ascending,
            members: vec![anchor.to_string()],
        }
    }

    /// log4net: the anchor is the live file and `<anchor>.<n>` are its backups, newest first.
    fn infer_backup(anchor: &str, siblings: &[String]) -> Option<Self> {
        let mut backups: Vec<(u128, String)> = Vec::new();
        for name in siblings {
            let Some(tail) = name.strip_prefix(anchor) else {
                continue;
            };
            let Some(digits) = tail.strip_prefix('.') else {
                continue;
            };
            if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
                continue;
            }
            // A backup index is small by construction — log4net's default maxSizeRollBackups is 1.
            // Anything that will not fit a u128 is not one, and is left out rather than truncated.
            if let Ok(n) = digits.parse::<u128>() {
                backups.push((n, name.clone()));
            }
        }
        if backups.is_empty() {
            return None;
        }
        // Oldest first is the *largest* index first. This one line is the whole of §5.5b's trap.
        backups.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| b.1.cmp(&a.1)));
        let mut members: Vec<String> = backups.into_iter().map(|(_, name)| name).collect();
        members.push(anchor.to_string());
        Some(Self {
            shape: Shape::Backup {
                live: anchor.to_string(),
            },
            order: Order::Descending,
            members,
        })
    }

    /// Serilog's `preserveLogFilename`: the anchor is the live file and `<stem>_<nnn><ext>` are its
    /// backups, newest first — [`Shape::Backup`]'s family, spelled differently.
    ///
    /// The number goes **before the extension** rather than after the whole name, which is the only
    /// thing separating this from [`infer_backup`](Self::infer_backup). It is a separate recogniser
    /// rather than a looser one there because the two spellings must not be allowed to match each
    /// other's sets by accident: `app.log.1` is not `app_1.log`.
    ///
    /// **A bigger number is older here too**, which was read off a real directory rather than
    /// assumed — see the test. The sequence counts up as files roll, so the highest is the one
    /// that rolled longest ago, and the live file carries no number at all.
    fn infer_preserved(anchor: &str, siblings: &[String]) -> Option<Self> {
        // The anchor splits at its *last* dot, so `Nurtur.Contact.Api.log` has the stem
        // `Nurtur.Contact.Api` and the extension `.log`. A name with no dot has no extension and
        // cannot carry a number before one.
        let (stem, ext) = anchor.rsplit_once('.')?;
        if stem.is_empty() {
            return None;
        }
        // **The live name does not vary — that is what the option is called.** Without this,
        // `log-20260728.txt` claims `log-20260728_001.txt` as a backup and the date set it really
        // belongs to never gets looked at. An anchor carrying a believable field of its own is
        // [`Shape::Varying`]'s business, and this recogniser runs first.
        if tokenise(anchor).iter().any(believable) {
            return None;
        }
        let mut backups: Vec<(u128, String)> = Vec::new();
        for name in siblings {
            let Some(rest) = name.strip_prefix(stem) else {
                continue;
            };
            let Some(rest) = rest.strip_prefix('_') else {
                continue;
            };
            let Some(digits) = rest.strip_suffix(&format!(".{ext}")) else {
                continue;
            };
            // Padded, for [`FIELD_MIN_DIGITS`]'s reason: `service_1.log` beside `service_2.log` is
            // two services as readily as two generations of one, and nothing in the names says
            // which. Serilog pads to three.
            if digits.len() < FIELD_MIN_DIGITS || !digits.bytes().all(|b| b.is_ascii_digit()) {
                continue;
            }
            if let Ok(n) = digits.parse::<u128>() {
                backups.push((n, name.clone()));
            }
        }
        if backups.is_empty() {
            return None;
        }
        // Oldest first is the largest index first — §5.5b's trap, in the other spelling.
        backups.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| b.1.cmp(&a.1)));
        let mut members: Vec<String> = backups.into_iter().map(|(_, name)| name).collect();
        members.push(anchor.to_string());
        Some(Self {
            shape: Shape::Backup {
                live: anchor.to_string(),
            },
            order: Order::Descending,
            members,
        })
    }

    /// Serilog and NLog: the members differ only in their numeric fields, and count up.
    fn infer_varying(anchor: &str, siblings: &[String]) -> Option<Self> {
        let anchor_tokens = tokenise(anchor);
        let fields: Vec<&Token> = anchor_tokens
            .iter()
            .filter(|t| matches!(t, Token::Number { .. }))
            .collect();
        if fields.is_empty() {
            return None;
        }
        // Every field has to be wide enough to be believed, not just one of them — `service2.log`
        // must not be admitted on the strength of a date somewhere else in the name.
        if !fields.iter().all(|t| believable(t)) {
            return None;
        }
        let literals = literals_of(&anchor_tokens);
        let mut members: Vec<String> = siblings
            .iter()
            .filter(|name| literals_of(&tokenise(name)) == literals)
            .cloned()
            .collect();
        if !members.iter().any(|m| m == anchor) {
            members.push(anchor.to_string());
        }
        if members.len() < 2 {
            return None;
        }
        members.sort_by(|a, b| natural_cmp(a, b));
        Some(Self {
            shape: Shape::Varying { literals },
            order: Order::Ascending,
            members,
        })
    }

    pub fn shape(&self) -> &Shape {
        &self.shape
    }

    pub fn order(&self) -> Order {
        self.order
    }

    /// The members, **oldest first**. Scrollback reads in this direction.
    pub fn members(&self) -> &[String] {
        &self.members
    }

    /// The member being written to now — the one to follow.
    ///
    /// Last for an ascending set, and last here for a descending one too, because
    /// [`members`](Self::members) is always oldest-first: the sort, not the caller, carries the
    /// direction. `the_newest_member_is_the_one_being_written_to` pins both families to that.
    pub fn newest(&self) -> &str {
        self.members
            .last()
            .map(String::as_str)
            .expect("a set always has at least its anchor")
    }

    /// One line the UI can show for confirmation.
    ///
    /// §5.5b: the inferred order is "**shown in the UI for confirmation** rather than silently
    /// assumed". This is that string. It names the direction in the terms a user can check against
    /// the file list in front of them — which file is oldest — rather than in the terms the code
    /// used, because "descending" is exactly the word a reader would nod at without testing.
    pub fn describe(&self) -> String {
        match &self.shape {
            Shape::Single => "1 file — no rolling set found beside it".to_string(),
            Shape::Varying { .. } => format!(
                "{} files, numbered upwards — oldest is {}, newest is {}",
                self.members.len(),
                self.members[0],
                self.newest()
            ),
            Shape::Backup { live } => format!(
                "{} files, log4net backups — {} is live, oldest is {}",
                self.members.len(),
                live,
                self.members[0]
            ),
        }
    }
}

/// A run of one filename: either literal text or a number.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Token {
    Text(String),
    Number { value: u128, digits: usize },
}

/// Is this field wide enough to be a rolling field rather than part of a name?
///
/// See [`FIELD_MIN_DIGITS`]. A leading zero is accepted at any width because padding is a statement
/// of intent — nobody writes `log_007` for the seventh of something that is not counted.
fn believable(token: &Token) -> bool {
    match token {
        Token::Text(_) => false,
        Token::Number { value, digits } => {
            *digits >= FIELD_MIN_DIGITS || (*digits > 1 && *value < 10u128.pow(*digits as u32 - 1))
        }
    }
}

/// Splits a name into alternating literal and numeric runs.
///
/// A numeric run that overflows `u128` — 39 digits or more — is kept as literal text. It is not a
/// counter, and silently wrapping it would make two different names compare equal.
fn tokenise(name: &str) -> Vec<Token> {
    let bytes = name.as_bytes();
    let mut tokens = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i].is_ascii_digit() {
            let start = i;
            while i < bytes.len() && bytes[i].is_ascii_digit() {
                i += 1;
            }
            let run = &name[start..i];
            match run.parse::<u128>() {
                Ok(value) => tokens.push(Token::Number {
                    value,
                    digits: run.len(),
                }),
                Err(_) => push_text(&mut tokens, run),
            }
        } else {
            let start = i;
            while i < bytes.len() && !bytes[i].is_ascii_digit() {
                i += 1;
            }
            push_text(&mut tokens, &name[start..i]);
        }
    }
    tokens
}

/// Appends literal text, merging with a preceding literal so the run sequence stays canonical.
///
/// Without the merge, an overflowing digit run would leave `Text, Text` where an ordinary name has
/// one `Text`, and two names that differ only in that would stop being members of the same set.
fn push_text(tokens: &mut Vec<Token>, text: &str) {
    match tokens.last_mut() {
        Some(Token::Text(existing)) => existing.push_str(text),
        _ => tokens.push(Token::Text(text.to_string())),
    }
}

/// The literal skeleton: the text runs, in order, with the numbers taken out.
///
/// **This is the membership test.** Two names belong to the same set exactly when their skeletons
/// are equal, which is why the position of the numbers matters and their values do not:
/// `log-20260728.txt` and `log-20260729.txt` share `["log-", ".txt"]`, and `error-20260728.txt`
/// does not.
///
/// A name whose numbers sit in *different places* produces a different skeleton and is correctly
/// excluded — `log-20260728_001.txt` is `["log-", "_", ".txt"]`, three runs against two.
fn literals_of(tokens: &[Token]) -> Vec<String> {
    let mut out = Vec::new();
    let mut pending = String::new();
    let mut seen_number = false;
    for token in tokens {
        match token {
            Token::Text(text) => pending.push_str(text),
            Token::Number { .. } => {
                out.push(std::mem::take(&mut pending));
                seen_number = true;
            }
        }
    }
    if seen_number || !pending.is_empty() {
        out.push(pending);
    }
    out
}

/// Compares two names with numeric runs compared by value. See the module note.
pub fn natural_cmp(a: &str, b: &str) -> Ordering {
    let (left, right) = (natural_key(a), natural_key(b));
    for (l, r) in left.iter().zip(right.iter()) {
        let ordering = match (l, r) {
            (Token::Text(l), Token::Text(r)) => l.cmp(r),
            (Token::Number { value: l, .. }, Token::Number { value: r, .. }) => l.cmp(r),
            // A number sorts before text at the same position, arbitrarily but consistently. Two
            // members of one set never reach this arm — their skeletons agree by construction.
            (Token::Number { .. }, Token::Text(_)) => Ordering::Less,
            (Token::Text(_), Token::Number { .. }) => Ordering::Greater,
        };
        if ordering != Ordering::Equal {
            return ordering;
        }
    }
    left.len()
        .cmp(&right.len())
        // Equal by value and by length can still be two different names — `log_01` and `log_1`.
        // Falling back to the raw bytes keeps the order total, so a sort is deterministic.
        .then_with(|| a.cmp(b))
}

/// The token sequence [`natural_cmp`] compares. Exposed for tests, not for callers.
fn natural_key(name: &str) -> Vec<Token> {
    tokenise(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    /// §5.5b row 1: `log-20260728.txt`, `log-20260729.txt` → one set, newest last.
    #[test]
    fn a_serilog_date_set_is_recognised_and_reads_oldest_first() {
        let set = RollingSet::infer(
            "log-20260729.txt",
            &names(&[
                "log-20260729.txt",
                "log-20260727.txt",
                "log-20260728.txt",
                "readme.md",
            ]),
        );
        assert_eq!(set.order(), Order::Ascending);
        assert_eq!(
            set.members(),
            ["log-20260727.txt", "log-20260728.txt", "log-20260729.txt"]
        );
        assert_eq!(set.newest(), "log-20260729.txt");
    }

    /// §5.5b row 2, and the reason numbers do not compare as text: Serilog pads, but if it did not,
    /// a byte sort would read 1, 10, 2 and present the history in an order nothing announces.
    #[test]
    fn an_unpadded_sequence_still_counts_upwards() {
        let set = RollingSet::infer(
            "log_001.txt",
            &names(&["log_1.txt", "log_10.txt", "log_2.txt", "log_001.txt"]),
        );
        // `log_1` and `log_001` are the same number written two ways, so the numeric comparison has
        // nothing left to say and the raw bytes break the tie. Which of the two wins is arbitrary;
        // that the tie is broken at all is not, or the sort has no defined result.
        assert_eq!(
            set.members(),
            ["log_001.txt", "log_1.txt", "log_2.txt", "log_10.txt"]
        );
        let mut byte_sorted = set.members().to_vec();
        byte_sorted.sort();
        assert_ne!(
            byte_sorted,
            set.members(),
            "if a byte sort agreed, this fixture would not be testing anything"
        );
    }

    /// §5.5b row 3: both options together, two fields in one name.
    #[test]
    fn a_date_and_sequence_set_orders_by_date_then_sequence() {
        let set = RollingSet::infer(
            "log-20260728_001.txt",
            &names(&[
                "log-20260729_001.txt",
                "log-20260728_002.txt",
                "log-20260728_001.txt",
            ]),
        );
        assert_eq!(
            set.members(),
            [
                "log-20260728_001.txt",
                "log-20260728_002.txt",
                "log-20260729_001.txt"
            ]
        );
    }

    /// §5.5b: "**descending** — `.1` is *newer* than `.2`". Getting this backwards is the failure the
    /// spec calls out by name, so it is asserted in full rather than by `newest()` alone.
    #[test]
    fn log4net_backups_read_oldest_first_which_is_highest_numbered_first() {
        let set = RollingSet::infer(
            "app.log",
            &names(&["app.log", "app.log.1", "app.log.2", "app.log.3"]),
        );
        assert_eq!(set.order(), Order::Descending);
        assert_eq!(
            set.members(),
            ["app.log.3", "app.log.2", "app.log.1", "app.log"]
        );
        assert_eq!(set.newest(), "app.log");
    }

    /// Serilog's `preserveLogFilename` shape is log4net's family wearing different punctuation.
    ///
    /// **Found by dogfooding, 2026-08-27.** The owner's own APIs write through
    /// `Serilog.Sinks.PersistentFile` with `preserveLogFilename = true`, which keeps the live file
    /// at its fixed name and numbers the rolled ones *before the extension* — `Api.log` beside
    /// `Api_001.log`, `Api_002.log`. Opening the live one reported **1 file** with two generations
    /// sitting next to it, because the anchor carries no numeric field for [`Shape::Varying`] to
    /// see and no `.N` suffix for [`Shape::Backup`]'s spelling to match.
    ///
    /// The direction was read off the disk rather than assumed, and it is §5.5b's trap either way:
    /// `Api.log` was last written at 21:00, `Api_001.log` at 10:06 the same day, `Api_002.log` five
    /// weeks earlier. **A bigger number is older**, exactly as in log4net, so this is the same
    /// shape and not a third one.
    #[test]
    fn serilogs_preserved_filename_backups_are_log4nets_family_in_other_punctuation() {
        let set = RollingSet::infer(
            "Nurtur.Contact.Api.log",
            &names(&[
                "Nurtur.Contact.Api.log",
                "Nurtur.Contact.Api_001.log",
                "Nurtur.Contact.Api_002.log",
            ]),
        );
        assert_eq!(set.order(), Order::Descending);
        assert_eq!(
            set.members(),
            [
                "Nurtur.Contact.Api_002.log",
                "Nurtur.Contact.Api_001.log",
                "Nurtur.Contact.Api.log"
            ],
            "oldest first, which is highest-numbered first"
        );
        assert_eq!(set.newest(), "Nurtur.Contact.Api.log");

        // An unpadded neighbour is not a generation. `service_1.log` beside `service_2.log` is two
        // services as readily as two rolls, and [`FIELD_MIN_DIGITS`] is where that line is drawn.
        let two = RollingSet::infer("service.log", &names(&["service.log", "service_1.log"]));
        assert_eq!(two.members(), ["service.log"], "one file, not a set");

        // Nor is a file that merely starts the same way.
        let apart = RollingSet::infer(
            "api.log",
            &names(&[
                "api.log",
                "api_001.log",
                "api_001.log.bak",
                "apiary_001.log",
            ]),
        );
        assert_eq!(apart.members(), ["api_001.log", "api.log"]);
    }

    /// The two families disagree about which end is new, and `members()` hides that from callers by
    /// always being oldest-first. If it did not, every consumer would need the order field.
    #[test]
    fn the_newest_member_is_the_one_being_written_to() {
        let ascending = RollingSet::infer(
            "log_002.txt",
            &names(&["log_001.txt", "log_002.txt", "log_003.txt"]),
        );
        assert_eq!(ascending.newest(), "log_003.txt");
        let descending = RollingSet::infer("app.log", &names(&["app.log", "app.log.1"]));
        assert_eq!(descending.newest(), "app.log");
    }

    /// `service2.log` beside `service3.log` is two services. See [`FIELD_MIN_DIGITS`].
    #[test]
    fn a_single_digit_in_a_name_does_not_make_a_rolling_set() {
        let set = RollingSet::infer(
            "service2.log",
            &names(&["service2.log", "service3.log", "service4.log"]),
        );
        assert_eq!(*set.shape(), Shape::Single);
        assert_eq!(set.members(), ["service2.log"]);
    }

    /// …but padding is a statement of intent, at any width.
    #[test]
    fn a_padded_field_is_believed_however_narrow_it_is() {
        let set = RollingSet::infer("log_01.txt", &names(&["log_01.txt", "log_02.txt"]));
        assert!(matches!(set.shape(), Shape::Varying { .. }));
        assert_eq!(set.members(), ["log_01.txt", "log_02.txt"]);
    }

    #[test]
    fn a_name_whose_numbers_sit_elsewhere_is_not_a_member() {
        let set = RollingSet::infer(
            "log-20260728.txt",
            &names(&[
                "log-20260728.txt",
                "log-20260729.txt",
                "log-20260728_001.txt",
                "err-20260728.txt",
                "log-20260728.txt.bak",
            ]),
        );
        assert_eq!(set.members(), ["log-20260728.txt", "log-20260729.txt"]);
    }

    /// A file on its own is a set of one rather than an error or an empty list, so every caller can
    /// treat "a source" and "a set" as the same thing.
    #[test]
    fn a_file_with_no_siblings_is_a_set_of_one() {
        let set = RollingSet::infer("app.log", &names(&["app.log", "notes.txt"]));
        assert_eq!(*set.shape(), Shape::Single);
        assert_eq!(set.members(), ["app.log"]);
        assert_eq!(set.newest(), "app.log");
    }

    /// A dated file with nobody to roll with is still one file. Inferring a set from a shape alone
    /// would put a lone `log-20260728.txt` into a "set" that a later sibling silently joins.
    #[test]
    fn a_lone_dated_file_is_not_a_set_until_a_second_one_exists() {
        let set = RollingSet::infer("log-20260728.txt", &names(&["log-20260728.txt"]));
        assert_eq!(*set.shape(), Shape::Single);
    }

    /// The anchor is a member even when the caller passed a listing taken before it existed, or
    /// filtered it out. Losing the file the user actually opened would be the worst possible bug.
    #[test]
    fn the_anchor_is_a_member_even_if_the_listing_omits_it() {
        let set = RollingSet::infer(
            "log_003.txt",
            &names(&["log_001.txt", "log_002.txt", "log_004.txt"]),
        );
        assert!(set.members().contains(&"log_003.txt".to_string()));
        assert_eq!(
            set.members(),
            ["log_001.txt", "log_002.txt", "log_003.txt", "log_004.txt"]
        );
    }

    /// Backup detection wins the tie deliberately: `app.log` has no field of its own, so the varying
    /// recogniser cannot see the set, and trying it first would return `Single` for log4net's
    /// default layout.
    #[test]
    fn a_backup_set_is_found_even_though_the_live_file_has_no_number() {
        let set = RollingSet::infer("app.log", &names(&["app.log", "app.log.1"]));
        assert_eq!(
            *set.shape(),
            Shape::Backup {
                live: "app.log".to_string()
            }
        );
    }

    /// `app.log.1.gz` is a retention artefact, not a backup index. §5.5b makes archived members part
    /// of the set, but decompressing them is §4.3 and v2 — admitting the name now would put a file
    /// we cannot read into the scrollback.
    #[test]
    fn a_compressed_backup_is_not_admitted_by_its_name_alone() {
        let set = RollingSet::infer(
            "app.log",
            &names(&["app.log", "app.log.1", "app.log.2.gz", "app.log.old"]),
        );
        assert_eq!(set.members(), ["app.log.1", "app.log"]);
    }

    /// §5.5b calls order "shown in the UI for confirmation". A description that says "descending"
    /// invites a nod; one that names a file invites a check.
    #[test]
    fn the_description_names_a_file_the_user_can_check() {
        let backup = RollingSet::infer("app.log", &names(&["app.log", "app.log.1", "app.log.2"]));
        assert_eq!(
            backup.describe(),
            "3 files, log4net backups — app.log is live, oldest is app.log.2"
        );
        let dated = RollingSet::infer(
            "log-20260729.txt",
            &names(&["log-20260728.txt", "log-20260729.txt"]),
        );
        assert_eq!(
            dated.describe(),
            "2 files, numbered upwards — oldest is log-20260728.txt, newest is log-20260729.txt"
        );
    }

    /// A digit run too wide for `u128` is not a counter. Wrapping it would make two different names
    /// compare equal, and a set is a set of *distinct* files.
    #[test]
    fn an_absurdly_long_digit_run_is_treated_as_text() {
        let wide = "9".repeat(40);
        let a = format!("log-{wide}0.txt");
        let b = format!("log-{wide}1.txt");
        assert_eq!(natural_cmp(&a, &b), Ordering::Less);
        assert_ne!(a, b);
        let set = RollingSet::infer(&a, &names(&[&a, &b]));
        assert_eq!(*set.shape(), Shape::Single);
    }

    /// Sorting demands a total order — antisymmetric and never zero for distinct names — or
    /// `sort_by` is free to produce anything. Two names equal by value and width are the case that
    /// nearly is not.
    #[test]
    fn the_natural_order_is_total_over_confusable_names() {
        let fixtures = [
            "log_1.txt",
            "log_01.txt",
            "log_001.txt",
            "log_0001.txt",
            "log.txt",
            "log",
            "1",
            "01",
            "",
            "log-20260728_001.txt",
            "log-20260728_1.txt",
        ];
        for a in fixtures {
            assert_eq!(natural_cmp(a, a), Ordering::Equal, "{a} against itself");
            for b in fixtures {
                let forward = natural_cmp(a, b);
                assert_eq!(
                    forward,
                    natural_cmp(b, a).reverse(),
                    "{a} vs {b} is not antisymmetric"
                );
                if a != b {
                    assert_ne!(forward, Ordering::Equal, "{a} and {b} compare equal");
                }
            }
        }
    }

    /// Membership is transitive or the set is not a set: if the anchor admits X, then anchoring on
    /// X must admit the anchor. An asymmetric rule would make the scrollback depend on which file
    /// the user happened to double-click.
    #[test]
    fn membership_does_not_depend_on_which_member_was_opened() {
        let listing = names(&[
            "log-20260727.txt",
            "log-20260728.txt",
            "log-20260729.txt",
            "other.txt",
        ]);
        let from_newest = RollingSet::infer("log-20260729.txt", &listing);
        for anchor in ["log-20260727.txt", "log-20260728.txt"] {
            let set = RollingSet::infer(anchor, &listing);
            assert_eq!(set.members(), from_newest.members(), "anchored on {anchor}");
            assert_eq!(set.order(), from_newest.order());
        }
    }

    /// The same, for the family where the members do not all look alike. Anchoring on a *backup*
    /// finds nothing — `app.log.1` has no `app.log.1.N` beside it — and a set of one is the honest
    /// answer, not `app.log`'s set with the wrong live file.
    #[test]
    fn anchoring_on_a_backup_does_not_invent_a_live_file() {
        let listing = names(&["app.log", "app.log.1", "app.log.2"]);
        let set = RollingSet::infer("app.log.1", &listing);
        assert_eq!(*set.shape(), Shape::Single);
        assert_eq!(set.members(), ["app.log.1"]);
    }
}
