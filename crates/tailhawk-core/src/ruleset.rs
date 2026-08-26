//! The rules editor's model — M7, V9, `UI-DESIGN.md` §5.
//!
//! The set of highlight rules as a thing being *edited*, where [`crate::rules`] is the same set as
//! a thing being *read from a file* and [`crate::highlight::RuleSet`] is the same set as a thing
//! being *applied*. The file has stood in for the editor since the rules landed; this is what §5's
//! grid sits on. As with [`crate::palette`] and [`crate::widget`], the model is here and the window
//! is not: the shell owns the overlay's pixels, the drag and the colour swatch, and asks this what
//! to draw.
//!
//! ## A row carries its own error
//!
//! §5 is explicit that regex validity is checked **as you type, with the error shown inline —
//! never on OK**. So validity is not a step at the end; it is a property of every row, recomputed
//! whenever that row changes. A set with one bad rule is still editable and still saveable, and
//! the bad rule says why beside itself.
//!
//! That is also why the editor holds [`Spec`]s — the written form — rather than compiled
//! [`crate::highlight::Rule`]s. A rule is briefly invalid on almost every keystroke that builds
//! it, and a compiled-only model would have it wink out of existence halfway through being typed.
//!
//! ## Order is precedence, and it is the vector's
//!
//! §5's drag handle reorders, and precedence is top-down and visible. There is no separate
//! priority field to disagree with the order on screen: moving a row *is* changing precedence.
//!
//! ## Import is not the file parser
//!
//! [`crate::rules::parse`] is deliberately lenient — unknown keys ignored, malformed lines
//! skipped — which is right for a file the user wrote by hand and wrong for one that arrived from
//! somebody else. `SPEC.md` §13.1 draws the line at **executable intent versus data**: colours and
//! flags are inert and accepted, patterns are accepted but bounded, and a field that names a
//! command, a program or an action is *rejected at parse time with the offending field named*.
//! Unknown fields are rejected too, not skipped — forward compatibility is a schema version's job,
//! not tolerance's. [`import`] is that stricter reading.
//!
//! **§13.1's remote-path row is not exercised yet, and that is an accident of scope, not a
//! decision.** A rule carries no path today, so there is nothing for an imported file to point at.
//! [`Editor::bound_to`] is `UI-DESIGN.md` §5's `Apply to ▾` and is carried but not yet matched on;
//! the moment it can hold a glob, a shared set can carry `\\attacker\share\*.log` and [`import`]
//! must list those for per-path confirmation rather than accept them silently.
//!
//! ## What §5 draws that this does not model yet
//!
//! §5's per-rule role is a **three-way** — *whole line* / *match only* / *identifier*, the last
//! feeding §7's correlation — against the `whole_line` boolean here. §7 is `[v2]`, so the third
//! state has nowhere to go; when it lands, the boolean becomes an enum rather than gaining a
//! neighbour. §5's `▉ auto-colour` is likewise unrepresented:
//! [`crate::highlight::Rule::derived`] is the runtime flag for it, but a [`Spec`] with neither
//! colour is currently a compile error, so "let the palette pick" cannot yet be asked for.

use crate::highlight::{Colour, RuleSet};
use crate::rules::{self, Spec};
use crate::widget::TextField;

/// Which of a row's cells is under edit. The flags and the order are not here: they are toggles
/// and moves, not text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cell {
    Name,
    Pattern,
    Fg,
    Bg,
}

/// A row as the painter should draw it, already validated.
#[derive(Clone, Debug, PartialEq)]
pub struct Row<'a> {
    pub name: &'a str,
    pub pattern: &'a str,
    pub fg: Option<Colour>,
    pub bg: Option<Colour>,
    pub whole_line: bool,
    pub enabled: bool,
    pub case_insensitive: bool,
    /// §5's `Ab` rather than `.*` — the pattern is matched as plain text.
    pub literal: bool,
    pub selected: bool,
    /// Why this rule does not compile, if it does not.
    pub error: Option<&'a str>,
}

/// The colour a rule gets when §5's "+ Add rule" makes one, so that a new row's only complaint is
/// the pattern it does not yet have.
const NEW_RULE_FG: Colour = [1.0, 0.78, 0.36, 1.0];

/// How many rules one imported file may carry.
///
/// `SPEC.md` §13.1 bounds an individual pattern through `size_limit` / `dfa_size_limit`, but a
/// shared set can be pathological by *aggregate* — five thousand individually-cheap regexes cost
/// over a second to compile, which is the frozen window §13.1 says must never happen. The cap is
/// generous against any hand-written set and refuses the file rather than the frame.
pub const MAX_IMPORTED_RULES: usize = 512;

/// The set under edit: its name and binding, the rules, their errors, the selection, and the cell
/// under the caret.
#[derive(Debug, Default)]
pub struct Editor {
    /// §5's title — `Rules — "App production"`.
    pub name: String,
    /// §5's `Apply to ▾`: this file, a glob, or a detected format. Carried, not yet matched on,
    /// exactly as [`crate::highlight::RuleSet::bound_to`] is.
    pub bound_to: Option<String>,
    specs: Vec<Spec>,
    errors: Vec<Option<String>>,
    selected: usize,
    /// The row and cell being typed into. The **row** is recorded rather than read back from
    /// `selected` at commit time, so a selection that moves mid-edit cannot land the text in a
    /// different rule.
    editing: Option<(usize, Cell)>,
    /// The cell under edit. Public because the shell drives it with the same key handler it uses
    /// for the find field.
    pub field: TextField,
    open: bool,
    dirty: bool,
}

impl Editor {
    /// An editor over `specs`, closed, with every row already validated.
    pub fn new(name: impl Into<String>, specs: Vec<Spec>) -> Self {
        let mut editor = Self {
            name: name.into(),
            specs,
            ..Self::default()
        };
        editor.revalidate_all();
        editor
    }

    pub fn is_open(&self) -> bool {
        self.open
    }

    /// Opens on the first row. Any error left over from a cancelled edit is recomputed, so what the
    /// grid shows is what the rules actually are.
    pub fn open(&mut self) {
        self.open = true;
        self.editing = None;
        self.selected = 0;
        self.revalidate_all();
    }

    /// Closes, abandoning the cell under edit. The set itself is kept — closing the editor is not
    /// discarding the rules, and `dirty` still says whether they need writing.
    pub fn close(&mut self) {
        self.cancel_edit();
        self.open = false;
    }

    /// Whether the set has changed since it was loaded or last saved.
    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    pub fn mark_saved(&mut self) {
        self.dirty = false;
    }

    pub fn len(&self) -> usize {
        self.specs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.specs.is_empty()
    }

    pub fn selected(&self) -> usize {
        self.selected
    }

    /// Which cell is being typed into, if any.
    pub fn editing(&self) -> Option<Cell> {
        self.editing.map(|(_, cell)| cell)
    }

    pub fn specs(&self) -> &[Spec] {
        &self.specs
    }

    pub fn rows(&self) -> Vec<Row<'_>> {
        self.specs
            .iter()
            .enumerate()
            .map(|(i, spec)| Row {
                name: &spec.name,
                pattern: &spec.pattern,
                fg: spec.fg,
                bg: spec.bg,
                whole_line: spec.whole_line,
                enabled: spec.enabled,
                case_insensitive: spec.case_insensitive,
                literal: spec.literal,
                selected: i == self.selected,
                error: self.errors.get(i).and_then(|e| e.as_deref()),
            })
            .collect()
    }

    /// The set the preview should draw: every row that is enabled and compiles, in precedence
    /// order, under this editor's name and binding. A row that does not compile is simply absent,
    /// which is what its inline error already says.
    ///
    /// **This compiles every pattern, so it is for when the set changes — not for every frame.**
    /// The shell builds one [`crate::highlight::Highlighter`] from it and keeps that until the
    /// rules move again.
    pub fn compiled(&self) -> RuleSet {
        RuleSet {
            name: self.name.clone(),
            rules: self
                .specs
                .iter()
                .filter(|spec| spec.enabled)
                .filter_map(|spec| spec.compile().ok())
                .collect(),
            bound_to: self.bound_to.clone(),
        }
    }

    /// Moves the selection, clamped at both ends. An edit in flight is committed first — arrowing
    /// off a cell keeps what was typed, as every grid the user knows does.
    pub fn move_selection(&mut self, delta: i32) {
        self.commit_edit();
        if self.specs.is_empty() {
            self.selected = 0;
            return;
        }
        let last = self.specs.len() as i32 - 1;
        let to = (self.selected as i32).saturating_add(delta).clamp(0, last);
        self.selected = to as usize;
    }

    /// Selects `row`. A click that lands on the row already selected does **not** end an edit in
    /// flight — a shell that selects on mouse-down before a double-click, or during a drag of the
    /// handle, would otherwise close the cell the user is working in.
    pub fn select(&mut self, row: usize) {
        if row == self.selected || row >= self.specs.len() {
            return;
        }
        self.commit_edit();
        self.selected = row;
    }

    /// Starts editing `cell` of the selected row, seeding the field with what is there. Colours
    /// are seeded as `#rrggbb` because that is what the user can type back.
    pub fn begin_edit(&mut self, cell: Cell) {
        self.commit_edit();
        let Some(spec) = self.specs.get(self.selected) else {
            return;
        };
        let text = match cell {
            Cell::Name => spec.name.clone(),
            Cell::Pattern => spec.pattern.clone(),
            Cell::Fg => spec.fg.map(rules::hex).unwrap_or_default(),
            Cell::Bg => spec.bg.map(rules::hex).unwrap_or_default(),
        };
        self.field = TextField::new(text);
        self.field.select_all();
        self.editing = Some((self.selected, cell));
    }

    /// Writes the field back into the row the edit started on and revalidates it. A colour cell
    /// that does not read as `#rrggbb` clears that colour rather than keeping a stale one — the
    /// swatch then shows nothing, and `Spec::compile` complains if neither colour is left.
    pub fn commit_edit(&mut self) {
        let Some((row, cell)) = self.editing.take() else {
            return;
        };
        let text = self.field.text().to_owned();
        let Some(spec) = self.specs.get_mut(row) else {
            return;
        };
        match cell {
            Cell::Name => spec.name = text,
            Cell::Pattern => spec.pattern = text,
            Cell::Fg => spec.fg = rules::colour(&text),
            Cell::Bg => spec.bg = rules::colour(&text),
        }
        self.dirty = true;
        self.revalidate(row);
    }

    /// Writes `text` into `cell` of the selected row and revalidates that row.
    ///
    /// **This is the door a real control uses**, where [`begin_edit`](Self::begin_edit) and
    /// [`commit_edit`](Self::commit_edit) are the drawn editor's: that surface had one
    /// [`TextField`] standing in for whichever cell held the caret, so an edit had to be opened
    /// on a cell and closed again. A dialog gives every cell a control of its own and the control
    /// *is* the text, so there is nothing to open. §5's "checked as you type, with the error shown
    /// inline — never on OK" then falls out for free: the row is revalidated by the keystroke that
    /// wrote it.
    ///
    /// A colour cell that does not read as `#rrggbb` clears that colour rather than keeping a
    /// stale one, exactly as committing one did.
    pub fn set_cell(&mut self, cell: Cell, text: &str) {
        let row = self.selected;
        let Some(spec) = self.specs.get_mut(row) else {
            return;
        };
        match cell {
            Cell::Name => spec.name = text.to_owned(),
            Cell::Pattern => spec.pattern = text.to_owned(),
            Cell::Fg => spec.fg = rules::colour(text),
            Cell::Bg => spec.bg = rules::colour(text),
        }
        self.dirty = true;
        self.revalidate(row);
    }

    /// Abandons the edit, leaving the row as it was — including its error, which
    /// [`preview_edit`](Self::preview_edit) may have set from text that is now discarded.
    pub fn cancel_edit(&mut self) {
        if let Some((row, _)) = self.editing.take() {
            self.revalidate(row);
        }
    }

    /// Revalidates while the user is still typing, so §5's inline error tracks the keystroke
    /// rather than waiting for the cell to be left. Only the pattern can be wrong in a way worth
    /// showing mid-edit, so only that cell is previewed.
    pub fn preview_edit(&mut self) {
        let Some((row, Cell::Pattern)) = self.editing else {
            return;
        };
        let Some(spec) = self.specs.get(row) else {
            return;
        };
        let candidate = Spec {
            pattern: self.field.text().to_owned(),
            ..spec.clone()
        };
        self.errors[row] = candidate.compile().err();
    }

    pub fn toggle_enabled(&mut self) {
        self.commit_edit();
        if let Some(spec) = self.specs.get_mut(self.selected) {
            spec.enabled = !spec.enabled;
            self.dirty = true;
        }
    }

    pub fn toggle_whole_line(&mut self) {
        self.commit_edit();
        if let Some(spec) = self.specs.get_mut(self.selected) {
            spec.whole_line = !spec.whole_line;
            self.dirty = true;
        }
    }

    /// §5's `.*` / `Ab` toggle: whether the pattern is a regex or plain text. This is **not** the
    /// case-sensitivity axis — `SPEC.md` §7.1 has both, and all four combinations are askable for.
    pub fn toggle_literal(&mut self) {
        self.commit_edit();
        let row = self.selected;
        if let Some(spec) = self.specs.get_mut(row) {
            spec.literal = !spec.literal;
            self.dirty = true;
            self.revalidate(row);
        }
    }

    pub fn toggle_case_insensitive(&mut self) {
        self.commit_edit();
        let row = self.selected;
        if let Some(spec) = self.specs.get_mut(row) {
            spec.case_insensitive = !spec.case_insensitive;
            self.dirty = true;
            self.revalidate(row);
        }
    }

    /// Moves the selected row by `delta`, carrying the selection with it. This is precedence
    /// changing, and the row stays selected so a second press keeps moving the same rule.
    pub fn move_row(&mut self, delta: i32) {
        self.commit_edit();
        if self.specs.is_empty() {
            return;
        }
        let last = self.specs.len() as i32 - 1;
        let to = (self.selected as i32).saturating_add(delta).clamp(0, last) as usize;
        if to == self.selected {
            return;
        }
        let spec = self.specs.remove(self.selected);
        let error = self.errors.remove(self.selected);
        self.specs.insert(to, spec);
        self.errors.insert(to, error);
        self.selected = to;
        self.dirty = true;
    }

    /// §5's "+ Add rule": a new row below the selection, selected, with the pattern cell open so
    /// the next keystroke goes where the user is looking.
    pub fn add_rule(&mut self) {
        self.commit_edit();
        let at = if self.specs.is_empty() {
            0
        } else {
            self.selected + 1
        };
        let spec = Spec {
            name: String::new(),
            pattern: String::new(),
            fg: Some(NEW_RULE_FG),
            bg: None,
            whole_line: false,
            enabled: true,
            case_insensitive: true,
            literal: false,
        };
        let error = spec.compile().err();
        self.specs.insert(at, spec);
        self.errors.insert(at, error);
        self.selected = at;
        self.dirty = true;
        self.begin_edit(Cell::Pattern);
    }

    pub fn remove_rule(&mut self) {
        self.editing = None;
        if self.selected >= self.specs.len() {
            return;
        }
        self.specs.remove(self.selected);
        self.errors.remove(self.selected);
        self.selected = self.selected.min(self.specs.len().saturating_sub(1));
        self.dirty = true;
    }

    /// Adds `specs` below the selection — what an accepted [`import`] lands as. Kept separate from
    /// import itself so the rejection is decided before anything here changes.
    pub fn insert_all(&mut self, specs: Vec<Spec>) {
        if specs.is_empty() {
            return;
        }
        self.commit_edit();
        let at = if self.specs.is_empty() {
            0
        } else {
            self.selected + 1
        };
        for (offset, spec) in specs.into_iter().enumerate() {
            let error = spec.compile().err();
            self.specs.insert(at + offset, spec);
            self.errors.insert(at + offset, error);
        }
        self.selected = at.min(self.specs.len().saturating_sub(1));
        self.dirty = true;
    }

    /// What "Export…" writes.
    pub fn to_toml(&self) -> String {
        rules::to_toml(&self.specs)
    }

    fn revalidate(&mut self, row: usize) {
        if let Some(spec) = self.specs.get(row) {
            self.errors[row] = spec.compile().err();
        }
    }

    fn revalidate_all(&mut self) {
        self.errors = self.specs.iter().map(|s| s.compile().err()).collect();
    }
}

/// The keys a `[[rule]]` may carry. Anything else is refused rather than ignored, per §13.1.
const KNOWN: [&str; 8] = [
    "name",
    "pattern",
    "fg",
    "bg",
    "whole_line",
    "enabled",
    "case_insensitive",
    "literal",
];

/// Fields that name something to *run* rather than something to *be*.
///
/// §13.1: executable intent is never accepted from an imported file, and the refusal names the
/// field. The [`KNOWN`] allow-list would already refuse each of these as unknown — this list only
/// changes *what the user is told*, which for this class is worth the duplication: "this file
/// tried to make Tailhawk run something" is a different message from "unknown field".
const INTENT: [&str; 8] = [
    "command", "exec", "program", "action", "run", "script", "env", "register",
];

/// Reads an imported rules file strictly, per `SPEC.md` §13.1 — the stricter sibling of
/// [`crate::rules::parse`].
///
/// Returns the rules, or the first reason the file is refused. Refusing the whole file rather than
/// the offending rule is deliberate: a shared set that silently arrives one rule short is worse
/// than one that does not arrive, because the user believes they have what they were sent.
pub fn import(text: &str) -> Result<Vec<Spec>, String> {
    let text = text.trim_start_matches('\u{feff}');
    let mut out: Vec<Spec> = Vec::new();
    let mut current: Option<Spec> = None;
    for (number, raw) in text.lines().enumerate() {
        let line = raw.trim();
        let at = number + 1;
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if line.starts_with('[') {
            let table = line
                .strip_prefix("[[")
                .and_then(|rest| rest.strip_suffix("]]"))
                .map(str::trim)
                .ok_or_else(|| format!("line {at}: only [[rule]] tables belong in a rules file"))?;
            if table != "rule" {
                return Err(format!("line {at}: unknown section \"{table}\""));
            }
            if let Some(spec) = current.take() {
                out.push(named(spec, out.len()));
            }
            if out.len() >= MAX_IMPORTED_RULES {
                return Err(format!(
                    "more than {MAX_IMPORTED_RULES} rules in one file, which is more than a set \
                     can be highlighted with"
                ));
            }
            current = Some(Spec {
                enabled: true,
                case_insensitive: true,
                ..Spec::default()
            });
            continue;
        }
        let Some(spec) = current.as_mut() else {
            return Err(format!("line {at}: a value before any [[rule]]"));
        };
        let Some((key, value)) = line.split_once('=') else {
            return Err(format!("line {at}: not a key = value"));
        };
        let key = key.trim();
        let raw_value = value.trim();
        let value = rules::strip_comment(raw_value);
        if INTENT.contains(&key) {
            return Err(format!(
                "line {at}: \"{key}\" asks for something to be run, which an imported file may \
                 never do"
            ));
        }
        if !KNOWN.contains(&key) {
            return Err(format!("line {at}: unknown field \"{key}\""));
        }
        let text = rules::unquote(value);
        match key {
            "name" => spec.name = text,
            "pattern" => spec.pattern = text,
            "fg" => spec.fg = Some(hex_value(&text, raw_value, at, key)?),
            "bg" => spec.bg = Some(hex_value(&text, raw_value, at, key)?),
            "whole_line" => spec.whole_line = boolean(value, at, key)?,
            "enabled" => spec.enabled = boolean(value, at, key)?,
            "case_insensitive" => spec.case_insensitive = boolean(value, at, key)?,
            "literal" => spec.literal = boolean(value, at, key)?,
            _ => unreachable!("the key was checked against KNOWN above"),
        }
    }
    if let Some(spec) = current {
        out.push(named(spec, out.len()));
    }
    if out.is_empty() {
        return Err("no [[rule]] in the file".to_owned());
    }
    for spec in &out {
        spec.compile()
            .map_err(|e| format!("a rule will not compile: {e}"))?;
    }
    Ok(out)
}

fn named(mut spec: Spec, index: usize) -> Spec {
    if spec.name.is_empty() {
        spec.name = format!("rule {}", index + 1);
    }
    spec
}

/// A colour, or why it is not one. An unquoted `#ff0000` is the likeliest mistake in this format —
/// the `#` starts a comment, so the value arrives empty — and it gets its own message rather than
/// the baffling "\"\" is not #rrggbb".
fn hex_value(text: &str, raw: &str, at: usize, key: &str) -> Result<Colour, String> {
    if text.is_empty() && raw.starts_with('#') {
        return Err(format!(
            "line {at}: {key} must be quoted — bare {raw} reads as a comment"
        ));
    }
    rules::colour(text).ok_or_else(|| format!("line {at}: {key} \"{text}\" is not #rrggbb"))
}

fn boolean(value: &str, at: usize, key: &str) -> Result<bool, String> {
    match value {
        "true" => Ok(true),
        "false" => Ok(false),
        other => Err(format!(
            "line {at}: {key} is \"{other}\", not true or false"
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(name: &str, pattern: &str) -> Spec {
        Spec {
            name: name.to_owned(),
            pattern: pattern.to_owned(),
            fg: Some([1.0, 0.0, 0.0, 1.0]),
            bg: None,
            whole_line: false,
            enabled: true,
            case_insensitive: true,
            literal: false,
        }
    }

    fn editor(specs: Vec<Spec>) -> Editor {
        let mut e = Editor::new("test", specs);
        e.open();
        e
    }

    /// A cell written straight from a control, with no begin-and-commit around it.
    ///
    /// **Real edit controls made this the natural door.** [`Editor::begin_edit`] and
    /// [`Editor::commit_edit`] exist because the drawn editor had one [`TextField`] standing in
    /// for whichever cell held the caret; a dialog gives every cell a control of its own, and the
    /// control *is* the text. §5's "checked as you type, never on OK" then costs nothing: the row
    /// is revalidated on the keystroke that wrote it.
    #[test]
    fn a_cell_can_be_written_straight_from_a_control_and_revalidates_as_it_goes() {
        let mut set = editor(vec![spec("first", "ERROR"), spec("second", "WARN")]);
        set.select(1);

        set.set_cell(Cell::Pattern, "(unclosed");
        assert!(
            set.rows()[1].error.is_some(),
            "the row says why on the keystroke that broke it"
        );
        assert!(set.rows()[0].error.is_none(), "and its neighbour does not");
        assert!(set.is_dirty());

        set.set_cell(Cell::Pattern, "(closed)");
        assert!(set.rows()[1].error.is_none(), "and again when it is fixed");
        assert_eq!(set.specs()[1].pattern, "(closed)");

        set.set_cell(Cell::Name, "renamed");
        assert_eq!(set.specs()[1].name, "renamed");
        assert_eq!(set.specs()[0].name, "first", "only the selected row moves");

        set.set_cell(Cell::Bg, "#3a1e1e");
        assert_eq!(
            set.specs()[1].bg,
            Some([0.227_450_98, 0.117_647_06, 0.117_647_06, 1.0])
        );
        set.set_cell(Cell::Bg, "");
        assert_eq!(
            set.specs()[1].bg,
            None,
            "a colour that does not read as #rrggbb clears it, as committing one did"
        );
    }

    #[test]
    fn a_bad_pattern_is_a_property_of_its_row_not_a_refusal_to_load() {
        let set = Editor::new(
            "test",
            vec![spec("good", "ERROR"), spec("bad", "(unclosed")],
        );
        let rows = set.rows();
        assert_eq!(rows.len(), 2, "both rules are present");
        assert!(rows[0].error.is_none());
        assert!(
            rows[1].error.is_some(),
            "the broken rule says why, beside itself"
        );
        assert_eq!(
            set.compiled().rules.len(),
            1,
            "only the one that compiles reaches the preview"
        );
    }

    #[test]
    fn the_error_tracks_the_keystroke_rather_than_waiting_for_ok() {
        let mut set = editor(vec![spec("r", "ERROR")]);
        set.begin_edit(Cell::Pattern);
        set.field.set_text("(unclosed");
        set.preview_edit();
        assert!(
            set.rows()[0].error.is_some(),
            "invalid while still being typed"
        );
        set.field.set_text("(closed)");
        set.preview_edit();
        assert!(set.rows()[0].error.is_none(), "and valid again on the fix");
    }

    #[test]
    fn abandoning_a_bad_pattern_takes_its_error_with_it() {
        let mut set = editor(vec![spec("r", "ERROR")]);
        set.begin_edit(Cell::Pattern);
        set.field.set_text("(unclosed");
        set.preview_edit();
        set.cancel_edit();
        assert_eq!(set.rows()[0].pattern, "ERROR", "the rule is untouched");
        assert!(
            set.rows()[0].error.is_none(),
            "and shows no error for text that was thrown away"
        );
    }

    #[test]
    fn closing_on_a_bad_pattern_leaves_no_phantom_error_behind() {
        let mut set = editor(vec![spec("r", "ERROR")]);
        set.begin_edit(Cell::Pattern);
        set.field.set_text("(unclosed");
        set.preview_edit();
        set.close();
        set.open();
        assert!(set.rows()[0].error.is_none());
    }

    #[test]
    fn a_toggle_does_not_wipe_the_error_of_a_pattern_still_being_typed() {
        let mut set = editor(vec![spec("r", "ERROR")]);
        set.begin_edit(Cell::Pattern);
        set.field.set_text("(unclosed");
        set.preview_edit();
        set.toggle_enabled();
        assert_eq!(
            set.rows()[0].pattern,
            "(unclosed",
            "the in-flight edit was committed, not discarded"
        );
        assert!(
            set.rows()[0].error.is_some(),
            "and the error it earned survived the toggle"
        );
    }

    #[test]
    fn moving_a_row_moves_its_precedence_its_error_and_the_selection() {
        let mut set = editor(vec![spec("first", "A"), spec("second", "(bad")]);
        set.select(1);
        set.move_row(-1);
        let rows = set.rows();
        assert_eq!(rows[0].name, "second", "the moved rule is now first");
        assert!(rows[0].error.is_some(), "and its error travelled with it");
        assert!(rows[0].selected, "and it is still the selected row");
        assert_eq!(set.selected(), 0);
    }

    #[test]
    fn arrowing_off_a_cell_keeps_what_was_typed() {
        let mut set = editor(vec![spec("a", "A"), spec("b", "B")]);
        set.begin_edit(Cell::Name);
        set.field.set_text("renamed");
        set.move_selection(1);
        assert_eq!(set.rows()[0].name, "renamed");
        assert_eq!(set.editing(), None, "and the edit is over");
    }

    #[test]
    fn the_edit_lands_in_the_row_it_started_on() {
        let mut set = editor(vec![spec("a", "A"), spec("b", "B")]);
        set.begin_edit(Cell::Name);
        set.field.set_text("belongs to a");
        set.selected = 1;
        set.commit_edit();
        assert_eq!(set.rows()[0].name, "belongs to a");
        assert_eq!(
            set.rows()[1].name,
            "b",
            "and not in whatever is selected now"
        );
    }

    #[test]
    fn selecting_the_row_already_selected_does_not_end_the_edit() {
        let mut set = editor(vec![spec("a", "A")]);
        set.begin_edit(Cell::Name);
        set.select(0);
        assert_eq!(
            set.editing(),
            Some(Cell::Name),
            "a mouse-down on the current row is not a reason to stop typing"
        );
    }

    #[test]
    fn escape_leaves_the_row_as_it_was() {
        let mut set = editor(vec![spec("a", "A")]);
        set.begin_edit(Cell::Name);
        set.field.set_text("thrown away");
        set.cancel_edit();
        assert_eq!(set.rows()[0].name, "a");
    }

    #[test]
    fn a_new_rule_complains_only_about_the_pattern_it_does_not_have_yet() {
        let mut set = editor(vec![spec("a", "A")]);
        set.add_rule();
        assert_eq!(set.selected(), 1, "below the selection, and selected");
        assert_eq!(
            set.editing(),
            Some(Cell::Pattern),
            "with the pattern cell open"
        );
        let error = set.rows()[1].error.expect("a patternless rule is invalid");
        assert!(
            error.contains("no pattern"),
            "and says so, rather than complaining about colour: {error}"
        );
    }

    #[test]
    fn removing_the_last_row_leaves_the_selection_somewhere_real() {
        let mut set = editor(vec![spec("a", "A"), spec("b", "B")]);
        set.select(1);
        set.remove_rule();
        assert_eq!(set.len(), 1);
        assert_eq!(set.selected(), 0);
        set.remove_rule();
        assert!(set.is_empty());
        assert_eq!(set.selected(), 0, "and an empty set selects nothing absurd");
        set.remove_rule();
        set.move_selection(1);
        assert_eq!(set.selected(), 0, "moving about an empty set is harmless");
    }

    #[test]
    fn the_selection_clamps_at_both_ends() {
        let mut set = editor(vec![spec("a", "A"), spec("b", "B")]);
        set.move_selection(-5);
        assert_eq!(set.selected(), 0);
        set.move_selection(i32::MAX);
        assert_eq!(set.selected(), 1, "and an absurd delta does not overflow");
        set.move_selection(i32::MIN);
        assert_eq!(set.selected(), 0);
    }

    #[test]
    fn plain_text_and_case_are_two_axes_not_one() {
        let mut set = editor(vec![spec("path", r"C:\logs\app(1).log")]);
        assert!(
            set.rows()[0].error.is_some(),
            "as a regex that pattern does not compile"
        );
        set.toggle_literal();
        assert!(
            set.rows()[0].error.is_none(),
            "and as plain text it is exactly what the user typed"
        );
        assert!(set.rows()[0].literal);
        assert!(
            set.rows()[0].case_insensitive,
            "which says nothing about case — the other toggle owns that"
        );
        set.toggle_case_insensitive();
        assert!(
            set.rows()[0].literal,
            "and the two do not disturb each other"
        );
        assert!(!set.rows()[0].case_insensitive);
    }

    #[test]
    fn a_colour_that_is_not_a_colour_clears_the_swatch() {
        let mut set = editor(vec![spec("a", "A")]);
        set.begin_edit(Cell::Fg);
        set.field.set_text("chartreuse");
        set.commit_edit();
        assert_eq!(set.rows()[0].fg, None);
        assert!(
            set.rows()[0].error.is_some(),
            "with no colour left, the rule cannot compile and says so"
        );
    }

    #[test]
    fn the_compiled_set_carries_the_name_and_the_binding() {
        let mut set = Editor::new("App production", vec![spec("a", "A")]);
        set.bound_to = Some(r"C:\logs\ndc\*.log".to_owned());
        let compiled = set.compiled();
        assert_eq!(compiled.name, "App production");
        assert_eq!(compiled.bound_to.as_deref(), Some(r"C:\logs\ndc\*.log"));
    }

    #[test]
    fn a_disabled_rule_keeps_its_place_but_leaves_the_preview() {
        let mut set = editor(vec![spec("a", "A"), spec("b", "B")]);
        set.select(0);
        set.toggle_enabled();
        assert_eq!(set.len(), 2, "still in the grid");
        assert_eq!(set.compiled().rules.len(), 1, "but not in the preview");
    }

    #[test]
    fn import_accepts_a_shared_set() {
        let text = "[[rule]]\nname = \"errors\"\npattern = \"ERROR\"\nfg = \"#ff0000\"\n\
                    \n[[rule]]\npattern = \"WARN\"\nbg = \"#332200\"\nenabled = false\n";
        let specs = import(text).expect("a plain shared set is accepted");
        assert_eq!(specs.len(), 2);
        assert_eq!(specs[0].name, "errors");
        assert_eq!(specs[1].name, "rule 2", "an unnamed rule is numbered");
        assert!(!specs[1].enabled);
    }

    #[test]
    fn import_reads_a_file_notepad_saved() {
        let text = "\u{feff}[[rule]]\npattern = \"ERROR\"\nfg = \"#ff0000\"\n";
        assert!(
            import(text).is_ok(),
            "a byte-order mark is not a reason to refuse the export we just wrote"
        );
    }

    #[test]
    fn import_rejects_executable_intent_with_a_different_message_than_an_unknown_field() {
        let intent = "[[rule]]\npattern = \"E\"\nfg = \"#ff0000\"\ncommand = \"calc.exe\"\n";
        let unknown = "[[rule]]\npattern = \"E\"\nfg = \"#ff0000\"\npriority = 3\n";
        let intent = import(intent).expect_err("a command may never arrive in a file");
        let unknown = import(unknown).expect_err("§13.1: unknown fields are rejected, not skipped");
        assert!(
            intent.contains("command") && intent.contains("run"),
            "{intent}"
        );
        assert!(
            unknown.contains("priority") && unknown.contains("unknown"),
            "{unknown}"
        );
        assert_ne!(
            intent, unknown,
            "the two refusals do not say the same thing"
        );
    }

    #[test]
    fn import_wants_a_double_bracketed_rule_table() {
        let single = "[rule]\npattern = \"ERROR\"\nfg = \"#ff0000\"\n";
        assert!(
            import(single).is_err(),
            "the strict reader must not be more tolerant than the lenient one"
        );
        assert!(import("[[settings]]\npattern = \"E\"\n").is_err());
    }

    #[test]
    fn import_names_the_unquoted_colour_mistake() {
        let text = "[[rule]]\npattern = \"ERROR\"\nfg = #ff0000\n";
        let error = import(text).expect_err("a bare # is a comment");
        assert!(error.contains("quoted"), "and says so plainly: {error}");
    }

    #[test]
    fn import_rejects_a_value_with_no_rule_and_a_boolean_that_is_not_one() {
        assert!(import("pattern = \"ERROR\"\n").is_err());
        let text = "[[rule]]\npattern = \"E\"\nfg = \"#ff0000\"\nenabled = yes\n";
        let error = import(text).expect_err("\"yes\" is not a TOML boolean");
        assert!(error.contains("true or false"), "{error}");
    }

    #[test]
    fn the_lenient_file_parser_still_skips_what_import_refuses() {
        let text =
            "[[rule]]\nname = \"mine\"\npattern = \"ERROR\"\nfg = \"#ff0000\"\npriority = 3\n";
        assert_eq!(
            rules::parse(text).len(),
            1,
            "a file the user wrote by hand is still read leniently"
        );
        assert!(
            import(text).is_err(),
            "and the same text arriving from elsewhere is not"
        );
    }

    #[test]
    fn import_refuses_a_file_whose_rule_would_not_compile() {
        let text = "[[rule]]\npattern = \"(unclosed\"\nfg = \"#ff0000\"\n";
        assert!(import(text).is_err(), "a hang payload never lands");
    }

    #[test]
    fn import_refuses_a_pattern_past_the_compiler_s_size_limit() {
        let huge = "(?:.{1000}){1000}";
        let text = format!("[[rule]]\npattern = \"{huge}\"\nfg = \"#ff0000\"\n");
        let error = import(&text).expect_err(
            "§13.1's size_limit is the guarantee; pin it at the door it is claimed for",
        );
        assert!(
            error.contains("size limit"),
            "and it is the size limit that refuses it, not something incidental: {error}"
        );
    }

    #[test]
    fn import_refuses_a_set_too_large_to_highlight_with() {
        let one = "[[rule]]\npattern = \"E\"\nfg = \"#ff0000\"\n";
        let many = one.repeat(MAX_IMPORTED_RULES + 1);
        let error = import(&many).expect_err("an aggregate-pathological set is still pathological");
        assert!(error.contains(&MAX_IMPORTED_RULES.to_string()), "{error}");
        assert!(
            import(&one.repeat(MAX_IMPORTED_RULES)).is_ok(),
            "and the cap itself is allowed"
        );
    }

    #[test]
    fn imported_rules_land_below_the_selection() {
        let mut set = editor(vec![spec("a", "A"), spec("b", "B")]);
        set.select(0);
        set.insert_all(vec![spec("imported", "C")]);
        let names: Vec<_> = set.rows().iter().map(|r| r.name.to_owned()).collect();
        assert_eq!(names, ["a", "imported", "b"]);
        assert!(set.is_dirty());
    }

    #[test]
    fn importing_nothing_changes_nothing() {
        let mut set = editor(vec![spec("a", "A"), spec("b", "B")]);
        set.mark_saved();
        set.insert_all(Vec::new());
        assert_eq!(set.len(), 2);
        assert_eq!(set.selected(), 0);
        assert!(!set.is_dirty(), "and does not claim the set needs saving");
    }

    #[test]
    fn a_round_trip_through_toml_keeps_every_rule_and_both_axes() {
        let mut literal = spec("two", "B");
        literal.literal = true;
        literal.case_insensitive = false;
        let set = Editor::new("test", vec![spec("one", "A"), literal]);
        let back = rules::parse(&set.to_toml());
        assert_eq!(back.len(), 2);
        assert_eq!(back[0].name, "one");
        assert_eq!(back[1].pattern, "B");
        assert!(back[1].literal, "plain text survives the round trip");
        assert!(!back[1].case_insensitive, "and so does case");
    }
}
