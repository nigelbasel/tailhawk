# Two ideas from the owner, 2026-10-28: block selection, and named rule sets per file

The owner, 2026-10-28: *"Many editors support column selection using alt + left mouse drag. This
would be useful as well when we have columns."* And: *"regarding the highlighting features. It
would be good to be able to have multiple named sets and be able to apply each one to a particular
file or file type."* He asked for analysis and recommendations that are as usable as possible.

Both ideas are already in the contract. `SPEC.md` §7.1 says a rule set *"can be bound to a file
glob or a detected format, so opening a matching file applies it automatically. No incumbent does
this."* `UI-DESIGN.md` §12 lists *rectangular selection (Alt+drag)* among the selection gestures,
and `SPEC.md` §4 says how it is to be modelled. What follows is what exists, what is missing, and
the shape each should take so that it is the obvious thing to use rather than a feature to learn.

---

## 1. Alt + left-drag: a rectangular selection

### What exists

- **The core is done.** `tailhawk-core::selection` has `SelectionMode::Block`, `Selection::block`,
  `set_mode`, and a `row_span` that returns the same cell band on every row in range; `byte_range`
  resolves the band per row through the cell model, so a band over a mix of ASCII and CJK covers
  the same *columns* on every row and the right *bytes* on each. Clamping to rows shorter than the
  band is tested. The module's own doc names Alt+drag as the gesture.
- **The painter needs nothing.** `Document::row_selection` flattens a row's span to a cell range and
  `paint.rs` fills it; a block paints as a rectangle today, the moment one exists.
- **Under columns, a band is a column band.** With a layout in force the selection's cells are
  cells of the *presented* row — the padded, column-aligned text — so a rectangle drawn over the
  `status` column is the `status` column on every row, which is exactly the case the owner named.

### What is missing

- **The shell never makes one.** Mouse-down reads `Shift` and the button from `wparam`; `Alt` is
  not read anywhere in mouse handling and `Selection::block` has no caller.
- **Copy joins block rows with nothing.** `copy_text` appends `\n` only where a row's span says
  `line_break`, and a block's spans never do (the core's doc says the caller supplies the newline).
  A block copied today would be one long line.
- **Copy as TSV ignores the selection's columns.** `copy_tsv` copies whole rows with every field.
- **Nothing selects a column from its header.** The header's gestures are sort, resize and reorder.

### Recommendation

**Increment 1 — the gesture (small).** Decide the mode at button-down, as VS Code and Sublime do,
in a pure function the shell only calls:

```
Selecting::from_click(shift, alt, triple) ->
    Alt            => Block start
    Shift+Alt      => extend the current selection as a block
    Shift          => extend (stream, as today)
    triple click   => line
    otherwise      => stream start
```

Dragging keeps the mode the drag started in; `set_focus` already keeps it. `Alt` alone must go on
focusing the menu bar (`UI-DESIGN.md` §2.2). Windows handles this for us — `DefWindowProc` cancels
the Alt-release menu activation when a mouse button is pressed while `Alt` is down — but the repo
has nothing that asserts it, so the first screen run of this increment checks exactly that: Alt+drag
selects, and the menu bar does not light up on release. If some Windows build disagrees, the
answer is to swallow `WM_SYSCOMMAND`/`SC_KEYMENU` with `lparam == 0` while a drag is in progress,
not to move the gesture.

Copy, in the same increment: a block copies its band from each row, rows joined with `\n`, trailing
padding trimmed per row (the band's padding is presentation, not the file's bytes). That is the
"lift a column of timestamps out of a log" case the core was written for. `Ctrl+Shift+C` (TSV) on a
block copies only the layout columns the band touches, still tab-separated, still with the header
row — so a band over `status` and `method` gives a two-column TSV. A block on a document with no
layout copies the band as text, which is what the user drew.

**Increment 2 — a column by its header (small, after 1).** `Ctrl+click` on a column title selects
that column as a block from the first row to the last. The selection is O(1) in rows, so selecting
a column of a 10 GB file costs nothing; copying it is §10.2's confirm path, as any huge copy is.
`Ctrl+Shift+click` extends to a band of adjacent columns. This is the gesture that makes columns
pay off, and it is the one no incumbent viewer has.

**Later, if wanted.** `Shift+Alt+arrows` to grow a block from the keyboard (VS Code's binding);
the status bar saying `12 rows × 8 cols` while a block is live.

**What not to do.** No "column mode" toggle that sticks; no second selection type for whole
columns. One `Selection`, two modes, as the core already is.

---

## 2. Named highlight rule sets, each applied to a file or a file type

### What exists

- **The model is half there.** `highlight::RuleSet { name, rules, bound_to: Option<String> }` with
  the comment *"§7.1's 'bound to a file glob or a detected format'. Carried, not yet matched on."*
  The editor (`ruleset::Editor`) carries `bound_to` the same way and never sets or draws it;
  `UI-DESIGN.md` §5's mock already has the button: **`Apply to ▾`** *binds the set to this file, a
  glob, or a detected format.* HANDOFF records the `Apply to ▾`, Import and Export as unwired.
- **One file, one flat list.** `tailhawk.rules.toml` is a list of `[[rule]]` tables with no sets;
  `rules::load` concatenates every tier's rules, exe-adjacent first; the editor edits the personal
  tier only and every tab is rebuilt from the same list. The owner's own file today is six
  whole-line rules (`info`, `warning`, `error`, `debug`, `-> 400`, `-> 444`) that apply to
  everything he opens.
- **Per-file state already has a home.** `settings::FileState` is keyed by the exact path (or the
  `loki://source?apps=…` URL) and holds chips, the collapse, bookmarks, labels, column widths and
  order. Nothing in it is matched by type.
- **"File type" already has two meanings in the product**, and both are usable: the format wizard's
  definitions carry a **glob** (`claims(path)`: a bare pattern matches the file name, a rooted one
  the full path), and every document has a **detected or chosen format** (`detection.accepted`),
  which for a Loki tail is keyed on the app. A Serilog file in any folder, or every `sb-edge` tail,
  is a *format*; `C:\logs\jobdispatcher\*.log` is a *glob*.
- **Labels sit above everything** (`Ctrl+Shift+1…9`, per file, replayed after every rebuild) and
  should stay that way: they are the reader's pen, not a rule set.

### Recommendation

**The file format: sets inside the same file, and the old shape still valid.**

```toml
# Rules outside any set are the Default set: everywhere, beneath the bound sets.
[[rule]]
name = "Error"
pattern = "error"
bg = "#ff0000"
whole_line = true

[[set]]
name = "Starberry edge"
apply_to = ["loki://starberry*", "format:Starberry access log"]

[[set.rule]]
name = "bad request"
pattern = "-> 400"
bg = "#ff00ff"
whole_line = true

[[set]]
name = "Job dispatcher"
apply_to = ["C:\\logs\\jobdispatcher\\*.log"]

[[set.rule]]
…
```

`apply_to` is a list, because one set often fits several things, and each entry is one of three
kinds, spelled so that it reads without a key: a **path or URL** (exact, case-folded, as
`FileState` matches today), a **glob** (anything with `*` or `?`, with the wizard's own `claims`
semantics so the two features cannot disagree), or **`format:<name>`** (the detected or chosen
format's name). A set with no `apply_to` is unbound: it applies nowhere until chosen by hand, and
it is how a set is drafted before it is bound. The file is rewritten whole on save, as the settings
file is, so hand edits survive.

**Resolution, per document, in a pure function.** `rules::for_document(path, format_name, &sets)`
returns the stack: every bound set whose `apply_to` matches, in file order, then the Default set,
then — above all of them — the file's labels. Curated sets from the exe-adjacent tier go in front
of personal ones, exactly as the flat rules do now, and a curated set may be bound too. Within a
set the existing first-claim-wins rule stands. Two matching sets simply stack; there is no
"most specific wins" to explain, because the editor shows the stack and the order is the file's.

**The editor: one switcher, one button, no new dialog.** `Ctrl+K` opens the editor on the set that
is *active for this document* — the first matching bound set, or Default if none. Across the top,
a set switcher (the status bar's `3 rules ▾` chip already reserved for exactly this in
`UI-DESIGN.md` §2.1) lists every set with a mark on those applying here, plus **New set…** and
**Rename**. The `Apply to ▾` menu offers *This file*, *Files like this* (a glob built from the
path, editable), *This format* (`format:<name>`, shown with the detected name so the reader knows
what they are binding to) and *Nowhere (by hand)*. Adding a rule while a document is open adds it
to the active set, which is the set the reader is looking at the effect of; moving a rule between
sets is a `Move to ▾` on the rule row. Preview continues to rebuild every tab, so the reader sees a
set take effect on the matching tabs and not on the others before saving.

**Applying a set by hand.** Choosing an unbound set, or any set, from the switcher applies it to
*this document* for the session, on top of what is bound. If the reader wants that to stick, the
switcher's **Always apply here** is `Apply to ▾ → This file` under a plainer name. That is the
"apply each one to a particular file" ask in one click, without a second mechanism.

**Migration.** A `tailhawk.rules.toml` with only `[[rule]]` tables reads as a Default set and keeps
working unchanged; the editor shows it as *Default (everywhere)*. The owner's six rules stay in
force on every file until he moves one.

**Order of work.** (1) The model: sets and bindings in `rules.rs`, the three binding kinds and
`for_document`, all core, all tested. (2) The document: `rebuild_highlighter_with` gathers the
stack instead of the flat list; the status chip shows the active set's name and count. (3) The
editor: switcher, `Apply to ▾`, `Move to ▾`, and Import/Export of a set (the file shape above is
the export). Medium effort in total; (1) and (2) are the value, (3) is the polish.

**What not to do.** No separate "rule sets" dialog; no binding by file extension alone (a `.log`
is every log — the format is the type that means something); no per-set colours or themes.

---

---

## 3. Rules that test an attribute, not the text

`C:\dev\tfsgit\TAILHAWK-REQUEST-attribute-highlight-rules-2026-10-28.md`, from the
create-liquidocean-firewall workstream, 2026-10-28: the Starberry gateway classifies every refused
request in two structured-metadata attributes, `reason` (`probe`, `payload`, `scanner`,
`proxy-abuse`, `method`, `multipart`, `unknown-host`, `badrequest`) and `status`, and they want to
tint attacks, policy blocks and raw-IP noise by *those*, not by a text match on the body that
happens to carry the same words today. Their workaround (`-> 444 probe` as a literal) works and is
fragile. They ask for a rule with a match target: the line text as now, or an attribute with
`=`, `one-of` or `contains`.

**The recommendation is to reuse the language the product already has, not to add a second one.**
`SPEC.md` §7.2's filter expressions already name attributes (`p2s_role = "edge"`, which is the chip
the owner's own settings carry), already have `in` and `contains`, and already answer *unknown* for
a field the record does not produce — which is exactly the request's "if absent, the rule simply
does not match". So a rule's match becomes one of two things:

```toml
[[set.rule]]
name = "attack"
where = 'reason in ["probe", "payload", "scanner", "proxy-abuse"]'
bg = "#7a1010"
whole_line = true

[[set.rule]]
name = "oversize"
where = "status = 413"
bg = "#8a5a00"
whole_line = true
```

`pattern` is today's text rule, unchanged; `where` is a §7.2 predicate evaluated on the parsed
record; a rule has one or the other. A `where` rule is whole-line by nature — there is no span in
the text to colour — and the editor says so by greying the toggle rather than by a third kind of
rule. List order stays the precedence, so an attack rule above a noise rule wins as the request
asks. The editor offers *Match: text | where*, and for `where` the attribute keys come from the same
per-tail list the column chooser already has.

**The one real change is where the record is.** The highlighter is fed the row's *text*
(`Document::colour_beneath` → `Highlighter::line(text, …)`), and a predicate needs the parsed
`Record`. The columns feature already parses the visible rows for their presentation, so the record
is at hand in the same frame; the highlighter's entry point gains it, the memo keys stay as they
are (the text is still what a span is over), and a `where` rule is one attribute lookup and compare
per row — cheaper than a regex over a 32 KB line, as the request notes.

This is step (1) of the rule-sets work above, done at the same time as the set model, because both
change the rules file's schema once.

---

## Order between the three

Block selection first: it is a day's shell work on a finished core, it needs no file format, and
the owner has columns on his screen today. Rule sets and attribute rules second, together, in the
three steps above, with the model landing before any UI so the resolution is tested before it is
drawn. The owner chose this order on 2026-10-28.
