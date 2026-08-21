# Tailhawk — UI Design

**Version:** 0.1 (draft for adversarial review)
**Date:** 2026-07-28
**Companion documents:** [`RESEARCH.md`](RESEARCH.md) · [`SPEC.md`](SPEC.md) · [`PLAN.md`](PLAN.md)

---

> ## ⚠ Phase markers — read this first
>
> **`SPEC.md` §15 is authoritative on phasing. This document is not.** An earlier draft carried no
> phase markers at all and presented the merged view, trace correlation and per-field filter actions
> as ambient product, with `Ctrl+M` sitting in the v1 keyboard map. A UI implementer works from *this*
> document — so v1 would have been built with v2 in it, and the discovery would have landed at M7,
> week 62, the milestone that gates release.
>
> Every section below is now tagged:
>
> | Tag | Meaning |
> |---|---|
> | **`[v1]`** | Ships in v1. The v1 window is §2, and it looks exactly as drawn there. |
> | **`[v2]`** | Designed now so v1 does not foreclose it. **Not built in v1.** |
> | **`[v3]`** | Sketched only. |

## 1. Design principles `[v1]`

**Tailhawk — watch your logs like a hawk.**

The identity leads with the idiom, because that is what makes the name decode correctly on first
contact. Everything else follows from seven principles:

1. **The log is the interface.** Chrome is thin, quiet and gets out of the way. At any moment the
   overwhelming majority of pixels are log content. No ribbon, no toolbar of 30 icons, no MDI child
   frames.
2. **Never lie, never silently drop.** A line that could not be parsed is *shown* as unparsed. A guess
   about format or encoding is *visible* and *one click to change*. A truncated line says it was
   truncated. A stale network view says it is stale.
3. **Nothing modal on the hot path.** Filtering, searching, changing a highlight rule, switching
   format — all happen inline with live preview. Modal dialogs are for genuinely separate tasks
   (settings, export).
4. **Progressive, never blocking.** Content paints before the index is built; the scrollbar refines
   itself; filters stream results. The window is interactive within 150 ms regardless of file size.
5. **Density without noise.** This is a professional tool used all day. Compact rows, real columns,
   generous information density — but a restrained palette so that *user* highlight colours are the
   loudest thing on screen.
6. **The four pillars of usability are requirements, not aspirations** — see §1.2.
7. **The UI is MVVM, so that the interesting half can be unit-tested** — see §1.3.

### 1.1 What "modern, not MDI" means concretely

| Rejected | Adopted |
|---|---|
| MDI child windows with their own title bars | Flat tab strip, drag-to-reorder, drag-out-to-split |
| Grey 3D-bevelled Win32 chrome | Flat surfaces, Windows 11 Mica on chrome, opaque grid |
| Toolbar of ambiguous 16×16 icons | **A menu bar and a toolbar**, primary; the command palette (`Ctrl+K`) as an extra |
| Modal config dialogs per feature | Inline editors with live preview over real data |
| Settings spread across 6 tabbed property sheets | One searchable settings surface |
| Fixed-function status bar | Status bar as a row of **live, clickable chips** |

**On menus and toolbars — a decision reversed, 2026-08-19.** An earlier draft of this table rejected
both, and made the command palette the only route to a command with no dedicated control. That was
wrong, and it is recorded here rather than quietly fixed.

**The ordering is now explicit: menus and the toolbar are the primary interface. The palette is an
additional option, not the answer.** A palette serves the user who already knows what a command is
called. A menu serves the user asking *"what can this program do?"* — and on Windows that question
is answered along the top of the window. It is the convention, users are entitled to it, and a
keyboard shortcut is not a substitute for it. Every feature must be reachable **by mouse, through
the menu or the toolbar**, and the palette must not be the only way to reach anything.

The rejection had a measurable cost. Building palette-first meant the mouse was consistently the
part deferred: by V9 the app had no `WM_RBUTTONDOWN` handler at all, six of §2.1's seven "live,
clickable" status chips were not clickable, and §6.2's boundary handles — specified as a *drag* —
were keyboard-only. A principle that reads as restraint was in practice a licence to skip the half
of the interface most users reach for first.

**What stays true** is the *register*, not the omission. No ribbon; no toolbar of ambiguous,
unlabelled 16×16 icons. The menu bar and toolbar are **drawn by the app** in its own flat
Windows 11 style — not a classic Win32 `HMENU`, whose grey 3D chrome the row above still rejects.
Toolbar buttons carry text, or an icon with a text label beside it, never an icon alone. Every menu
item names its accelerator, so the menu teaches the keyboard instead of competing with it.

---

### 1.2 The four pillars, and how each is checked

*(Owner's requirement, 2026-08-20. Stated here because the first five principles are about what the
window looks like, and these are about whether anyone can use it.)*

A principle that cannot fail a check is a slogan. Each pillar below has a rule that can be pointed
at a build and answered yes or no.

**Discoverability — can a user find it without being told?**

- Every command in the register appears in **at least one menu**. This is a test, not an intention:
  `Menu::ids()` and `Command::LISTED` exist to be walked against each other, and a command reachable
  only from the palette fails it.
- Everything reachable by keyboard is reachable by **mouse**, through the menu, the toolbar, a
  context menu or a chip. The palette is an accelerator for people who already know the name.
- A right-click offers what applies **to the thing under the pointer**. §6.5's "right-click a
  representative line" is the pattern: the object carries its own verbs.
- §10's neglected states say what to do next, not merely what went wrong.

**Memorability — having found it once, can they find it again?**

- One command has **one name**, everywhere it appears — menu, toolbar, palette, context menu. The
  register is the single source, so the same string is drawn in all four.
- Position is stable. A menu's items do not reorder by frequency, and a command that is unavailable
  is **greyed in place** rather than removed (§1.1) — a menu whose shape changes cannot be learned.

**Learnability — does using it teach the faster way?**

- Every menu item **names its accelerator** beside it. The menu is how the keyboard is learned; it
  does not compete with it.
- Modal surfaces carry a legend of their own keys along the bottom, as §5's rules editor and §6.2's
  wizard do.
- The toolbar is labelled. An icon with no text teaches nothing the second time either.

**Usability — how much work is the common thing?**

- The daily actions — follow, find, filter, change format, change rules — are **one action from the
  main window**, not two levels into a menu.
- Nothing modal on the hot path (principle 3), and every inline editor previews over real data.
- Mouse and keyboard are each **complete paths**. Neither is a second-class route that runs out
  halfway, which is the failure §1.1 records: by V9 the app had no `WM_RBUTTONDOWN` handler at all.

### 1.3 The UI is MVVM `[v1]`

*(Owner's framing, 2026-08-21. Stated as a principle because the pattern was already being followed
and had no name, and an unnamed convention is one that erodes.)*

**The view-model holds everything worth testing; the view is the part that could be replaced.** In a
portable codebase that is what MVVM buys: the executable half sits in a class with no platform in
it, and only the drawing changes per platform. Tailhawk is built that way already —

| Role | Here |
|---|---|
| Model | `tailhawk-core` — the index, the decoder, the filter, the sort, the menu tree |
| **View-model** | `MenuFrame`, `RulesOverlay`, `WizardOverlay`, `FormatRow`, `HeaderColumn` — each one the surface **as one frame should draw it**, carrying no `HWND` and no device |
| Mapping | `menu_frame_of`, `rules_overlay_of`, `wizard_overlay_of`, `format_menu_of` — pure, and unit-tested |
| View | The Win32 shell: it draws a view-model and routes input back, and ideally decides nothing |

**The check, which is quicker to apply than the principle: could this run with no window?** If the
answer is no, and the code is not literally a `SetWindowPos` call, the decision is in the wrong half.

This is not theory. Three defects found in a single session all had the same shape — a decision that
stayed in the view, where nothing could reach it:

- the `message` column's title silently vanished from the header;
- every column divider was drawn two cells away from the drag boundary it advertises;
- a click on a **disabled** menu item ran whatever the menu had highlighted, so with no document
  open the greyed `Close Tab` would have opened a file dialog.

None of the three had a test, because none *could* have one where it sat. Each became testable the
moment the decision moved out, into `Document::header_columns`, `menubar::chosen_by_click` and
`menubar::hit_at`. That is the whole argument for the pattern, and it is why the rule is stated
here rather than left as a preference.

---

## 2. Main window `[v1]`

**This is the v1 window as it actually is at v1** — no merge tab, no trace popover, no per-field
filter action.

```
┌───────────────────────────────────────────────────────────────────────────────────────────┐
│ 🦅  api.log  ×  │ jobdispatcher.log ● × │ nginx-access.log × │  +          ─  □  ✕        │  ← Mica title bar,
├───────────────────────────────────────────────────────────────────────────────────────────┤     tabs inline
│ File   Edit   View   Format   Rules   Help                                                │  ← menu bar §2.2
├───────────────────────────────────────────────────────────────────────────────────────────┤
│ Open  Find  Filter  Follow  Collapse  Detail  Rules  Format  Export                       │  ← toolbar §2.3
├───────────────────────────────────────────────────────────────────────────────────────────┤
│ ⌕ Search…              │ ⊕ level >= Warning ⊗ /healthz/ ⊕ Ab timeout  + Filter │ ⚙ Rules  │  ← command bar:
├──┬────────────┬─────────────────────┬─────┬──────────┬───────────────────────────────────┤     filter CHIPS
│  │ #          │ Timestamp           │ Lvl │ Logger   │ Message                           │  ← column header
├──┼────────────┼─────────────────────┼─────┼──────────┼───────────────────────────────────┤
│  │ 4,182,993  │ 09:14:02.117        │ INF │ Api.Cont │ Started HTTP GET /api/contacts    │▲ │
│  │ 4,182,994  │ 09:14:02.álig131    │ INF │ Api.Cont │ Query returned 412 rows in 88ms   │█ │  ← scrollbar with
│▌ │ 4,182,995  │ 09:14:03.884        │ ERR │ Api.Disp │ Failed to dispatch job 41982      │█ │    match density
│  │            │                     │     │          │ ▸ System.InvalidOperationExcep…   │  │    marks
│  │ 4,183,001  │ 09:14:04.002        │ WRN │ Api.Cont │ Retry 1/3 for job 41982           │▼ │
├──┴────────────┴─────────────────────┴─────┴──────────┴───────────────────────────────────┤
│ ⬤ Following  │ Serilog (file) 99.2% ▾ │ UTF-8 ▾ │ 4,183,001 lines · 8.2 GB │ 3 rules ▾   │  ← status chips
└───────────────────────────────────────────────────────────────────────────────────────────┘
   ▲ gutter: bookmark, unparsed stripe, new-data marker
```

### 2.1 Regions

**Title bar and tab strip.** Tabs live in the title bar to reclaim vertical space. Mica backdrop
(Windows 11 22621+, probed at runtime). A tab shows a **●** when new content has arrived while
unfocused. Middle-click closes; drag reorders; drag out of the strip creates a split pane; drag onto
another window merges. `+` opens a file, a file set, or a watched folder.

**Menu bar and toolbar.** Two rows under the tab strip — see §2.2 and §2.3. Together they are the
**primary** interface per §1.1: every feature is reachable by mouse from one of them, and the palette
is an accelerator over the top rather than the only door.

**Command bar.** Search on the left; a **filter chip row** on the right; rules button.

**The filter surface is a row of chips, not a text field.** An earlier draft drew a single box, which
cannot express three of the owner's five daily-use features — include filters, exclude filters, and
multiple composing text filters. Each chip is one independent predicate object per the grammar in
`SPEC.md` §7.2:

```
  ⊕ level >= Warning     include chip, comparison predicate
  ⊗ /healthz/            exclude chip, regex predicate
  ⊕ Ab timeout           include chip, plain-text predicate ("Ab" = literal, ".*" = regex)
  + Filter               add a new chip
```

- **Click the ⊕/⊗ glyph** to flip a chip between include and exclude — the single most common edit.
- **Click the chip body** to edit it inline as text; it expands into a field with live validation.
- **Middle-click or ✕** removes it. Chips are draggable to reorder for readability only; order does
  not affect the result.
- A chip naming a column the current format lacks renders in a **warning state** naming the missing
  field, per §7.2's unknown-field rule — it does not silently match nothing.
- Chips overflow into a `⋯ +3` affordance rather than wrapping the command bar.

Search remains a single field and accepts plain text or regex via a `.*` toggle. No modal find dialog.

**Column header.** Real, resizable, reorderable, hideable columns — not monospace padding. Sortable
headers show the sort affordance **only when sorting is eligible** (§11.4 of SPEC: filtered set
≤ 2M rows); otherwise the affordance is absent rather than present-and-broken.

**Gutter.** Narrow, left of the line-number column. Carries: bookmark marks, a coloured stripe for
unparsed lines, the new-data separator, and severity as a **redundant non-colour channel** (a shape,
so severity is legible in High Contrast and to colour-blind users).

**Grid.** Virtualised, `u64` line-index scrolling. Rows are logical records; the `#` column shows
**physical line numbers** so they match what every other tool reports. A record with continuation lines
shows a **▸** chevron and expands in place.

**Continuations are collapsed by default**, and this is a correctness requirement, not a preference —
uniform row height is what makes the `u64` scroll model O(1). MEL Simple logs are multi-line by
construction, so without collapse-by-default the *base* state of the view would be variable-height.
Expanded rows are held in a capped side table (`SPEC.md` §6.4).

**There is no word wrap in v1** (`SPEC.md` §6.4) — long lines scroll horizontally instead.

**Scrollbars.** The vertical trough carries **density marks** for search hits and filter matches — a
whole-file overview for free, and a genuinely liked feature in LogFusion Pro. A **horizontal
scrollbar** appears when content exceeds the viewport width; its extent comes from the per-block
`max_byte_len` bound (`SPEC.md` §3.3) and refines as blocks are laid out, so the thumb may shrink
slightly during a long scroll — it never jitters per vertical scroll. **Shift+wheel** scrolls
horizontally.

**Status chips.** Every chip is live and clickable:

| Chip | Shows | Click |
|---|---|---|
| `⬤ Following` | Follow state | Toggle follow / pin-to-file |
| `Serilog (file) 99.2% ▾` | Detected format + parse health | Format dropdown / define new |
| `UTF-8 ▾` | Detected encoding | Encoding override |
| `4,183,001 lines · 8.2 GB` | Position and size | Go to line |
| `3 rules ▾` | Active highlight rule set | Rule set switcher |
| `⚠ Network mode` | *(conditional)* UNC source | Explains the polling interval |
| `⚠ Settings not saved` | *(conditional)* stateless mode | Explains why |

**Parse health lives in the format chip** — `Serilog (file) 99.2% ▾` — with the full breakdown
(`99.2% parsed · 812 continuation · 14 unparsed`) on hover. This single number is what lets a user
judge whether the detected format is right, and it is far more useful than a confidence percentage
they cannot check.

### 2.2 The menu bar `[v1]`

Six menus. Every item names its accelerator, so the menu **teaches** the keyboard rather than
competing with it; an item with no accelerator has none rather than a blank column.

```
 File            Edit                  View               Format            Rules             Help
 ─────────────── ───────────────────── ────────────────── ───────────────── ───────────────── ──────────────
 Open…    Ctrl+O Copy           Ctrl+C Follow tail      F Format…           Highlight rules…  Command palette
 Open set…  ^⇧O  Copy as TSV   Ctrl+⇧C Collapse    Ctrl+E   ✓ Serilog 99%     …………… Ctrl+H    ………… Ctrl+K
 ─────────────── ───────────────────── Invisibles  Ctrl+I   Plain text      Open rules file   Keyboard map
 Export view…    Find…          Ctrl+F Record detail ^⏎    ───────────────── Reload rules     About Tailhawk
 Keep saving…    Find next          F3 ────────────────── Define from a line…
 ─────────────── Find previous    ⇧+F3 Split pane   Ctrl+\ Import layout…
 Close tab Ctrl+W Go to line…   Ctrl+G Next tab   Ctrl+Tab ─────────────────
 Exit      Alt+F4 ───────────────────── Previous  ^⇧Tab   Encoding        ▸
                 Filter: include Ctrl+L Back        Alt+←
                 Filter: exclude ^⇧L    Forward     Alt+→
                 Clear filters         ──────────────────
                 ───────────────────── Theme: dark / light
                 Bookmark      Ctrl+D
                 Bookmarks     Ctrl+⇧D
```

- **Custom-drawn**, per §1.1 — not a Win32 `HMENU`. That means `Alt` activation, mnemonics, arrow
  navigation, type-ahead and the UIA tree are this application's own work, and §13 holds them to the
  same standard as any other surface.
- `Alt` alone focuses the bar; `Alt+F` and friends open a menu directly; `←`/`→` move between menus
  with one open, `↑`/`↓` within one, `Enter` chooses, `Esc` closes one level.
- A menu item that cannot act right now is **shown disabled, not hidden** — a menu whose shape
  changes is a menu that cannot be learned.
- Items that reflect state carry a **✓** (Follow tail, Collapse, Invisibles, the current format, the
  current theme).
- **One popup implementation** serves the menu bar, §6.1's format chip, the status chips and §2.4's
  context menus. A second would drift.

### 2.3 The toolbar `[v1]`

One row, **text labels** — or an icon with a label beside it, never an icon alone (§1.1).

```
 Open   Find   Filter   Follow   Collapse   Detail   Rules   Format   Export
```

Buttons are the commands reached most often in a session, not a mirror of the menu. A button whose
command is unavailable is disabled in place. Toggles (Follow, Collapse, Detail) draw pressed when
their state is on, so the toolbar reads as a status display as well as a control.

The toolbar is **hideable** — `View ▸ Toolbar` — and its state is remembered per §12.4, because a
user who works from the keyboard should be able to buy the row back.

### 2.4 Context menus `[v1]`

Right-click is a first-class route, not a shortcut for people who know it is there. Every surface
that has actions has a context menu, and each is a strict subset of what the menu bar offers:

| Right-click on | Offers |
|---|---|
| A grid line | Copy, Copy as TSV, **Define format from this line…** (`SPEC.md` §6.5), Filter to / Filter out this text, Bookmark, Record detail |
| The column header | Sort ascending / descending, Top N, Hide column, Reset columns |
| A tab | Close, Close others, Split pane, Copy path, Reveal in Explorer |
| A filter chip | Edit, Include ⇄ exclude, Disable, Remove |
| The gutter | Bookmark, Clear bookmarks |
| A status chip | The same menu its left-click drops |

**"Define format from this line"** is the one §6.5 names explicitly, and it is the reason the grid's
context menu exists at all: the wizard is documented as opening on a right-clicked line.

---

### 2.5 The column header `[v1]`

*(Settled 2026-08-21, from the owner's observation that the header does not read as a header.)*

**The header is a control strip, not a caption.** It already carries three gestures — click a title
to cycle the sort, drag a boundary to resize, drag a title to reorder — and §2.4 gives it a context
menu. What it has never had is anything on screen saying so.

**Why it does not read as a header, measured rather than guessed.** The band *is* filled, with
`header_bg`; the fill is simply invisible. Against the row background that is **1.11 : 1** in the
dark theme and **1.16 : 1** in the light one, where nothing is discernible below about 1.5. Worse,
the header's ink is **6.9 : 1** dark and **4.8 : 1** light against a row ink of **14.3 : 1** — so
the strip that names the columns is drawn *fainter than the data it names*. A header quieter than
its own rows is backwards, and no change of typeface fixes it.

**Sorting is kept, and deliberately not promoted.** `SPEC.md` §11.4 already makes sorting a mode
you leave following for: it disables follow and says so, rows arriving while sorted are held rather
than inserted, the status counts them, and clearing the sort releases them in file order. That is
the right behaviour and it is why sorting a live log is coherent at all. But it means **sort is the
one header action that breaks the tool's primary mode**, so it does not get an inviting click
target. Its routes are the context menu of §2.4, the menu bar, and the indicator already drawn
beside a sorted column's title. Explorer invites sorting because a folder is static; a log is not.

Three consequences follow, and they are the design rules for this strip:

- **Filter belongs in the header; sort does not.** Filtering preserves following, sorting ends it.
  That asymmetry, not familiarity with spreadsheets, is what decides which action earns the
  prominent affordance. (The per-field filter *action* is `[v2]` — see §1.2's note on scope.)
- **Top-N deserves a better route than the palette.** It is the honest answer to "show me the
  slowest requests" *while still following*, because it is a heap over a scan and §11.4 leaves it
  uncapped. Today it is reachable only as three palette entries per column.
- **Any sort affordance must respect §11.4's eligibility.** Above the cap a whole sort is refused,
  and a control that invites a click it will decline fails §1.1's "never lie".

### 2.5.1 Colour a column by its values `[v2]` — proposed 2026-08-21

*(Owner's idea: "click on the column and have the app decide on colours to use for each of those
values".)*

A column whose values come from a small fixed set — `note`, `test`, `ci`, `task`; or `GET`, `POST`,
`PUT`; or a service name — is a column the eye should be able to sort at a glance. Choosing the
column and letting the app assign a colour per distinct value is a **zero-typing** version of what
§5's rules already do with a hand-written pattern each.

What makes it cheap rather than a new subsystem:

- **The palette exists.** `Theme::labels` is nine colours already chosen to sit together and already
  reused per theme; §7.1's semantic hues are the precedent for assigning them by meaning rather than
  by the user picking.
- **The header is now a control surface** (§2.5), so "click the column" has somewhere to live, and
  the context menu of §2.4 is where it belongs rather than as a fourth gesture on the strip.
- **Distinct values are already computable**: the filter engine parses field-scoped predicates, so
  the same column accessor that answers `kind = note` can enumerate what `kind` contains.

The parts that need deciding, and none of them are obvious:

- **What happens past nine values.** Nine labels, and a column with forty services. Colour the top
  nine by frequency and leave the rest plain? Hash to a hue and accept collisions? Refuse and say
  why? Refusing is the honest option and the least useful one.
- **Whether it survives a reopen**, and therefore whether it belongs in §12.4's per-file state
  beside the chips and bookmarks — where sort keys deliberately are *not* held.
- **How it composes with §7.1's semantic layer and §5's user rules**, which already colour parts of
  the same line. Three sources of colour on one row is where "restrained palette" stops being true.
- **High Contrast**, where `suppress_rules` turns user highlighting off. This is user highlighting
  by another name and should almost certainly go with it.

Not started. Recorded here rather than in a backlog because the constraints above are the design,
and they are cheaper to settle now than to discover during implementation.

**Removing sorting was considered and rejected**, and the reasoning is worth keeping because the
saving looks larger than it is. `sort.rs` is *"Column sort and top-N"* — one module, one `Order`
carrying `top: Option<usize>`, sharing the key extraction, the comparison, the direction handling
and the rule that a value the format cannot read is **missing** rather than text, so an unknown
spelling never floats to the top. Dropping sort drops top-N with it. And the derived row space it
appears to justify is not its own: `Filtering::active()` is `filtered() || sorted().is_some()`, so
the `file_row`/`view_row` indirection exists for filtering regardless. Removal would buy a handful
of call sites, cost the feature that answers the question people actually ask, and leave the
architecture where it was.

---

## 3. Split view — the two-pane model

klogg's most-validated layout, plus the in-place hide it has an open request for. Both ship; the user
chooses per tab.

```
┌───────────────────────────────────────────────────────────────────────────────────────────┐
│ ⌕ timeout                                    │ ▼ Filter: level >= Warning        ⚙ Rules  │
├──┬────────────┬─────────────────────┬─────┬──────────┬───────────────────────────────────┤
│  │ 4,182,993  │ 09:14:02.117        │ INF │ Api.Cont │ Started HTTP GET /api/contacts    │  │  ← full log
│▌ │ 4,182,995  │ 09:14:03.884        │ ERR │ Api.Disp │ Failed to dispatch job 41982      │  │
│  │ 4,183,001  │ 09:14:04.002        │ WRN │ Api.Cont │ Retry 1/3 for job 41982           │  │
╞══╪════════════╪═════════════════════╪═════╪══════════╪═══════════════════════════════════╡  ← draggable
│  │ 3,918,004  │ 08:51:19.771        │ ERR │ Api.Sql  │ Connection timeout after 30000ms  │  │     splitter
│▌ │ 4,182,995  │ 09:14:03.884        │ ERR │ Api.Disp │ Failed to dispatch job 41982      │  │  ← matches only
│  │ 4,190,551  │ 09:22:41.006        │ ERR │ Api.Sql  │ Connection timeout after 30000ms  │  │
├──┴────────────┴─────────────────────┴─────┴──────────┴───────────────────────────────────┤
│ 3 of 1,204 matches · scanning 62% ████████░░░░ · cancel                                   │  ← streaming
└───────────────────────────────────────────────────────────────────────────────────────────┘
```

Selecting in either pane scrolls the other to the same record. The bottom pane is a real view, not a
copy — bookmarks, highlight rules and column layout all apply.

### 3.1 Many logs at once — tiled panes, independent follow, and scroll lock

*(Owner's requirement, 2026-08-19. §3 above is one log split two ways; this is several logs at
once, which is a different problem and the one MDI existed to solve.)*

**Tabs and document windows are not two modes. They are both, always** — HooWinTail's arrangement,
and the owner's requirement.

Every open log is a **document window** inside the main window, and every open log has a **tab**,
whatever those windows are doing. Maximise a document window and the result reads as an ordinary
tabbed viewer: one log filling the frame, tabs across the top. Restore it and the others are there
beside or behind it — and the tab strip is still the way to bring one to the front, which is the
thing that makes overlapping windows usable rather than a pile.

So a tab is *"show me this document"*, and it means that in every layout. It does not mean "switch
to the only visible pane"; there may be four visible, and clicking a tab raises and focuses one of
them. The View menu offers **Maximise**, **Tile** and **Cascade** over the document windows, per
§2.2; none of them takes the tabs away.

**This reverses §1.1's rejection of MDI child windows** — the third of that section's original
positions to be overturned, after menus and toolbars. What was actually wrong with MDI was never
the child windows; it was the grey bevelled chrome, the title bar on every child eating vertical
space, and windows that could not be snapped or tiled without dragging them by hand. A document
window here is drawn in the app's own flat register with a one-line header, and Tile and Cascade
are commands rather than an exercise in mouse dexterity. The rest of §1.1 — no ribbon, no toolbar of
unlabelled icons, thin quiet chrome — is untouched.

**Tiled.** The window divides into a grid of panes — 2-up, 2×2, or arbitrary splits by dragging a
pane edge. Every pane is a full independent document: its own file (or rotated set), its own
filters, its own columns, its own bookmarks. Nothing about a pane is a lesser version of a tab.

**Follow is per pane.** All panes tail by default, and *pausing one does not pause the others* —
scroll back through the gateway's log while the API's keeps running at the tail. This is the
behaviour that makes tiling worth having, and §12's "scrolling up auto-pauses follow" applies to
the pane under the pointer alone.

**Scroll lock — the point of the whole arrangement.** Panes may be **linked**, and a linked pane
follows the one being scrolled. Two ways to link, because they answer different questions:

| Lock | Links on | For |
|---|---|---|
| **By timestamp** | The record time | *"What was the gateway doing when the API threw?"* — the reason to tile at all |
| **By line number** | The physical line number | Two runs of the same log, or a file against its own rotated predecessor |

A timestamp lock needs no shared clock discipline to be useful, but it must be honest about what it
does not know: a pane whose format carries no timezone (log4net `%date`, RFC 3164) says so, the same
way §4's merged view does, and offers the same per-source override. A pane whose format has **no
timestamp at all** cannot take part in a timestamp lock and is shown as unlinkable with the reason,
rather than silently scrolling to nothing.

Lock state is visible: linked panes carry a matching link mark, and the status bar says what the
lock is on. Locking is opt-in, per pane, and a pane can be dropped out of the group without
disturbing the rest.

**How this relates to §4.** The merged timeline puts every source in *one* grid, interleaved. Tiling
with a timestamp lock keeps them in *separate* grids, aligned. They answer the same question and
neither replaces the other: merge is better for a request crossing four services; tiled-and-locked
is better when each log has its own columns worth reading. §4 stays `[v2]`; this is v1.

**The streaming bar is not cosmetic.** Filtering is a full-file pass (SPEC §7.2), so the UI tells the
truth about it: partial results appear immediately, the match counter climbs, the scrollbar is
provisional, and cancel is always available. On UNC sources the filter field requires **Enter** rather
than filtering as you type, and the field's placeholder says so.

---

## 4. Merged timeline view `[v2]`

**Not built in v1.** Designed now so the v1 grid, record model and source abstraction do not foreclose
it. The flagship differentiator, and the place where a naive implementation looks broken.

```
┌───────────────────────────────────────────────────────────────────────────────────────────┐
│ 🦅  Merged: 4 sources  ×  │  +                                        ─  □  ✕            │
├───────────────────────────────────────────────────────────────────────────────────────────┤
│ ⌕ Search…                        │ ▼ Filter…                    │ ⏱ Reorder window: 2s ▾  │
├──┬──────────────────┬────────────┬─────┬─────────────┬───────────────────────────────────┤
│  │ Timestamp        │ Source     │ Lvl │ Logger      │ Message                           │
├──┼──────────────────┼────────────┼─────┼─────────────┼───────────────────────────────────┤
│  │ 09:14:02.117     │ ▌api       │ INF │ Api.Cont    │ Started HTTP GET /api/contacts    │
│  │ 09:14:02.painted │ ▌gateway   │ INF │ Gw.Proxy    │ Forwarding to api:8080            │
│  │ 09:14:03.884     │ ▌api       │ ERR │ Api.Disp    │ Failed to dispatch job 41982      │
│  │ 09:14:03.901     │ ▌jobs      │ WRN │ Job.Runner  │ Job 41982 marked for retry        │
├──┴──────────────────┴────────────┴─────┴─────────────┴───────────────────────────────────┤
│░░│ 09:14:04.112     │ ▌jobs      │ INF │ Job.Runner  │ Retry scheduled                   │  ← settling band
│░░│ 09:14:04.118     │ ▌gateway   │ INF │ Gw.Proxy    │ 502 returned to client            │     (dimmed)
├───────────────────────────────────────────────────────────────────────────────────────────┤
│ ⬤ Following · lagging 2s │ 4 sources │ ⚠ gateway: no timezone — using local ▾             │
└───────────────────────────────────────────────────────────────────────────────────────────┘
```

Four design decisions carry the honesty principle:

- **The settling band is visible.** Records inside the bounded reorder window (default 2s) render
  dimmed with a hatched gutter. They can and will reorder. Once committed they never move again. Async
  writers (Serilog batching, NLog `AsyncWrapper`) emit out of timestamp order by up to their flush
  interval, so without this the viewport jumps under the cursor once a second and the feature reads as
  buggy.
- **The lag is stated**, in the status bar: `lagging 2s`. The reorder window is adjustable from the
  command bar.
- **Timezone problems are surfaced, not guessed.** A source whose format carries no zone (log4net
  `%date`, RFC 3164) shows a warning chip with a one-click per-source timezone override. A source that
  cannot participate at all (Serilog console default has *no date*) is shown greyed in the source list
  with the reason.
- **Source colour is a stable left stripe**, not a colour applied to the text — text colour belongs to
  the user's highlight rules.

**Columns are the union** of participating sources, with `Source` always present. Per-source constants
(host, service — OTel `resource`) appear in the pane header, not repeated on every row.

**Scrollback is bounded at 1M merged records** (SPEC §8.3). Scrolling beyond it shows a quiet marker —
*"streaming older records…"* — while a fresh k-way merge runs. It does not pretend to be instant.

---

## 5. Highlight rules and filters

Inline, non-modal, live preview over the real file. This is the surface that replaces LogExpert's
"set up a dev environment and compile a DLL".

```
┌─ Rules — “App production” ────────────────────────────────────────────────── ✕ ──┐
│                                                                                   │
│  ⣿ ☑  ERROR|FATAL                        .*  ▉ red on ▉ ─    ▸ whole line   ⋮     │
│  ⣿ ☑  timeout|timed out                  Ab  ▉ amber        ▸ match only    ⋮     │
│  ⣿ ☑  (?<id>0HN[A-Z0-9]+)                .*  ▉ auto-colour  ▸ identifier    ⋮     │
│  ⣿ ☐  deprecated                         Ab  ▉ grey         ▸ match only    ⋮     │
│                                                                                   │
│  + Add rule                                        Import…  Export…  Apply to ▾   │
├───────────────────────────────────────────────────────────────────────────────────┤
│  Preview — 200 lines from this file                                               │
│  09:14:03.884 ERR Api.Disp  Failed to dispatch job 41982 ▉                        │
│  08:51:19.771 ERR Api.Sql   Connection timeout after 30000ms ▉                    │
└───────────────────────────────────────────────────────────────────────────────────┘
```

- **`.*` / `Ab` toggle** per rule selects regex vs plain text. Regex validity is checked as you type,
  with the error shown inline — never on OK.
- **Drag handle (`⣿`)** reorders; precedence is top-down and visible.
- **Checkbox** enables/disables without deleting — a Hoo WinTail affordance the owner uses.
- **`Apply to ▾`** binds the set to *this file*, *a glob* (`C:\logs\jobdispatcher\*.log`), or *a
  detected format*. Binding to a glob or format is the thing no incumbent does.
- **Import/Export** writes a shareable file — and **imported rules are treated as untrusted**
  (SPEC §13.1): regexes compile with explicit size limits, and a rule that would arm an action or
  reference a remote path is rejected on import with a clear explanation.
- **`identifier`** marks a capture group as correlatable, feeding §7.

Beneath user rules sits the **zero-config semantic layer** — timestamps, durations, GUIDs, IPs, URLs,
paths, HTTP verbs and status codes, `key=value`, quoted strings — on by default. klogg's fatal UX flaw
is an empty highlighter set on first run; Tailhawk is useful before you configure anything.

---

## 6. Format detection and the format wizard

### 6.1 The chip

The format chip is the whole trust model in one control:

```
   Serilog (file) 99.2% ▾
   ┌────────────────────────────────────────┐
   │ ✓ Serilog (file)              99.2%    │
   │   log4net                     71.4%    │  ← runner-up shown when margin < 15%
   │   Plain text                           │
   │ ─────────────────────────────────────  │
   │   Define format from a line…           │
   │   Import layout from config…           │
   │   Scan folder for logging config…      │
   │ ─────────────────────────────────────  │
   │   Remember for  ○ this file            │
   │                 ● C:\logs\ndc\*.log    │
   └────────────────────────────────────────┘
```

When no format clears 0.75 absolute **and** a 15% margin, the chip renders in a warning state —
*"Detected: Serilog (file) — also matched log4net"* — rather than silently picking. Silent
mis-columnising is worse than no columnising.

**A remembered definition wins over detection, and the chip says so.** *(Settled 2026-08-21.)*
§6.5.1's "Remember for" is an instruction, not a hint: a user who chose a glob has told Tailhawk
what this family of files is, and a format that loses to a confident sniff the next time it is
opened has not remembered anything. So when a saved definition claims the path, it is what compiles,
whatever detection scored.

That is only safe because the override is **visible rather than merely available**. The chip has a
third state beside "detected" and "warning":

```
   ★ NDC pipeline (remembered) ▾          ← a saved definition claimed this path
   ┌────────────────────────────────────────┐
   │ ★ NDC pipeline    C:\logs\ndc\*.log    │
   │   ─────────────────────────────────    │
   │   Serilog (file)              99.2%    │  ← what detection would have picked
   │   Plain text                           │
   │   ─────────────────────────────────    │
   │   Forget C:\logs\ndc\*.log             │
   └────────────────────────────────────────┘
```

Three things that state has to do, and each answers an objection to letting the memory win:

- **Name the glob that claimed the file**, so a rule written months ago for one service and now
  catching another is legible at a glance rather than a mystery about why the columns look wrong.
- **Still show what detection thought**, so overriding a 99% Serilog match is a visible choice and
  one click to undo — the user does not have to guess what they are giving up.
- **Offer "Forget"** in the same menu that offered "Remember". A memory with no way to un-remember
  is a trap, and the chip is where the user already looks.

The precedence when several definitions claim the same path is `load`'s tier order — exe-adjacent
before roaming, first match wins — and `Definition::claims` is where a bare pattern is matched
against the file name and a rooted one against the whole path.

### 6.2 Define from example

```
┌─ Define format ───────────────────────────────────────────────────────────── ✕ ──┐
│  Example line (right-clicked)                                                     │
│  2026-07-28 09:14:02,117 [12] INFO  Zenith.Automation.Runner - Evaluated 412…     │
│  ├──── ts ────────────┤ ├th┤ ├lvl┤  ├──── logger ─────────┤   ├─── message ───    │
│                                                                                   │
│  Pattern   <ts> [<thread>] <level> <logger> - <message>            Edit as regex… │
│                                                                                   │
│  Roles     ts → Timestamp ▾   level → Severity ▾   message → Body ▾               │
│                                                                                   │
├───────────────────────────────────────────────────────────────────────────────────┤
│  Preview — next 200 lines                                    ✓ 198 matched (99%)  │
│  ┌──────────────────┬────┬─────┬──────────────────────┬────────────────────────┐  │
│  │ 09:14:02,117     │ 12 │ INFO│ Zenith.Automation.R… │ Evaluated 412 triggers │  │
│  │ 09:14:03,884     │ 12 │ ERROR│ Zenith.Data.Session │ Could not open connect │  │
│  └──────────────────┴────┴─────┴──────────────────────┴────────────────────────┘  │
│                                                    Save as…    Test    Cancel     │
└───────────────────────────────────────────────────────────────────────────────────┘
```

The user drags the boundary handles under the example line; the pattern and the preview update live.
The generated artefact is a **pattern DSL** string (`<name>` captures, `<_>` discards), not a regex —
it is what a normal engineer can read and edit, and it compiles to a linear scanner. "Edit as regex"
is there for the 10% who need it. **Test** re-runs the definition's stored sample lines.

### 6.3 Import a layout — paste first, scan if you can

```
┌─ Import layout ───────────────────────────────────────────────────────────── ✕ ──┐
│  Paste a layout string from your logging config:                                  │
│  ┌─────────────────────────────────────────────────────────────────────────────┐  │
│  │ ${longdate}|${level:uppercase=true}|${logger}|${message}                     │  │
│  └─────────────────────────────────────────────────────────────────────────────┘  │
│  Recognised as: NLog layout                                          ✓ compiles   │
│                                                                                   │
│  ── or ──                                                                         │
│  🔍 Scan folder for logging config                                                │
│     Found in C:\dev\ndc\Api\:                                                     │
│       ● appsettings.json → Serilog:WriteTo:Args:outputTemplate                    │
│       ○ NLog.config → target "file" layout                                        │
└───────────────────────────────────────────────────────────────────────────────────┘
```

Two clicks instead of a regex-writing session. Serilog `outputTemplate`, NLog `layout`, log4net
`conversionPattern` and Logback patterns are all accepted.

**The paste box is the primary route and the scan is the fallback**, which is why the box is at the
top and the scan sits under an "── or ──". A viewer normally has the log and nothing else: logs are
copied off a server, read from a share, or sent by someone, and an application that ships to Loki or
Seq keeps no config anywhere near its output. Pasting needs only the text — from source control, a
pull request, whoever owns the service — so it works in every one of those cases. The scan needs the
config to be a few directories up from the log, which is the development machine and little else.

That ordering is already what the mock draws; it is stated here because the section used to be
titled "the .NET differentiator", which invited the opposite reading and would have someone build
toward an access model that mostly does not exist.

---

## 7. Trace correlation `[v2]`

**Not built in v1.** Clicking an identifier — a GUID, a `traceparent`, a CLEF `@tr`, or any column marked `identifier` —
opens an inline affordance rather than a dialog:

```
│  4,182,995  09:14:03.884  ERR  Api.Disp  Failed to dispatch job 41982
│                                          trace 4bf92f3577b34da6a3ce929d0e0e4736
│                                          ┌──────────────────────────────────────┐
│                                          │ 47 records · 4 sources               │
│                                          │ ⟨ prev   next ⟩                      │
│                                          │ ⌕ Filter to this trace               │
│                                          │ ⊞ Group as one operation             │
│                                          │ ⧉ Copy trace id                      │
│                                          └──────────────────────────────────────┘
```

Every occurrence of that identifier gets a **stable derived colour** across all open sources, so the
same request is the same colour everywhere. "Group as one operation" collapses a request's lines into
a single expandable unit (lnav's `opid` model).

---

## 8. Record detail

For very long lines, structured payloads and stack traces, a detail pane (`Ctrl+Enter`, or the row
chevron) opens at the bottom or right:

```
┌─ Record 4,182,995 ────────────────────────────────────────────────── ⇱ ⇲  ✕ ──┐
│  Timestamp   2026-07-28T09:14:03.8841200+01:00                                 │
│  Severity    ERROR (17)                                                        │
│  Logger      Zenith.JobDispatcher.Dispatcher                                    │
│  Trace       4bf92f3577b34da6a3ce929d0e0e4736 · span 00f067aa0ba902b7           │
│  ───────────────────────────────────────────────────────────────────────────   │
│  Body        Failed to dispatch job 41982                                      │
│  Exception   System.InvalidOperationException: Queue 'jobs' is not registered   │
│                 at Zenith.JobDispatcher.Dispatcher.Dispatch(Job job)            │
│                 at Zenith.JobDispatcher.Worker.Run()                            │
│  ───────────────────────────────────────────────────────────────────────────   │
│  Properties  JobId 41982 · Queue "jobs" · MachineName "APP01"                   │
│                                                        Raw  Pretty  ⧉ Copy ▾    │
└────────────────────────────────────────────────────────────────────────────────┘
```

**Raw** shows the original bytes exactly as they appear in the file — always available, because the
record model retains them losslessly. **Pretty** JSON-formats a structured body.

**`[v2]`** — Every field gains a **filter for this value / filter out this value** action, borrowed
from Grafana, which no desktop log viewer has. It creates an ordinary include or exclude chip in the
command bar (§2.1), editable as text like any other. **These are buttons in a persistent row on the
selected field, not hover-only affordances** — hover-only interactions are invisible to keyboard users
and, more practically, they defeat the RDP scroll-blit path (§15) by dirtying regions on mouse move.

For a truncated long line the Body area shows the cap and the escape:

```
│  Body        {"request":{"headers":{…32 KB shown of 41.2 MB…                    │
│              ▸ expand    ▸ open in viewer    ⧉ copy full
```

---

## 9. Command palette

`Ctrl+K`. The single discovery surface, which is why the chrome can stay thin.

```
┌───────────────────────────────────────────────────────────────────────┐
│ ⌘ merge                                                               │
├───────────────────────────────────────────────────────────────────────┤
│  ⊞  Merge open files by timestamp                                     │
│  ⊞  Merge selected tabs…                                              │
│  ⏱  Set reorder window…                                    2s         │
│  ─────────────────────────────────────────────────────────────────    │
│  Recent                                                               │
│  ⌕  Filter: level >= Warning                                          │
│  📁 Open file set: App production                                     │
└───────────────────────────────────────────────────────────────────────┘
```

Everything reachable by menu is reachable here, plus file sets, watched folders, saved filters and rule
sets by name.

---

## 10. States that are usually neglected

Honesty principle #2 in practice. Each of these is a designed state, not an error dialog.

**Waiting for a file that does not exist yet** — `tail -F` semantics as the default:
```
│                                                                       │
│                        ⏳  Waiting for                                 │
│                        C:\logs\jobdispatcher\app.log                   │
│                        Watching the folder — will start automatically  │
│                                                                       │
```

**The writer has locked us out** — the single most useful error in the product, because it names the
fix on the *writer's* side, which is the only side that can fix it:
```
│   ⚠  Cannot read app.log — the writing process has opened it exclusively.        │
│                                                                                  │
│      This is a setting on the application that writes the log, not on Tailhawk.  │
│                                                                                  │
│      Serilog    add  shared: true  to the file sink                              │
│      NLog       set  keepFileOpen="false"  or  concurrentWrites="true"           │
│      log4net    use  <lockingModel type="…FileAppender+MinimalLock"/>            │
│                                                                    Retry  ▸      │
```

**Network mode** — a persistent status chip, not a one-time toast: `⚠ Network mode — polling every
500 ms, updates may lag`.

**Stateless mode** — `⚠ Settings will not be saved` with a tooltip explaining that the exe's folder is
read-only (running from a share) and offering to use `%APPDATA%` instead.

**Format changed mid-file** — non-modal, in the format chip: *"This file may have changed format —
re-detect?"* Triggered when the rolling non-match rate exceeds ~20%.

**Skipped ahead under load** — when the render loop drops behind a very fast writer, a quiet inline
marker: `⋯ skipped ahead — 41,882 lines while catching up`. Data was never lost; the *view* skipped,
and it says so.

---

## 10b. Remote Desktop — reduced fidelity `[v1]`

`SPEC.md` §3.2 makes RDP a first-class v1 render path with a scroll-blit invariant, and an earlier
draft of this document never mentioned it — including in the "states that are usually neglected"
section — while specifying hover interactions that would defeat it. A log viewer is used over RDP and
on jump boxes constantly, so this is a designed mode, not a degradation.

Detected via `GetSystemMetrics(SM_REMOTESESSION)`. **WARP is not a trigger** — a local software-rendered
session renders normally.

| Element | Local | Over RDP |
|---|---|---|
| Repaint rate | 60 Hz | **~15 Hz**, coalesced |
| Scrolling | Full redraw | **Scroll-region blit** — only newly exposed rows drawn |
| Mica / Acrylic chrome | On | **Off** — flat opaque surfaces; translucency is expensive to encode |
| Smooth / inertial scroll | On | **Off** — snaps to whole rows, so each frame is a clean blit |
| Hover affordances | Available | **Suppressed** — they dirty regions on mouse move and defeat the blit |
| Selection, detail actions, per-field filters | Hover or click | **Persistent controls with key bindings** |
| Density marks in the scrollbar | Live | Redrawn on scroll end, not during |
| Status chip | — | **`⚠ Remote session — reduced fidelity`**, explaining the changes on hover/focus |

**The design rule this imposes on every other section: no interaction may be hover-only.** Anything
reachable by hover must also be reachable by keyboard and visible as a persistent control when
selected. That is also what makes the chrome testable through the v1 UIA provider (§13).

## 11. Visual language

### 11.1 Typography

- **Grid:** Cascadia Mono, falling back to Consolas, then the system monospace. Size and line height
  user-adjustable; default 9pt at 100%.
- **Chrome:** Segoe UI Variable where available, Segoe UI otherwise.
- **The grid renders to an opaque target**, which per `D2D1_TEXT_ANTIALIAS_MODE` gets ClearType by
  default — but this is a free consequence of the architecture, **not a design goal, not a user
  setting, and not something schedule is spent on** (SPEC §3.2). Greyscale is an acceptable outcome.
  Mica is confined to the title bar, tab strip and panels.

### 11.2 Colour

The palette is deliberately restrained so that **user highlight colours are the loudest thing on
screen**. Severity uses a colour-blind-safe ramp **plus a redundant gutter glyph**, never hue alone:

| Severity | Glyph | Dark theme | Light theme |
|---|---|---|---|
| FATAL (21–24) | `■` | magenta-red | dark magenta |
| ERROR (17–20) | `▲` | red-orange | dark red |
| WARN (13–16) | `▲` outline | amber | dark amber |
| INFO (9–12) | `·` | foreground | foreground |
| DEBUG (5–8) | `·` dim | muted | muted |
| TRACE (1–4) | `·` faint | more muted | more muted |
| *(none)* | *(blank)* | foreground | foreground |

Severity with no value renders **blank**, not INFO — W3C, nginx and logfmt rows genuinely have no
severity and the OTel spec sanctions leaving it empty.

**High Contrast:** system colours are respected and **user highlight rules are suppressed**, with a
visible chip explaining why — they would otherwise be invisible or illegible.

### 11.3 Density and DPI

Three density settings (Compact / Default / Comfortable) affecting row height only. Per-monitor-V2
DPI: metrics recompute on `WM_DPICHANGED`, the glyph atlas rebuilds per scale factor, and **column
advances are integer device pixels** so no drift accumulates. Acceptance test: drag between a 100% and
a 150% monitor and verify no column misalignment.

---

## 12. Keyboard map

Muscle memory from `less`, Visual Studio and the incumbents, in that order of precedence where they
conflict.

| Key | Action |
|---|---|
| `Alt` | Focus the menu bar; `Alt+F`, `Alt+E`, `Alt+V`, `Alt+O`, `Alt+R`, `Alt+H` open a menu directly (§2.2) |
| `Ctrl+K` | Command palette |
| `Ctrl+O` / `Ctrl+Shift+O` | Open file / open file set |
| `Ctrl+F` / `F3` / `Shift+F3` | Search / next / previous |
| `Ctrl+L` | Focus filter |
| `Ctrl+G` | Go to line |
| `Ctrl+H` | Rules editor |
| `Ctrl+T` / `Ctrl+N` / `Ctrl+M` / `R` | **Inside the format wizard only** (§6.2): test, split a field, merge, cycle its role. The wizard is modal, so while it is up `Ctrl+D`, `Space` and `F` are its own too, not the rows in this table. |
| `F` | Toggle follow (also `Ctrl+End` jumps to tail and re-enables) |
| `Ctrl+Enter` | Record detail pane |
| `Ctrl+D` / `Ctrl+Shift+D` | Toggle bookmark / bookmarks panel |
| `Ctrl+Shift+0…9` | Numbered bookmarks (Hoo WinTail parity) |
| `Ctrl+Shift+1…9` | Ad-hoc colour label on selection (klogg parity) |
| `Ctrl+Tab` / `Ctrl+Shift+Tab` | Next / previous tab |
| `Ctrl+W` | Close tab |
| `Ctrl+\` | Split pane |
| `Shift+wheel` | Horizontal scroll |
| `Ctrl+M` | **`[v2]`** Merge selected tabs by timestamp |
| `Ctrl+C` / `Ctrl+Shift+C` | Copy raw / copy as TSV with columns |
| `Alt+←` / `Alt+→` | Back / forward through view states (nerdlog's idea — filter and position history) |
| `Home` / `End` / `Ctrl+Home` / `Ctrl+End` | Line and document extremes |
| `Space` / `b` | Page down / up (`less` muscle memory) |

**Scrolling up while following auto-pauses follow** and shows a `⤓ Jump to end` affordance. This is the
single most-wanted behaviour in every tail tool and getting it wrong is very visible.

**Smooth and inertial scrolling** uses `WM_POINTER` (or Direct Manipulation) rather than discrete
`WM_MOUSEWHEEL` deltas — without it the app scrolls in visible jumps next to Edge, VS Code and Terminal,
which is what makes a hand-rolled Win32 app feel homemade.

**Selection** supports shift-click extension, word and line double/triple click, **rectangular
selection** (Alt+drag), and autoscroll-on-drag. All hand-written, all specified, because none is free
in a custom-drawn grid.

---

## 13. Accessibility in the UI

**Split across phases per `SPEC.md` §14.1** — an earlier draft of this section was written in the
present tense as though all of it shipped in v1, while the plan deferred all of it to v2.

| | Phase |
|---|---|
| **Chrome provider** — tabs, buttons, status chips, text fields, palette, dialogs exposed with names, values and focus order | **`[v1]`** |
| **Grid text provider** — virtualised `ITextProvider`/`ITextRangeProvider` over tens of millions of rows, caret and selection eventing | **`[v2]`** |

The v1 half is not primarily an accessibility feature: it is **the only automated interaction-test
surface v1 has**. Without it, tabs, drag-out-to-split, the palette, the rules editor, the format
wizard and eleven text fields are validated forever by one person dragging a mouse.

Design-level consequences of SPEC §14.1:

- **Live-tail is quiet by default.** A screen reader is not fed 1,000 lines/second. An explicit
  *"read new lines"* action, and an optional *"announce matches of rule X only"* mode, replace naive
  live-region announcement.
- **Every status chip is a focusable control** with a name and a value, so the format, encoding, follow
  state and parse health are all reachable without sight.
- **Focus order** is defined across the custom-drawn surface: tab strip → search → filter → grid →
  status chips.
- **`[v2]`** The grid exposes a virtualised UIA text/table provider.
- **No interaction is hover-only** (§10b) — a rule imposed by the RDP path, and one that also makes
  every affordance reachable by keyboard and by an automated test.

---

## 14. First-run experience

There is no wizard, no account, no telemetry consent, and no "getting started" tour. The app opens,
and if launched without arguments shows a single quiet surface:

```
│                              🦅                                        │
│                     Watch your logs like a hawk                        │
│                                                                        │
│          Drop a log file here, or press Ctrl+O                         │
│                                                                        │
│          Recent                                                        │
│          C:\logs\ndc\api.log                          8.2 GB           │
│          App production (file set — 6 files)                           │
│                                                                        │
│          Tailhawk never phones home.                                   │
```

That last line is a deliberate, verifiable claim and a competitive differentiator, so it is stated on
the surface the user sees first.

**The wording matters and an earlier draft got it wrong.** It said *"Tailhawk makes no network
connections"* — which is falsified the moment the user opens a log on a UNC share, a first-class v1
source with its own status chip in this same document. The honest claim, and the one SPEC §13.2
actually specifies, is that Tailhawk initiates **no outbound connection of its own**: no telemetry, no
update ping, no font or CDN fetch. Network I/O occurs only to sources the user explicitly opened. The
CI assertion must be written to observe kernel-mode SMB traffic from a UNC open and *exclude* it,
rather than asserting zero sockets — otherwise the test either fails on the first UNC fixture or
proves nothing.
