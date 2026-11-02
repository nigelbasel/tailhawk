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

## 4. A filter helper: pick a column, an operator, a value; build a regex

The owner, 2026-10-28: *"It would also be good to have some sort of filter helper in the dialog to
easily pick columns and select operators and possibly values and build regexes."*

### What exists

The Filters dialog's **Edit…** (and a column header's *Filter on this column…*) opens the
composer, `dialog::show_filter_dialog` over `FilterEdit`: an Include/Exclude pair, a **scope**
combo of every column the format has (hidden ones included, the owner's rule of 2026-09-22, plus
the model's own `level`, `timestamp`, `body`, `source`, `trace`, `span`), a **Match** combo of ten
operators (contains, equals, is not, <, ≤, >, ≥, like, starts with, ends with), a value field,
*Regular expression* and *Match case* for the whole-record case, and the expression itself always
visible and editable, validated live by the parser that will run it. `compose_filter` turns the
choices into §7.2 text — `contains(status, "413")`, `level >= warning` — so nothing the helper
writes is a second language.

### What is missing, and the recommendation

- **Values to pick from.** The value field is a plain edit. Make it an editable combo whose list
  is the distinct values the chosen column carries in the rows on screen and the head sample —
  the walk the column chooser and the `where` rules already make — most frequent first, with the
  count beside each (`403 (212)`). Typing still works; picking is one click. Opened from a cell's
  context menu the value is pre-filled with the cell's.
- **One of.** An operator *is one of*, writing `status in ["403", "444"]`, with the value list
  multi-select while it is chosen. The grammar has `in`; the composer does not offer it.
- **Empty / not empty**, as *is empty* → `column = ""` and *is not empty* → `column != ""`,
  because "rows with no trace" is a daily question with no button.
- **A regex builder** on the whole-record case, shown when *Regular expression* is ticked: a
  short list of templates that write the pattern and leave it editable — *any of these words*
  (`\b(a|b|c)\b` from a comma list), *starts with* / *ends with* (`^…`, `…$`), *a status in a
  range* (`\b(4\d\d|5\d\d)\b` from `4xx, 5xx`), *an IPv4 address*, *a GUID*, *a timestamp* — and
  a **Quote this text** button that escapes the metacharacters of what was typed, which is the
  mistake a hand-written regex over a path or a URL makes most. The expression line shows the
  result and the parser's verdict as it does now.
- **What not to do.** No wizard with pages; no second expression language; no hiding the text.
  The helper only ever writes the expression the reader could have typed.

**Order of work.** Values and *one of* first (the daily win, and the value walk exists); empty /
not empty with them; the regex templates second. Small to medium.

---

## 5. Filters that OR as well as AND

The owner, 2026-10-29: *"currently they are all anded together, it would be good to be able to
select either and or or."*

### What exists

`SPEC.md` §7.2, after TextAnalysisTool: *include chips AND together, then exclude chips are
subtracted; order of chips is display order and does not affect the result.* `Chips::keeps` is
that rule in eleven lines. Within **one** filter the grammar already has both words — `status =
500 or status = 502`, `level >= Warning and not contains(message, "retry")`, `and` binding
tighter than `or`, parentheses allowed — so an OR of two things has always been writable as one
chip. What is missing is an OR *between* filters in the list, which is how the dialog invites a
reader to think: one row per thing to look for.

### Three shapes, and which one

- **A per-list choice, as Explorer's search and Outlook's rules put it: rows must match *all*
  include filters, or *any* include filter.** Two radios under the *All filters* heading. Excludes
  go on subtracting whichever is chosen, because "hide retries" and "hide heartbeats" are never
  meant as "hide rows that are both". Stored beside the chips in the file's `[[file]]` table as
  `filters = "any"`, absent meaning *all*, so every settings file written so far reads as it did.
  The status pane says which is in force (`· any include filter` beside the count), because
  a list that reads the same under both rules and means something different is the one thing
  this must not become.
- **A per-filter conjunction** — each row after the first carries *and* / *or* with the row above,
  as a spreadsheet's advanced filter does. Expressive, but `a and b or c` then needs a precedence
  rule the list cannot show, and the dialog's own promise that order does not change the result
  stops being true. Not recommended until the first shape proves too small.
- **Groups** — sub-lists that AND inside and OR between. The most general, a tree control's worth
  of UI, and the case it serves (`(a and b) or (c and d)`) is one chip in the grammar today.

**The recommendation is the first.** It is one decision in the model (`Chips::keeps` reads a
`Combine::All | Any` beside the rows, pure, tested on the truth table including *unknown*
answers, which under *any* must not count as a match), one key in the settings file, two radios
in the dialog, and a word in the status pane. A reader who needs the second shape writes that
one row with `and`/`or` in the composer's expression line, which the grammar accepts and the
status line validates. Small; a morning.

**What not to do.** No implicit change of rule when a second include is added; no *or* that
spans an include and an exclude; no rewriting the stored chips into one expression, which would
lose the per-row enable and the per-row edit the dialog was asked for.

## 6. From a stack frame to its source

The owner, 2026-11-02: *"another featuer that would be useful is to be able to navigate to source
code from a stack trace is the file info is preset. This could be in a TailHawk popup, or possibly
nabigating to a text editor."*

### What exists

Nothing that reads a frame. The detail window (`detailwin.rs`) already shows a record with its
continuation lines under it, which is where a stack trace ends up, and the grid's context menu
already carries record-scoped commands (*Follow this trace*, Ctrl+T). The corpus has the case:
the migration log's Node frames name local absolute paths with line and column, such as
`(C:\dev\…\harness\tsc-merge.js:80:8)`. Those would open today with no configuration at all.

### Three problems, and they are separate

**Finding the frames.** This is a pure function over a record's text, `frames_of(text) ->
Vec<Frame>`. Each `Frame` carries the span it came from, the path as written, a line, and a column
when there is one. The shapes worth recognising are few, and each is a fixed pattern:

| Runtime | Shape |
|---|---|
| .NET | `at Ns.Type.Method() in C:\src\Foo.cs:line 42` (also `/src/Foo.cs:line 42` from a Linux container) |
| Node / JS | `at fn (C:\x\a.js:80:8)` and the bare `at C:\x\a.js:80:8` |
| Python | `File "C:\x\a.py", line 10, in fn` |
| Rust, Go, gcc, clang | `src/main.rs:12:5` |
| MSBuild, csc, tsc | `C:\x\Foo.cs(42,7): error CS1002` |
| Java, Kotlin | `at com.acme.Foo.bar(Foo.java:42)`: a file name and a package, no path |

The table is the test: one row per shape, with the near misses beside each. A timestamp `12:05:33`
is not a line and column, a URL's `:443` is not a line, and `line 42` without a path is not a frame.

**Resolving the path.** This is the real work. A path in a log says where the code was built, not
where it is on this machine: `/src/Nurtur.X/Foo.cs` from a container, `D:\a\1\s\…` from a build
agent, a bare `Foo.java` from Java. Resolution is a second pure function, given the frame, the
reader's **source roots** and a probe for existence:

1. If the path is absolute and exists, it is used as written. The migration log takes this path.
2. If a **mapping** applies, the prefix is rewritten. For example, `/src/` becomes
   `C:\dev\tfsgit\Nurtur.Direct.Communications\` and `D:\a\1\s\` becomes a checkout. Mappings are
   prefix pairs in settings, tried longest first.
3. Otherwise the **longest tail that exists** under one of the source roots is used: for
   `/app/Api/Controllers/Foo.cs`, try `Api/Controllers/Foo.cs` under each root, then
   `Controllers/Foo.cs`. A tail of the bare file name alone is used only if it is unique under the
   roots, so a common name never opens a guess. A Java package plus file name is tried as a tail.
4. Otherwise the frame is shown as **unresolved**, with what was tried, and the reader is offered
   *Map this path…*. That dialog proposes the mapping from the frame and the folder they pick, so
   every later frame from that build resolves too.

The roots and mappings belong to the reader's settings, not a file's, because one checkout serves
every log from that service. A rule set can carry them later if per-file roots prove necessary.

**Opening it.** This is where the owner offered two answers, and the recommendation is both, in
this order:

- **A *Source* page in the detail window.** It is a list view of the record's frames: function,
  file, line, and whether each one resolved. Selecting a frame shows the file around that line
  below the list in a read-only edit control, with the frame's line selected and scrolled to the
  middle. This needs no editor and no configuration, it appears the moment a record with frames is
  on the caret, and it is built from controls the window already uses. Standard controls throughout.
- **Open in editor**, on Enter or double-click in that list, and as *Go to source* on the grid's
  context menu (first resolved frame), F12: Ctrl+G is already *Go to line*, and F12 is *Go to
  Definition* in both Visual Studio and VS Code. The editor is a command line in settings with
  `{path}`, `{line}` and `{column}` in it, and a preset is filled in for what is found installed:
  - VS Code: `code --goto "{path}:{line}:{column}"`
  - Rider: `rider64.exe --line {line} "{path}"`
  - Notepad++: `notepad++.exe -n{line} "{path}"`
  - Visual Studio: `devenv.exe /Edit "{path}"`, which cannot take a line, so it opens at the top
    and the page says so.

  With nothing configured, Enter opens the file with its default application, at the top.

### What a log may not do

Log text is untrusted: a Loki tail is production output, and a frame is a string someone else
wrote. So:

- **Nothing in a frame is ever run.** The editor command comes from settings. The path is
  substituted as one argument and started with `CreateProcessW`, never through `cmd.exe`, and never
  with a `ShellExecute` verb on the raw string.
- **Only a file that exists on a local fixed drive is opened or read.** UNC paths (`\\host\share`),
  device paths (`\\?\`, `\\.\`) and URLs are refused at resolution. Merely *probing* a UNC path for
  existence makes Windows authenticate to that host, which leaks the reader's NTLM hash to whoever
  wrote the log. So they are refused before any probe, not after.
- **The source page reads a bounded window** around the line, and only text files. A frame
  pointing at a 2 GB file or a binary shows the frame, not the file.

### Knowingly left out

- **The checkout may not be the build.** A line number from yesterday's release can point at a
  different line in today's checkout. The page shows the file's own path and modification time, so
  a mismatch can be seen. Fetching the exact revision (`git show <sha>:<path>` when the log carries
  a commit) is the natural follow-up, not the first step.
- **Symbol servers, PDBs and source links** are a debugger's job, not a viewer's.

### Size, and the order inside it

- **Increment 1, about a day.** `frames_of` and the resolver, pure, with the shape table and the
  near misses as tests. This is a new core module, so the `CLEANROOM.md` row comes first. Then the
  Source page and *Open in editor* with the presets. That opens the migration log's frames and any
  local run's .NET frames with no configuration.
- **Increment 2, about half a day.** Mappings, the tail search over source roots, and *Map this
  path…*. That is what makes container and build-agent traces from Loki open.

Two answers from the owner change details, not the shape: which editor he uses day to day, which
sets the default preset; and where the traces he wants this for come from, a local run or Loki.

## Order between the three

Block selection first: it is a day's shell work on a finished core, it needs no file format, and
the owner has columns on his screen today. Rule sets and attribute rules second, together, in the
three steps above, with the model landing before any UI so the resolution is tested before it is
drawn. The owner chose this order on 2026-10-28.
