# UX review — Tailhawk against Microsoft's Windows UX guidance

**Asked for by the owner on 2026-09-09:** *"read the windows ux guidance, and rigorously review the
current ui against that. I want this to satisfy those guidelines wherever possible. If that is not
possible, then lets review the options."*

**What was read.** Microsoft Learn's Win32 UX Guide, the pages that cover the surfaces this program
has: [Menus](https://learn.microsoft.com/en-us/windows/win32/uxguide/cmd-menus),
[Toolbars](https://learn.microsoft.com/en-us/windows/win32/uxguide/cmd-toolbars),
[List Views](https://learn.microsoft.com/en-us/windows/win32/uxguide/ctrl-list-views) and the
[UX checklist for desktop applications](https://learn.microsoft.com/en-us/windows/win32/uxguide/top-violations),
which is Microsoft's own list of the violations programs commit most. Every quotation below is from
those pages. The guide carries a banner saying it was written for Windows 7 and that "much of the
guidance still applies in principle" — where a rule has been overtaken by how Windows 11 actually
behaves, that is said in the finding rather than glossed.

**How to read the verdicts.**

| | |
|---|---|
| **Fails** | The guidance is clear, we do not follow it, and there is no reason not to. Fix. |
| **Partly** | We follow the letter and miss the intent, or follow it in one place and not another. |
| **Decision** | It conflicts with something the owner has already decided. His call stands; recorded so it is a decision rather than an oversight. |
| **Platform** | The platform will not do it. Options given. |

---

## Summary

| # | Surface | Verdict | Finding |
|---|---|---|---|
| 1 | Menu bar | **Fixed 2026-09-09** | `Settings` is a menu category and `Preferences…` a menu item. The guide forbids both words by name. |
| 2 | Menu bar | **Fixed 2026-09-09** | `Preferences…` carries an ellipsis it should not have. |
| 3 | Context menu | **Fixed 2026-09-09** | The row context menu prints shortcut keys. Context menus never do. |
| 4 | Edit menu | **Part fixed** | No `Select all`; `Go to line` is under View where the standard puts it in Edit. |
| 5 | Toolbar | **Fails** | No `Open remote source` button, though it is among the most used commands. |
| 6 | Toolbar | **Fails** | `Collapse`'s icon is not self-explanatory — the owner could not tell what it was. |
| 7 | Toolbar | **Partly** | Right-clicking the toolbar does nothing; a toolbar with options owes a context menu. |
| 8 | Column header | **Fails** | No way to choose columns. The guide names the exact remedy. |
| 9 | Applications picker | **Fixed 2026-09-09** | Commit buttons are spread across the bottom instead of right-aligned in one row. |
| 10 | Highlight rules | **Fixed 2026-09-09** | Commit buttons are not right-aligned; `Close` carries an access key it should not have. |
| 11 | Highlight rules | **Done as asked** | Modeless by §5's design; the owner wants modal with Save/Cancel. |
| 12 | Title bar | **Fails** | The title carries frame timings and atlas statistics — diagnostics shipped to a user. |
| 13 | Detail pane | **Fails** | Drawn over the grid with no frame of its own; the rule line renders as a row of boxes. |
| 14 | Column headings | **Partly** | Headings are lower-case field names; the guide asks for sentence-style capitalisation. |
| 15 | Menus | **Platform** | Win32 menus cannot be dark-themed. Already measured and recorded. |

---

## 1. `Settings` and `Preferences` are both forbidden words — **Fails**

> **Use the following menu item names for the stated purpose: Options** — To display program
> options. … **Preferences** — Don't use. Use Options instead. … **Settings** — Don't use as a menu
> label. Use Options instead.
> — *Menus*, Labels

Tailhawk has a top-level **Settings** category containing **Preferences…**. That is the same rule
broken twice in one place, and it is the guide's own example of what not to write.

**Fix.** Rename the category **Tools** and the item **Options** (no ellipsis, see finding 2). The
guide's standard bar is *File, Edit, View, Tools, Help*, and Tools is where Options belongs. `Dark
theme` and `Remote sources…` move with it; `Font…` from the Format menu belongs there too, leaving
Format for what shapes the log.

## 2. `Preferences…` should not have an ellipsis — **Fails**

> Commands whose implicit verb is to show another window don't take an ellipsis, such as **Advanced,
> Help, Options, Properties, or Settings**.
> — *UX checklist*, Text

Every other ellipsis in the menus is correct: `Go to line…`, `Define from a line…`, `Import layout…`,
`Highlight rules…`, `Remote sources…` and `Open…` all gather information before acting. Options does
not.

**Fix.** `Options` with no ellipsis, as part of finding 1.

## 3. The row context menu prints shortcut keys — **Fails**

> **Document all shortcut keys.** … **Exception:** Don't display shortcut key assignments within
> context menus. Context menus don't display the shortcut key assignments because they are optimized
> for efficiency.
> — *Menus*, Shortcut keys

Our row context menu shows `Bookmark  Ctrl+D`, `Follow this trace  Ctrl+T` and the rest. The same
page also says context menus **remove** rather than disable items that do not apply, which we should
check while we are in there.

**Fix.** Strip the accelerator column from context menus; keep it in the menu bar, which is where
the guide says the teaching happens.

## 4. Edit is missing `Select all`, and `Go to line` is in the wrong menu — **Partly**

> **Edit:** Undo, Redo | Cut, Copy, Paste | **Select all Ctrl+A** | Delete | Find… Ctrl+F, Find next
> F3, Replace… Ctrl+H, **Go to… Ctrl+G**
> — *Menus*, Standard menus

We have Cut/Copy/Paste and the find family in Edit, and `Go to line…` under View. There is no
`Select all`, though the grid supports a selection.

**Fix.** Add `Select all  Ctrl+A` to Edit, and move `Go to line…  Ctrl+G` there from View. Undo and
Redo do not apply — nothing in a log viewer is edited — and the guide's standard list is explicitly
"use it when it makes sense".

## 5. The toolbar has no `Open remote source` — **Fails**

> **For supplemental toolbars, provide commands that are used the most frequently.**
> — *Toolbars*, Controls and commands

Opening a remote source is the command this program exists for and it is reachable only through a
menu. `Open` (a file) has a button; its sibling does not.

**Fix.** A second button beside Open, in the same group, with a tooltip reading `Open remote
source…` — the ellipsis because it gathers information before it acts, which the *Toolbars* page
asks for explicitly.

## 6. `Collapse`'s icon does not say what it does — **Fails**

> **Choose icon designs that clearly communicate their purpose**, especially for the most frequently
> used commands. Well-designed toolbars need icons that are self-explanatory because **users can't
> find commands efficiently using their tooltips**.
> … **Provide labels for frequently used commands,** especially if their icons aren't well-known.
> — *Toolbars*, Icons and Controls

The owner's words on 2026-09-09: *"Im not sure what exctly the colapse item is in the toolbar."*
That is the test failing in the field. The command is *Collapse continuations* — fold a wrapped
record's continuation lines into one row — and a chevron does not say that.

**Fix, in order of preference:** a glyph that shows lines folding together rather than a chevron;
and if no glyph reads clearly, label that one button, which the guide explicitly allows ("some
command buttons can be labeled if they are frequently used"). The tooltip should also say
`Collapse continuations`, not `Collapse`.

## 7. Right-clicking the toolbar does nothing — **Partly**

> **On right-click:** For customizable toolbars, display the context menu for customizing the
> toolbar. … **Provide a context menu with the following commands:** a check box list to display the
> available toolbars, Lock/Unlock toolbars, Customize…
> — *Toolbars*, Interaction and Customization

Ours is not customisable, so the full list does not apply — but it does have options (show/hide,
small/large icons) that live only in a menu three levels down.

**Fix.** Right-click the toolbar or its gripper: `Toolbar` (checked), `Small icons`, `Large icons`.
The same items the View submenu carries, where the hand already is.

## 8. There is no way to choose columns — **Fails**

> **Column chooser.** List views sometimes have so many columns that it isn't practical to show them
> all. In this case, the best approach is to display the most useful columns by default and **allow
> users to add or remove columns as needed**. … **Right-clicking the column heading displays a
> context menu that allows users to add or remove columns.** … Clicking **More** in the column
> header context menu displays the **Choose Columns dialog box**, which allows users to add or
> remove columns as well as reorder them.
> — *List Views*, Usage patterns

A JSON log can produce a column per key. Today the only ways to change what is shown are dragging a
boundary to nothing — which the owner rightly called non-standard — and `Reset columns`.

The same page also says: **"Make double-click behavior redundant. There should always be a command
button or context menu command that has the same effect."** The divider double-click added on
2026-09-09 had no redundant path at all.

**Fix.** Right-click the header → a checked list of columns, then **Select columns…** opening a
dialog with the columns in order, check boxes, Move up / Move down, OK / Cancel. Drop
hide-by-drag-to-zero. Keep the divider double-click as Explorer means it — auto-fit to contents —
now that the menu and dialog carry the function.

## 9. The applications picker's buttons are not in one right-aligned row — **Fails**

> **Right-align commit buttons in a single row across the bottom of the dialog box** … **Present the
> commit buttons in the following order:** OK/[Do it]/Yes, [Don't do it]/No, Cancel, Apply.
> — *Dialog Boxes*; *UX checklist*, Keyboard

`Interleave` sits at the left edge, `Separate windows` beside it, `Cancel` at the right. The two
commit buttons are specific verbs, which the guide prefers to a bare OK — that part is right.

**Fix.** `Interleave` and `Separate windows` move to the right, in that order, with `Cancel` last.

## 10. The rules editor's buttons — **Fails**

Two faults in one row. They are at `Save` (with an access key) and `Close`, laid out under the
right-hand column of verb buttons rather than across the bottom:

> **Don't assign access keys to OK, Cancel, and Close buttons.** Enter and Esc are used for their
> access keys. However, always assign an access key to a control that means OK or Cancel, but has a
> different label.
> — *UX checklist*, Keyboard

So `&Save` is correct — it means OK under a different name — and `&Close` is not.

**Fix.** Right-align the commit row across the bottom, drop the access key from the closing button,
and rename per finding 11.

## 11. The rules editor is modeless — **Decision**

`UI-DESIGN.md` §5 made it modeless deliberately: *"inline, non-modal, live preview over the real
file"* — you type a pattern and the log under it recolours. The guide supports that choice:

> **Modeless dialog boxes** don't require interaction, so use them when users need to switch between
> a dialog box and the owner window. They are best used for frequent, repetitive, or ongoing tasks.
> — *UX checklist*, Dialog boxes

The owner's judgement on 2026-09-09 is that it does not behave like a proper window — *"I can close
it by clicking the menu button"* — and that the buttons should be **Save** and **Cancel**. Cancel
implies the rules revert, which today they cannot: edits apply live and there is nothing to revert
to.

**What that costs, so the choice is informed.** Modal keeps the live preview — the log still
recolours as you type, you simply cannot click into it while the editor is open — and makes Cancel
meaningful, because the editor can restore the rule set it opened with. What is lost is scrolling
the log to a different part of the file to test a pattern against it without closing the editor.

**Recommendation.** Do as he asks: modal, `Save` / `Cancel`, Cancel restoring the rules the editor
opened with. If the "scroll while it is open" case turns out to matter, the modeless behaviour can
come back with an explicit `Apply` — which the guide permits only for property sheets, so it would
be a considered exception rather than the current accident.

## 12. The title bar ships diagnostics — **Fails**

> **Remove redundant text.** Look for redundant text in window titles…
> — *UX checklist*, Text

A real title from the running program:

```
Tailhawk 2026.9.9.20581 — hardware — ● following — toolbar-demo.log: UTF-8 — 1 file —
toolbar-demo.log · timestamped text 100%, 400 lines, 31492 bytes — frame p95 66.0 ms, worst 66.0 ms,
1 over budget — placeholders 33 queued (37 distinct), 0 refused, 1 blanked, 36 landed, 0 sheet-full,
atlas holds 1 blanks of 5115 slots, 1 painter builds
```

The file name appears twice, and everything after "bytes" is instrumentation this project added to
measure itself. It is genuinely useful — several defects this month were found in it — but it is not
a window title.

**Fix.** Title: `<file> — Tailhawk`, the Windows convention. The document's own facts (encoding,
line count, size, following, lag) belong in the status bar, which already exists and is nearly
empty. The frame and atlas instrumentation goes behind the existing `TAILHAWK_ATLAS_LOG`-style
switch — on when a harness asks for it, absent otherwise.

## 13. The detail pane — **Fails**

The owner: *"the show details panel does not look good. it is just splashed over the main window. it
would be better suited to a property page control I would think, also there is a long row of squares
in it above the last line."*

Two separate faults.

**The squares are a rendering defect, and are now diagnosed.** `detail.rs` draws its rule as a
full-width run of `─` (U+2500). Measured with the atlas trace:

```
too large GlyphKey { glyph: 2469, px_per_em: 16 } ink 12x18 slot 11x18
```

The glyph's ink is one pixel wider than the cell, so the atlas refuses it permanently and every
character on that line draws the placeholder box. Any box-drawing character in any log would do the
same, so the fix belongs in the atlas — a glyph one pixel over should be clipped or given the slot
it needs — not in that one line of `detail.rs`.

**The pane has no frame of its own**, which is what "splashed over" means: it is rows drawn in the
grid's own surface, with no border, no title and no close affordance.

> **Use specific, meaningful tab labels.** Avoid generic tab labels that could apply to any tab, such
> as General, Advanced, or Settings.
> — *UX checklist*, Property Sheets

**Fix.** Give it the shape the owner named: a tabbed pane with a real frame, `Fields` / `Raw` /
`JSON` as the tabs — specific labels, not General — a close button, and a splitter between it and
the grid. `SysTabControl32` already runs the tab strip, so it is the control we know.

## 14. Column headings are lower-case field names — **Partly**

> **Heading labels:** Keep the heading labels brief (three words or fewer). Use a single noun or
> noun phrase with no ending punctuation. **Use sentence-style capitalization.**
> — *List Views*, Labels

Ours read `timestamp`, `level`, `message` because they are the log's own field names, which is
honest — a JSON log's keys are its columns, and renaming them would hide what the file says.

**Options.** Leave them (truthful, non-standard); or capitalise the first letter for display only
(standard, and the file's key is still what the column *is*). Recommendation: capitalise for display
where the name came from a format we recognise, leave verbatim where it came from the file's own
keys, and say which is which in `UI-DESIGN.md`.

## 15. Win32 menus cannot be dark-themed — **Platform**

Measured on this machine (Windows 11, 10.0.26200) on 2026-09-09: all five uxtheme dark-mode ordinals
resolve, `SetPreferredAppMode(ForceDark)` runs before any window or menu exists,
`RefreshImmersiveColorPolicyState` and `FlushMenuThemes` follow, `AllowDarkModeForWindow` is called
for the frame — and the menu bar and its popups are drawn light regardless. Windows' own
classic-menu applications behave the same way.

The only route left is owner-drawing the items and painting the bar behind them through undocumented
messages, which is this program drawing its own chrome again. **The owner's decision of 2026-09-09:
leave it, revisit one day, possibly never.**

---

## What this review did not cover

Honesty about scope: the pages read cover menus, toolbars, list views and the checklist. Not read,
and therefore not audited: the Wizards page against the format wizard's flow, the Error Messages and
Confirmations pages against our notices, the Accessibility page against the UIA provider, and the
Layout page against control spacing inside each dialog. Those are the next reading if this review is
to be complete rather than representative.

---

## What has been done since this review was written

**2026-09-09, the same day.**

- **1, 2 — `Settings`/`Preferences`.** The category is **Tools** and the item is **Options**, with no
  ellipsis. `Font…` moved there from Format, where §2.2 had put a second copy beside Preferences;
  one home, the standard one.
- **3 — context-menu accelerators.** Stripped from the grid, header and filter-row menus, and pinned
  by `no_context_menu_prints_a_shortcut_key`, which walks every context menu the program builds.
- **4 — `Go to line…` moved to Edit**, where the guide's standard menu puts it. `Select all` is
  deliberately **not** added: this program opens multi-gigabyte files, and a command whose obvious
  next step is Copy would offer to put ten gigabytes on the clipboard. Recorded as a considered
  exception rather than an oversight.
- **9 — the applications picker's commit row** is right-aligned: `Interleave`, `Separate windows`,
  `Cancel`.
- **10, 11 — the highlight rules editor is modal**, its commit row is right-aligned at the bottom,
  and the buttons are **Save** and **Cancel** — the closing button losing the access key it should
  never have had. The live preview survives, exactly as the owner said it would: a modal loop
  dispatches the owner's `WM_PAINT` like any other, so the log recolours under the box as a pattern
  is typed.

  **A defect found by driving it rather than reading it:** the proc still called `DestroyWindow` on
  its way out, which for a modal dialog destroys the window and leaves the modal loop running — the
  owner window stayed disabled and the application looked hung. `EndDialog` is what ends a modal
  dialog. Verified by asking Windows whether the owner is enabled before, during and after.

---

# Part two — the rest of the guide

**Part one read four pages.** The owner asked for the complete list, so the remaining twenty-eight
were read as well: the keyboard, accessibility, messages and notifications, progress, wizards,
window management, layout, fonts, colour, icons, text and tone, the standard dialogs, and
Microsoft's own top-violations checklist. Ten readers, one page group each, every finding quoting
the rule it is drawn from and naming the line of ours that breaks it.

**Eighty-eight findings: 58 *Fails*, 23 *Partly*, 6 *Passes*, 1 *Platform*.** They are listed in
full below. The list is deliberately unfiltered — it is the map, not the plan, and a good many of
its entries are a word in a status line.

## Three that are defects rather than infelicities

These came out of the audit and were confirmed by hand afterwards, because a finding that names a
line number can still be wrong about what the program does.

**`Ctrl+H` is advertised in three places and bound in none.** The register carries it, the Rules
menu prints it, the toolbar's tooltip repeats it, and a status notice tells the user to press it —
and `VK_H` appears nowhere in the shell. There is no accelerator table either, so nothing else can
be catching it. Pressing it does nothing, and this is the shortcut for the editor that was made
modal this morning. The guide also says not to reassign a standard key: `Ctrl+H` is Replace in the
standard Edit menu, so the binding to write is a different one.

**`F1` does nothing.** `VK_F1` appears nowhere. The Help menu's `Keyboard map` is the one help
surface this program has, and it is not on the key every Windows user tries first.

**The column header's context menu cannot be reached at all.** `Document::header_hit` opens with
`if self.header_ctl.is_some() { return None }` — correct in itself, since the real control owns its
own band — but the header context menu is built only under that branch, and the control exists for
every pane with columns. `Sort ascending`, `Sort descending`, `Top N…`, `Filter on <column>…` and
`Clear sort` therefore have no route by mouse; the `Shift+F10` anchor is hardcoded one pixel below
the header, so they have none by keyboard either. The header control has to answer `WM_CONTEXTMENU`
itself, mapping the point through `HDM_HITTEST`.

## The themes underneath the eighty-eight

- **The keyboard is the weakest surface.** Beyond the two dead keys above: the filter panel is drawn
  rather than built from controls, so it has no tab stops and `widget::Focus` has exactly one
  variant; column width and order are drag-only; `Ctrl+\` uses a character the guide names as
  locale-specific.
- **Accessibility is a stated gap, not an unknown one.** The provider deliberately excludes the grid
  — the log text every line of this program exists to show is invisible to a screen reader. Two
  *large* findings in the whole list, and this is one of them.
- **Status-bar text is where most of the small findings live**: sentence case, ending punctuation,
  symbols standing in for words, and faults that say what happened without saying what to do.
- **The dialogs are close.** Most findings are a label, an access key, a gap below the 3-DLU
  minimum, or a commit button whose verb should name what it commits.


### Fails (58)

| # | Surface | What the guide asks | What we do | The fix | Size |
|---|---|---|---|---|---|
| 1 | Rules menu — Highlight rules (Ctrl+H) | Document all shortcut keys. Document shortcuts in menu bar items, toolbar tooltips, and a single Help article that documents all shortcut keys used. —… | Ctrl+H is documented in four places and bound in none. main.rs:3537 registers (Command::EditRules, "Highlight rules…", "Ctrl+H"); menubar.rs:565 draws `&Highlight rules…\tCtrl+H`; the toolbar tip is g… | Bind the key — and bind it to Ctrl+K or Ctrl+R, not Ctrl+H, which the standard Edit menu reserves for Replace…. Update the register row, the menu and the notice text together, since all three read the… | small |
| 2 | Help menu — F1 | For well-known shortcut keys, use the standard assignments. | The standard menu bar gives Help as "<program name> help F1 <separator> About <program name>". Tailhawk's Help menu is menubar.rs:602 Item::command("&Keyboard map", "", ID_KEYMAP) and menubar.rs:604 I… | Bind F1 to ID_KEYMAP in the WM_KEYDOWN arm and print `F1` beside `&Keyboard map` in the menu, so the one Help surface the program has is reachable by the key every Windows user tries first. | small |
| 3 | Column header context menu | Ensuring keyboard access (for example, a tab stop for every interactive control) so that users can accomplish the same things in your program with eit… | menubar::header_context (menubar.rs:187-219) carries `Sort &ascending`, `Sort &descending`, `Top &N…`, `&Filter on {title}…` and `Clear s&ort`. It has exactly one construction site — context_menu's Un… | Handle WM_CONTEXTMENU on the header control (or NM_RCLICK in header.rs) and map the point through HDM_HITTEST; make the Shift+F10 anchor follow the focused surface instead of a fixed grid coordinate. | small |
| 4 | Filter panel | Assign tabs stops to all interactive controls, including read-only edit boxes. | The whole panel is painted glyphs with click rectangles. `Clear all`, `Remove`, `Edit…` and `Add…` are drawn by controls::button into panel_hits (main.rs:551-563); each chip row's `[x]` mark, `+`/`−`… | Rebuild the panel's title row as real Button controls and the chip list as a SysListView32 with check boxes inside a child window with WS_TABSTOP, so tab order, access keys and accessibility come from… | large |
| 5 | Log grid (WM_GETOBJECT provider) | Enabling programmatic access to all UI elements and text (for example, using the Active Accessibility COM interface, IAccessible). | The provider deliberately excludes the grid. main.rs:6845 — "The grid's text is `[v2]`'s virtualised text provider and is not here." uia::children (main.rs:6901-6917) returns only Kind::Tab(i), Kind::… | Implement ITextProvider/ITextRangeProvider over the visible row window (or IGridProvider + IValueProvider per cell once columns exist), backed by the same RowSource::row_text the painter reads, so wha… | large |
| 6 | Column header (SysHeader32) | Don't make double-clicking and dragging the only way to perform an action. These can be difficult movements for some users. | Column width and column order are drag-only. header::request_of maps HDN_ENDTRACKW → Request::Resize and HDN_ENDDRAG → Request::Reorder (header.rs:153-157), and the control is created with HDS_DRAGDRO… | Add the `Select columns…` dialog the earlier review's finding 8 already calls for and give it Move up / Move down plus a width field, so order and width have a non-drag route; the double-click can sta… | medium |
| 7 | View ▸ Split pane (Ctrl+\) | Don't use the following characters for shortcut keys: @ $ {} [] \ ~ / ^ ' < >. These characters require different key combinations across languages or… | main.rs:3510 registers (Command::Split, "Split the pane / unsplit", "Ctrl+\\") and menubar.rs:509 draws `&Split pane\tCtrl+\`. The handler is main.rs:5311, `if key == VK_OEM_5.0` — an OEM virtual key… | Move Split pane to Ctrl+J or Ctrl+K — both on the guide's recommended list and both unused here — and match on VK_J/VK_K instead of VK_OEM_5. | small |
| 8 | File ▸ Open remote source, and Format ▸ Log format | Assign access keys to all menu items. No exceptions. — and: For dynamic menu items (such as recently used files), assign access keys numerically. | Two submenus are built from runtime data and neither row carries a mnemonic. menubar.rs:358: `.map(/(n, name)/ Item::command(name, "", ID_SOURCE_BASE + n as u32))` — the source's configured name, verb… | Number these rows the way recent_label already numbers recent files, by prefixing `&{n+1}` in both closures, and extend every_choosable_item_marks_a_mnemonic to build a bar with sources and format row… | small |
| 9 | Applications picker — the "Separate windows" option | A top-level window has no owner window and is displayed on the taskbar. Examples: application windows. | This is the page's own definition of the word the button uses, and the option does not produce one. dialog.rs:993 labels the radio `&Separate windows`; main.rs:7987 handles Pick::Separate by looping t… | Rename the option, its doc comments and apps.rs's refusal text to "Separate tabs" (MAX_SEPARATE's eight-tail argument is unchanged), or make it genuinely open top-level windows. The label is the cheap… | small |
| 10 | Remote-source failure notices — the one message that gives d… | Accurate. Users should feel reassured that the information is technically accurate. If the information isn't accurate, users' experience with that spe… | Confirmed still present at pull.rs:82 — PullFault::NoSecret prints "No secret is stored for this source — open Settings ▸ Remote sources and paste it." There is no longer a Settings menu: menubar.rs:5… | Change the message to `… — open Tools ▸ Remote sources and paste it.` and widen the test at pull.rs:390 to assert the menu path, not just the item name. | small |
| 11 | Status bar — application picker fallback | Don't give possibly unlikely problems, causes, or solutions in an attempt to be specific. Don't provide a problem, cause, or solution unless it is lik… | main.rs:7869-7877 turns both outcomes of the label call into Fetched::NoApps — the benign one ("Loki lists no applications for this source") and any PullFault (`Err(why) => { let why = why.to_string()… | Split the two cases at main.rs:7869: keep NoApps for the empty-list result, and route a PullFault to Fetched::Failed so it is reported once, without the "opening all of it" promise, and without a seco… | small |
| 12 | Status bar — remote tail stops | A solution. Provides a solution so that users can fix the problem. … Actionable. Users should either perform an action or change their behavior as the… | tail.rs:359-363: on a write failure the worker sends "{name}: could not write new records: {why}" and then `return;` — the tail thread ends permanently and nothing restarts it. The message does not sa… | Say what happened and what to do — "{name}: this source has stopped following because new records could not be written ({why}). Close and reopen the source to start again." — and have the worker signa… | medium |
| 13 | Status bar — every notice | Relevant. The message presents a problem that users care about. … Rare. Displayed infrequently. Frequently displayed error messages are a sign of bad… | Shell::notice: Option<String> is declared at main.rs:4313 and initialised `notice: None` at main.rs:10807. It is assigned at thirteen sites (main.rs:4556, 5110, 5373, 5554, 6016, 6039, 6082, 6131, 614… | Clear notice when the condition it describes ends — on a successful poll_fetch answer for the same source, and on the next successful tail poll — or give it a timestamp and let status_text drop it aft… | small |
| 14 | Remote sources dialog — OK with a fault | When a user input problem is reported, set input focus to the first control with the incorrect data. Scroll the control into view if necessary. If the… | dialog.rs:851-856: on OK, `if let Some(fault) = data.editor.fault() { set_dlg_text(hdlg, ID_S_FAULT, fault); return 1; }`. The same call already ran on every keystroke at dialog.rs:820, so the static… | Have fault() return the offending row index with the text, select that row in the list, SetFocus the first bad field and EM_SETSEL its contents, and prefix the message with the source's name ("live: t… | medium |
| 15 | Export progress (status bar) | Clearly indicate real progress. The progress bar must advance if progress is being made. | The export worker sends running totals every 20,000 rows — export.rs:178 `let _ = tx.send(Update::Progress { written, scanned });` with REPORT_EVERY = 20_000 (export.rs:109-110) — and the shell throws… | Bind the Progress arm's `written` into a live counter on Tee (leg-so-far plus completed legs) so the line advances as rows are written. | small |
| 16 | Export completion notice (status bar) | Provide useful progress details. Provide additional progress information, but only if users can do something with it. Make sure the text is displayed… | main.rs:1560 composes the only confirmation an export finished — "✓ exported {} lines → {} — " — and it survives exactly one tick: poll_tee returns early while `changed` is true on the tick that set `… | Hold the finished Tee for a readable interval (a few seconds) before clearing it, or keep the "✓ exported N lines → file" text in Shell::notice, which already persists beside the document. | small |
| 17 | Opening / indexing a file | Will the operation complete in about five seconds or less? If so, use an activity indicator instead, because displaying a progress bar for such a shor… | main.rs:5467-5468 sets the status to format!("opening {}…", path.display()) and hands the work to spawn_open (main.rs:10547-10555), a bare std::thread over an mpsc channel that carries a single Result… | Set IDC_APPSTARTING while a read is outstanding, and give LogSet::open a progress sender (bytes indexed of file size) so that after five seconds the status bar can show a determinate bar instead of a… | medium |
| 18 | Main window — restoring saved placement | If the current monitor configuration prevents displaying a window using its last state: Try to display the window using its last monitor. If the windo… | main.rs:10723 reads `let placement = settings.window;` and main.rs:10842-10845 passes w.x, w.y, w.width.max(320), w.height.max(200) straight into CreateWindowExW. The only validation anywhere is setti… | Apply the saved rect with SetWindowPlacement after creation instead of through CreateWindowExW: pick the monitor with MonitorFromRect(MONITOR_DEFAULTTONEAREST), shrink the rect to rcWork, then slide i… | small |
| 19 | Main window — default size on first run | May be optimized for higher resolutions, but sized down as needed at display time to the actual screen resolution. | main.rs:10844-10845 hands CreateWindowExW a hard-coded size when there is no saved placement: placement.map_or(1280, /w/ w.width.max(320)) and placement.map_or(800, /w/ w.height.max(200)). Nothing que… | Before CreateWindowExW, take the target monitor's rcWork (MonitorFromPoint + GetMonitorInfoW) and its DPI (GetDpiForMonitor), scale the 1280x800 default by dpi/96, then clamp width and height to rcWor… | small |
| 20 | Main window — minimum size | Should set a minimum window size if there is a size below which the content is no longer usable. For resizable controls, set minimum resizable element… | There is no WM_GETMINMAXINFO handler — confirmed by grep across crates/tailhawk/src for both GETMINMAXINFO and the message value 0x0024, which return nothing; the window proc's match arms end at WM_DE… | Handle WM_GETMINMAXINFO and set ptMinTrackSize from a pure function of the chrome band heights, the toolbar's minimum width and a few grid rows, scaled by GetDpiForWindow — the same shape as the exist… | small |
| 21 | Theme switching (whole window, including High Contrast) | Handle theme changes. Theme changes are handled automatically by windows with standard window frames and common controls. Windows with custom window f… | The palette is decided once, at start-up: theme_for (main.rs:8898-8904) calls theme::chosen(name, hc.is_some(), system_uses_light_theme()) and the result goes into the process-wide slot at theme.rs:34… | Add a WM_THEMECHANGED arm and widen the WM_SETTINGCHANGE arm to also act on "HighContrast"/"ImmersiveColorSet" and on wparam == SPI_SETHIGHCONTRAST; each should re-run theme_for, set_theme, re-apply c… | medium |
| 22 | Status bar — Loki response faults (WireFault) | Messages intended to help the program's developers find bugs are left in the release version of the program. These error messages have no meaning or v… | pull.rs:102: `PullFault::Wire(why) => write!(f, "Loki's answer could not be read: {why}")`, where why is WireFault's Display at lokiwire.rs:265: `WireFault::Malformed { at, wanted } => write!(f, "expe… | Give the user one sentence with a cause and a remedy ("Loki's answer could not be read. The source may be answering with something other than a log query — check the query in Tools ▸ Remote sources.")… | medium |
| 23 | Status bar — remote source faults (PullFault transport arms) | Avoid creating troubleshooting problems. Don't rely on a single error message to report a problem with several different detectable causes. … Use a di… | pull.rs:84 and :94 discard the cause they are holding: `PullFault::TokenTransport(_) => f.write_str("Could not reach the token endpoint.")` and `PullFault::QueryTransport(_) => f.write_str("Could not… | Change the arms to `PullFault::QueryTransport(why) => write!(f, "Could not reach Loki: {why}")` (and the same for the token endpoint), which is exactly what the Wire arm at pull.rs:102 already does. G… | small |
| 24 | Status bar — remote source faults (PullFault::Label) | Don't use phrasing that blames the user or implies user error. | pull.rs:103: `PullFault::Label(_) => f.write_str("That is not a label name Loki would recognise.")`. The label name is never the user's: it is the constant APP_LABEL: &str = "app" at main.rs:7822, pas… | This branch is unreachable except through a Tailhawk bug, so it should not be a user-facing sentence at all — send it to header::trace and let the label call fall through to the "opening all of it" pa… | small |
| 25 | Status bar — renderer and split-pane faults | Use user-centered explanations. Describe the problem in terms of user actions or goals, not in terms of what the software is unhappy with. Use languag… | main.rs:5110: `self.notice = Some(format!("paint: {e}"));` and main.rs:5554: `Err(e) => self.notice = Some(format!("split: {e}"))`. Both prefixes are internal function names. `e` is tailhawk_core::Err… | Replace with user-facing sentences that name the goal, not the routine: "Tailhawk could not draw this frame; the display device is being rebuilt." and "This tab could not be opened a second time: {rea… | small |
| 26 | Status bar — saving highlight rules | Whenever possible, propose a practical, helpful solution so users can fix the problem. … Always provide a text description of the problem and solution… | main.rs:6037-6040: `if let Err(e) = std::fs::write(&target, self.rules_editor.to_toml()) { self.notice = Some(format!("rules not saved: {e}")); return; }`. `e` is std::io::Error, whose Display on Wind… | Name the file and give the remedy: format!("The highlight rules could not be saved to \"{}\" — {}. Check that the file is not read-only.", target.display(), e), with the bare OS text kept as the cause… | small |
| 27 | Status bar — remote tail write faults | Illegal, invalid, bad (use incorrect instead) | stdin.rs:825: `return Err(Error(format!("{}: invalid handle", path.display())));` in create_locked_down (declared stdin.rs:783). That function is called by SpillSet::append at stdin.rs:484 when a spil… | Reword the three stdin.rs strings away from the reserved words and away from Win32 vocabulary — "the temporary file could not be opened" in place of "invalid handle", and drop the "spill security desc… | small |
| 28 | Export / tee status bar text | Accurate. Users should feel reassured that the information is technically accurate. | Confirmed at main.rs:1559-1562: format!("⇥ saving → {} ({} lines) — ", t.name(), t.written), format!("✓ exported {} lines → {} — ", t.written, t.name()) and format!("⇥ exporting → {} ({} lines) — ", …… | Give the three tee fragments the singular arm Finder::status_line already has, and update the assertion at main.rs:12162. | small |
| 29 | UIA provider (events and focus) | Ensuring programmatic events are triggered by all UI activities (for example, focus events for all UI activities involving focus movement). | Element::is_focusable returns a hardcoded false (main.rs:7012-7014), has_focus returns false (main.rs:7016-7018), IRawElementProviderFragmentRoot::GetFocus returns none() (main.rs:7282-7284), and SetF… | Raise UIA_AutomationPropertyChangedEventId for ValueValue when the status text changes and UIA_StructureChangedEventId when children() gains or loses a chip; once the grid is exposed, raise a focus ev… | medium |
| 30 | UIA provider (tabs and status bar) | Preferring common controls to custom controls, because common controls have already implemented the Windows accessibility APIs. | uia::children still emits Kind::Tab(i) and Kind::Status (main.rs:6901-6917) for surfaces that are now real Win32 controls with their own accessibility: SysTabControl32 (tabstrip.rs, held as Shell::tab… | Drop Kind::Tab and Kind::Status from children() and delete their branches; let the two common controls answer for themselves and keep the custom provider for the surfaces that have no HWND. | small |
| 31 | Define Format wizard and Import Layout dialog — window frame | Use resizable windows for a wizard that can benefit from more screen space but doesn't require it. Assign an appropriate minimum size. Resizable windo… | Both are fixed size. template (dialog.rs:229-232) sets only WS_POPUP / WS_CAPTION / WS_SYSMENU / DS_MODALFRAME / DS_SETFONT / DS_CENTER — no WS_THICKFRAME, no WS_MAXIMIZEBOX — and the two are opened a… | Add WS_THICKFRAME to both templates and a WM_SIZE handler that keeps the buttons on the bottom-right and gives the growth to the two list views and the Pattern box, with a minimum size (WM_GETMINMAXIN… | medium |
| 32 | Remote sources dialog — jargon and missing main instruction | Don't invent words or apply new meanings to standard words. Assume that users are more familiar with a word's established meaning than with a special… | dialog.rs:626-641 — seven labelled fields, &Name:, &URL:, &Token URL:, &Client id:, &Scope:, &Query:, S&ecret:. Four of them (Token URL, Client id, Scope, Query) are OAuth and LogQL terms with no in-c… | Add a main instruction across the top ("Add a Loki server Tailhawk can tail") and a supplemental line under the credential group naming where the three OAuth values come from. Client id should also re… | medium |
| 33 | Define Format dialog and Import Layout dialog — top of the c… | Every wizard page must have a main instruction. — and, from the dialog-box page: Omit control labels that restate the main instruction. | Neither dialog has one. The first item in format_dialog_items (dialog.rs:3103-3108) is a control label — "&Example line — select in it to set a field's bounds:" — sitting at (7, 7, 300, 8), i.e. the l… | Add a single-sentence main instruction static across the top of each dialog above the existing labels — for example "Mark the parts of this line that Tailhawk should read as fields." and "Paste the la… | small |
| 34 | Import Layout dialog — the "Recognised as:" readout | One culprit in this excess is redundancy. Because of templates used in early wizard design, the same language might appear in multiple locations on a… | IMPORT_HINT (dialog.rs:411-412) is "paste a layout from your logging config — Serilog outputTemplate, NLog layout, log4net pattern", and recognised_label (dialog.rs:416-419) prints it into the ID_I_RE… | Leave ID_I_RECOGNISED empty when nothing is pasted (or word it as a value, "Nothing pasted yet"), move the language list ("Serilog, NLog, log4net or Logback") into the main instruction added above, an… | small |
| 35 | Highlight rules editor and Define Format wizard — the Remove… | Don't disable a control with input focus. Doing so may prevent the window from receiving keyboard input. Instead, before disabling a control with inpu… | Both dialogs grey the button the user just pressed. In the rules editor, (ID_R_REMOVE, BN_CLICKED) at dialog.rs:1846-1851 removes the rule and calls rules_refresh, which reaches rules_show_selected (d… | In both handlers, move focus before the refresh — SetFocus(GetDlgItem(hdlg, ID_R_LIST)) / ID_W_FIELDS, the control the removal leaves the user working in — so the disable never lands on the focused bu… | small |
| 36 | In-place fault lines — Remote sources, both wizards, and the… | However, in-place error messages should use a small error icon (16x16 pixel) to clearly identify them as error messages. — and, from the warnings page… | Four in-place fault lines, all bare text statics with no icon in the DLGTEMPLATE: ID_S_FAULT at dialog.rs:642, Item::new(Class::Static, "", ID_S_FAULT, (7, 220, 340, 9), 0) — full width, below every f… | Add a 16×16 SS_ICON static immediately left of each fault line, loaded with IDI_ERROR (IDI_WARNING for the Filter dialog's unknown-column case) and shown only when the line has text; move each line's… | medium |
| 37 | Dialogs — initial placement (Find, Highlight rules, Remote s… | If a window is an owned window, initially display it "centered" on top of the owner window. For subsequent display, consider displaying it in its last… | Every dialog in the program is built by the one template() in dialog.rs:218, whose style word at line 230 is WS_POPUP / WS_CAPTION / WS_SYSMENU / DS_MODALFRAME / DS_SETFONT / DS_CENTER (DS_CENTER = 0x… | Drop DS_CENTER from template() and position on WM_INITDIALOG: GetWindowRect the owner, place the dialog with 45% of the slack above and 55% below, clamp to the owner's monitor rcWork. One pure fn cent… | small |
| 38 | Modal dialogs (DLGTEMPLATE font) | For Segoe UI, use a 9 point font size or larger. The Segoe UI font is optimized for these sizes, so avoid using smaller sizes. … Win32 or WinForms / W… | Confirmed in the current tree at dialog.rs:241-242: `t.push(8); // point size, then the face DS_SETFONT promises` then `push_wsz(&mut t, "MS Shell Dlg");`. The style word at dialog.rs:228-231 is WS_PO… | Write the template with 9 and "Segoe UI", and update the assertion at dialog.rs:4459 to match. Better still, keep the template font only as a layout metric and send WM_SETFONT with tabstrip::shell_fon… | medium |
| 39 | Toolbar icon glyphs | Don't mix color types. That is, always match theme colors with their associated theme colors, system colors with their associated system colors, and h… | toolbar.rs:688-691 tints the glyph bitmaps with the hardwired palette's ink: `let ink = tailhawk_core::theme::theme().ink;` folded to 0x00RRGGBB and passed to icon_list(px, colour), which premultiplie… | Take the tint from the same authority that paints the band. Either open the toolbar's theme with OpenThemeData(hwnd, L"Toolbar") and GetThemeColor(TP_BUTTON, TS_NORMAL, TMT_TEXTCOLOR) — then the glyph… | medium |
| 40 | Toolbar tooltips | As with normal labels, keep tooltips brief typically five words or less but prefer specific labels over vague ones. — and: Add an ellipsis if the labe… | On an icon-only bar the tooltip is the only text a user gets, and tip_for (toolbar.rs:170-179) builds it from ToolButton::label, taking only the keys from Command::LISTED. Four of the resulting tips a… | Have tip_for take the name from Command::LISTED rather than ToolButton::label, trimming the long register entries to five words or fewer — "Open…", "Highlight rules…", "Define format…", "Export visibl… | small |
| 41 | Derived identifier colours (highlight.rs / theme.rs) | Assign colors by default that are easy to distinguish. Generally, colors are easy to distinguish if they are far apart from each other in the HSL/HSV… | derived_colour (highlight.rs:102-110) folds an FNV-1a hash onto theme().identifiers, eight entries at theme.rs:154-163 (dark) and theme.rs:228-237 (light). The doc-comment at highlight.rs:89-91 claims… | Move dark entry 6 and light entry 5 to fill the gaps in each ring — dark has nothing between 272° and 340°, light nothing between 253° and 307°. Then add a test beside the existing one asserting a min… | small |
| 42 | Derived identifier colours (customisation and labelling) | Allow users to customize these color assignments because color choice is subjective and a personal preference. If there are many coordinated colors, a… | This is data colouring in the guide's exact sense — "assign color to data to help users differentiate it" — and neither affordance exists. The assignment is a fixed hash onto a fixed array: derived_co… | Add the eight identifier colours to the Highlight rules editor as an editable swatch row persisted through settings.rs, and let a derived assignment be pinned and named — a small table of text → colou… | medium |
| 43 | Application icon (assets/tailhawk.ico) | Icon (.ico format) files must contain the 4- and 8-bit versions, as well as the 24-bit + alpha. … Icon files require 8-bit and 4-bit palette versions… | Parsing the ICONDIR of assets/tailhawk.ico (11,181 bytes, 8 entries): every entry reports bColorCount = 0 and wBitCount = 32. There is no 8-bit and no 4-bit cut of any size. tools/make-icon.ps1 builds… | Extend tools/make-icon.ps1 to emit 8-bit (256-colour) and 4-bit (Windows 16-colour) versions of at least the 16, 32 and 48 px cuts alongside the 32-bit ones, and re-run it. The guide notes these need… | medium |
| 44 | Options dialog (Tools ▸ Options) | Dialog boxes: Display the command, feature, or program from which the dialog box came. Don't use the title to explain the dialog box's purpose that's… | The menu item is now &Options (menubar.rs:598, under &Tools at :588) but the dialog it opens is still titled "Preferences" — dialog.rs:2375 template("Preferences", 217, 86, &items). The title names a… | Change the template title at dialog.rs:2375 to "Options", and the test fixture at :4450/:4454 with it. | small |
| 45 | Dialog title bars (all nine) | Use title-style capitalization for titles, sentence-style capitalization for all other UI elements. | dialog.rs — four titles are title-style: :3215 "Define Format", :3659 "Import Layout", :3060/:3062 "Add Filter" / "Edit Filter". Five are sentence-style: :901 "Remote sources", :1367 "Highlight rules"… | Pick one and apply it to all nine. A title bar is a title, so title-style is the guideline's answer: Define Format, Import Layout, Add Filter, Remote Sources, Highlight Rules, Go To Line, Keyboard Map… | small |
| 46 | Find dialog — button labels | Use title-style capitalization for titles, sentence-style capitalization for all other UI elements. Doing so is more appropriate for the Windows tone.… | dialog.rs:2588 "&Find Next" and :2595 "Find &Previous" are title-style, in the same dialog as :2559 "Match &case", :2573 "Regular e&xpression" and :2580 "W&rap around", which are sentence-style. The l… | Rename the two buttons &Find next and Find &previous so the whole dialog is sentence-style, matching the menu bar's own Find &next / Find previo&us (menubar.rs:404, :406). | small |
| 47 | File menu — Close Tab | Use title-style capitalization for titles, sentence-style capitalization for all other UI elements. | menubar.rs:366 — cmd("&Close Tab", "Ctrl+W", Command::CloseTab). It is the only title-cased item on the bar. Every other multi-word item is sentence-style: Open re&mote source (:358), &Export view… (:… | &Close tab. The comment at menubar.rs:773 that refers to it by name should follow. | small |
| 48 | Status bar and Find dialog static text — capitalisation | Use sentence-style capitalization. — sentence-style still means an initial capital. | The same status line mixes two registers. Sentence-capitalised: main.rs:4556 "The request to Loki did not finish.", main.rs:8064 "Could not create a temporary file for the records.", and every PullFau… | Capitalise the first word of every one of these strings, and drop the "paint:"/"split:"/"rules not saved:" prefixes as part of the rewrites above. | small |
| 49 | Status bar (Document::describe) — symbols standing in for wo… | Don't use symbols as a substitute for simple words. — Incorrect: users & computers / users + computers / domain / workgroup / # of users. Correct: use… | main.rs:1559 "⇥ saving → {name} ({n} lines)", :1560 "✓ exported {n} lines → {name}", :1562 "⇥ exporting → …" — the arrow stands in for the word "to". Alongside it: :1594 "● following", :1596 "‖ paused… | Write the words: "Saving to out.txt (3 lines)", "Exported 3 lines to out.txt", "Following", "Paused — press Ctrl+End to follow". If a state marker is wanted, use a status-bar part with an icon rather… | medium |
| 50 | Add / Edit Filter dialog — commit button | Prefer specific labels over generic ones. Ideally users shouldn't have to read anything else to understand the label. — and, from the commit-button ta… | dialog.rs:3040 — the Add/Edit Filter dialog commits with a bare "OK". It is a window for one specific task; its own title (dialog.rs:3060-3062) already says which task. The program gets this right els… | Label the button &Add when the title is Add Filter and &Save when it is Edit Filter — the title variable at dialog.rs:3059 already distinguishes the two cases. | small |
| 51 | Define Format and Import Layout dialogs — the Save button's… | Don't assign access keys to: OK, Cancel, and Close buttons. Enter and Esc are used for their access keys. However, always assign an access key to a co… | Both wizards give their commit button the label Save on IDOK with no mnemonic: dialog.rs:3178-3184 (Item::new(Class::Button, "Save", IDOK, (W_RIGHT - 110, 286, 50, 14), WS_TABSTOP / BS_DEFPUSHBUTTON))… | Label both &Save, which is also the standard table's assignment for Save. Check against &Save as: (dialog.rs:3154 / 3583), which already owns S in each — so give the commit button &Save and move the f… | small |
| 52 | Grid — Spacebar, B and the whole navigation map | Document all shortcut keys. Document shortcuts in menu bar items, toolbar tooltips, and a single Help article that documents all shortcut keys used. D… | main.rs:9567 maps NEXT / SPACE => Some(Navigate::ByPages(1)) and main.rs:9569 maps bare B => Some(Navigate::ByPages(-1)), the comment saying "b for page-up is less muscle memory". Neither key is in Co… | Add a Navigation section to the keyboard map: pass keymap_sheet_of a second, hand-written run of rows for the navigation keys (Up/Down, Page Up/Page Down, spacebar, B, Home/End, Ctrl+Home/Ctrl+End) al… | medium |
| 53 | View ▸ Back / Forward, and the Keyboard map | For arrow keys, use left arrow, right arrow, up arrow, and down arrow. Don't use graphic labels for the arrow keys. | main.rs:3481 and :3483 register the two view-history commands with the key strings "Alt+←" and "Alt+→", and menubar.rs:521-522 draws them into the View menu unchanged (&Back\tAlt+←, F&orward\tAlt+→).… | Change the two register strings to "Alt+Left arrow" and "Alt+Right arrow". One edit; the menu, the tooltip and the keyboard map all read that register. | small |
| 54 | Rules menu — shortcut column for Colour-label lines | Ellipses mean incompleteness. Use ellipses in UI text as follows: Commands: Indicate that a command needs additional information … Data: Indicate that… | menubar.rs:574 — cmd("&Colour-label lines", "Ctrl+Shift+1…9", Command::Label(1)). The ellipsis is being used as a range operator meaning "through", which is none of the three sanctioned uses, and the… | Show one real combination in the column — Ctrl+Shift+1 — since that is what Command::Label(1) invokes, and leave the range to the keyboard map, which has room to explain it. | small |
| 55 | Status bar (filter and sort progress) | Otherwise, if the window has a status bar, display the modeless progress in the status bar. Put any corresponding text to its left in the status bar.… | Filtering and sorting both compute a genuine percentage and then print it as words on the end of an already long sentence: main.rs:3157 format!("↕ sorting by {column}… {pct}%") and main.rs:3220 text.p… | Give the status bar a second part, create a msctls_progress32 child of it positioned from SB_GETRECT, and drive it from the filter/sort/export percentages; drop the "%" from the text once the bar carr… | medium |
| 56 | Ending punctuation on status text | Periods — Don't place at the end of control labels, main instructions, or Help links. Place at the end of supplemental instructions, supplemental expl… | Two lists apply the rule inconsistently. filter_status (dialog.rs:2674-2694): "Type a filter, or build one above." (period), "Not a valid filter: {e}" (none), the unknown-column warning (period), and… | Take the period off "OK." (or replace it with something that says what is valid, e.g. "Filter is valid"), put one on "Not a valid filter: {e}", and end pull.rs:102 with a period after {why}. | small |
| 57 | Dialog layout — control gaps below the 3 DLU minimum | If controls aren't touching, have at least 3 DLUs (5 relative pixels) of space between them. Otherwise, users may click on inactive space between the… | Nine pairs sit at 2 DLU. Horizontal: dialog.rs:596 the Remote sources label closure `(7, y + 2, 48, 8)` spans x 7..55 against dialog.rs:602's field at x 57 — 2 DLU on all seven rows (Name, URL, Token… | Move the sources field column from x 57 to 58, the filter operator combo from 188 to 189, the Format name edit from 146 to 147, and raise each of the six wizard labels by 1 DLU (y 7→6, 34→33, 142→141;… | small |
| 58 | Command buttons across Remote sources, Applications picker a… | For each window make the command buttons the same width. If that's impractical, limit the number of different widths for command buttons with text lab… | Three surfaces break the two-width cap. Remote sources: dialog.rs:647 "OK", 1, (293, 232, 54, 14) and dialog.rs:650 "Cancel", 2, (353, 232, 60, 14) — two adjacent commit buttons at 54 and 60 DLU, beca… | Remote sources: make both 60 DLU — OK at (287, 232, 60, 14), Cancel unchanged. Applications picker: when the three commit buttons are gathered into the right-aligned row the earlier review's finding 9… | small |

### Partly (23)

| # | Surface | What the guide asks | What we do | The fix | Size |
|---|---|---|---|---|---|
| 59 | Key map — Esc | Don't assign different meanings to well-known shortcut keys. Because they are memorized, inconsistent meanings for well-known shortcuts are frustratin… | Three separate register rows claim Esc: main.rs:3454 (Command::ClearSearch, "Clear search", "Esc"), main.rs:3458 (Command::ClearFilter, "Clear filters", "Esc") and main.rs:3552-3556 (Command::ClearSor… | Give Esc one documented meaning — one register row worded as what it does, e.g. "Undo the narrowing: the search, then the sort, then the filters" — and blank the key on the other two rows, which keeps… | small |
| 60 | Colour labels (Ctrl+Shift+1…9) | Never rely on color alone to convey meaning. Use color only as a means of reinforcing the meaning provided by text, design, location, or sound. | A label is a whole-line background fill and nothing else: Rule::new(name, pattern).bg(theme().labels[usize::from(n) - 1]).whole_line() (main.rs:2247-2254). Nine labels, nine hues, no glyph, no gutter… | Carry the label number as a gutter glyph through RowSource::row_mark/row_glyph so 1…9 reads without colour, and disable the label commands under High Contrast rather than letting them appear to act —… | medium |
| 61 | Search highlighting — which match you are standing on | Never use color as a primary method of communication, but as a secondary method to reinforce meaning visually. | Which match the user is standing on is carried by background hue alone. Dark theme: match_bg: [0.36, 0.29, 0.06, 1.0] against current_match_bg: [0.95, 0.62, 0.16, 1.0] (theme.rs:119-120); light theme:… | Give the current match an ordinal in the status bar ("match 3 of 41") and a shape the High Contrast palette keeps — a one-pixel box in t.caret around the current run — so the answer survives both colo… | small |
| 62 | Highlight rules editor — closing with unsaved rules | Use "Save changes" confirmations only when there are significant changes. Don't confirm changes that weren't directly made by the user, such as automa… | main.rs:6014-6018: `fn close_rules_editor(&mut self) { if self.rules_editor.is_dirty() { self.notice = Some("rules closed unsaved — Ctrl+H, Ctrl+S to save".to_owned()); } self.rules_editor.close(); …… | Ask before discarding: a task dialog with "Save" / "Don't save" / "Cancel" when is_dirty() and the editor is closed by any route. | medium |
| 63 | Define Format wizard — Cancel | Don't ask users to confirm whether they really intend to cancel. Doing so can be annoying. Exceptions: … The action may result in a significant loss o… | Cancel and Esc call EndDialog(hdlg, 0) unconditionally (dialog.rs:3976-3978) with no confirmation. The caller then runs close_wizard() (main.rs:8571), which drops the whole Wizard and posts "format wi… | Track whether the Wizard has been edited away from propose's opening proposal, and on Cancel with edits present show one "Discard this format definition?" confirmation. Leave the unedited case silent,… | small |
| 64 | Status bar — spill creation for a remote source | A cause. Explains why the problem occurred. A solution. Provides a solution so that users can fix the problem. | main.rs:8061-8075: `let Ok(spill) = tailhawk_core::stdin::SpillSet::create() else { set_notice(hwnd, "Could not create a temporary file for the records.".to_owned()); return; };` and, three lines on,… | Bind the error and append it as a cause clause, and name the directory: "The records could not be saved to a temporary file in {dir} — {why}." Free disk space is the likely remedy and is worth saying. | small |
| 65 | Define Format wizard and Import Layout dialog — the Save com… | Use specific commit button labels that make sense on their own and are a response to the main instruction. Ideally users shouldn't have to read anythi… | Both commit buttons read "Save" (dialog.rs:3180 and dialog.rs:3621). The button does two things, and names one: save_format (main.rs:6141-6183) writes the definition into tailhawk.formats.toml and the… | Label the commit button "Save and apply", or state the second effect in the main instruction added below; and relabel the "Save as:" edit "Format &name:" so it stops reading as a file destination. | small |
| 66 | Define Format wizard and Import Layout dialog — the fault li… | Use dialog boxes to give error messages that apply to the whole page, and result from clicking a commit button. — the same page also backs the inline… | Every refusal, including the ones raised by the commit button, is written inline to the ID_W_ERROR static by format_say (dialog.rs:3445-3450). The Save handler at dialog.rs:3954-3968 calls it with "na… | Route only the three IDOK-raised refusals through a task dialog or MessageBox owned by the dialog; leave the verb-button refusals on the inline fault line where they are correct. | small |
| 67 | Toolbar Export button glyph | Use established concepts where possible, to ensure consistency of meanings for the icon and its relevance to other uses. … Consider how the icon will… | toolbar.rs:207-208 defines EXPORT as U+E74E and names it in its own doc-comment: "Save — export writes a file." That code point is the Save glyph in Segoe Fluent Icons and Segoe MDL2 Assets — the flop… | Use a glyph whose established meaning is export rather than save: Share/OpenFile U+E8E5 (a page with an arrow leaving it) is the one the OPEN doc-comment at toolbar.rs:188-189 already identifies as re… | small |
| 68 | Severity gutter glyph | Providing alternatives to color (for example, icon differentiation or the use of sounds). — and: Never use color as a primary method of communication,… | RowSource::row_glyph is documented as "UI-DESIGN.md §11.2's severity glyph — the redundant non-colour channel beside the line number" (rows.rs:204-207), and Fatal/Error/Warn are properly differentiate… | Give Trace its own mark — a hollow '∘' against Debug's filled '·' works at 16 px and keeps the visual weight below Warn's '△' — or drop the glyph for Trace as Info already does, so absence is the chan… | small |
| 69 | Disabled control labels (shared controls module) | Disabled / 9 pt. dark gray (#323232) Segoe UI … Foreground color. Light gray indicates that text is disabled. | controls.rs:65-70 draws a disabled button's label in theme().field_hint. That value is [0.55, 0.57, 0.62] in the light theme (theme.rs:205) and [0.42, 0.45, 0.50] in the dark (theme.rs:131). Computed… | Give disabled text its own theme slot rather than borrowing field_hint, at a luminance near the guide's #323232 relative to the ground (roughly [0.20, 0.20, 0.20] on light, and its inverse on dark), a… | small |
| 70 | Application icon (assets/tailhawk.ico) — compression | Only a 32-bit copy of the 256x256 pixel image should be included, and only the 256x256 pixel image should be compressed to keep the file size down. Se… | All eight entries in assets/tailhawk.ico are PNG-compressed, not just the 256. Checking the first eight bytes at each entry's offset for the PNG signature 89 50 4E 47 0D 0A 1A 0A: 16x16 (397 bytes) PN… | Have tools/make-icon.ps1 write the seven sizes below 256 as uncompressed 32-bit BITMAPINFOHEADER + XOR/AND mask entries and keep PNG only for the 256. Windows Vista and later read PNG at any size so n… | small |
| 71 | Define Format wizard and Import Layout dialog — title bars | Use the exact command name for command-based names, but don't include the ellipsis if there is one. You can include the command's menu title if necess… | Import Layout is right: the menu item is "&Import layout…" (menubar.rs:557) and the title is "Import Layout" (dialog.rs:3659) — the command name, ellipsis dropped, no "Wizard" in it. Define Format is… | Title the first dialog "Define Format from a Line" (or rename the menu item to "&Define format…" and keep the title "Define Format"), and settle on one wording for ImportLayout across menubar.rs:557,… | small |
| 72 | Filter dialog — validation line (ID_F_STATUS) | Illegal, invalid, bad (use incorrect instead) | dialog.rs:2682: `Err(e) => format!("Not a valid filter: {e}")`. The forbidden word is "invalid"; "not a valid" is the same accusation split in two, in the same register. The rest of the line is good —… | Lead with the specific problem rather than the verdict: format!("{e}") alone already reads "expected a value (at 14)". If a frame is wanted, "This filter can't be read: {e}" avoids the word entirely. | small |
| 73 | Both wizards' Pattern box, and the rules editor's two colour… | Whenever possible, assign unique access keys to all interactive controls or their labels. Read-only text boxes are interactive controls (because users… | Four tab-stopped edit controls have no access key anywhere. dialog.rs:3138 label("Pattern:", (W_LEFT, 142, 40, 8)) over ID_W_PATTERN at dialog.rs:3139-3145 (WS_BORDER / WS_TABSTOP / ES_AUTOHSCROLL / E… | Give the Pattern label a mnemonic in both wizards (Pa&ttern: — P is free in each), and put a labelled static in front of each colour field in the rules editor (e.g. Te&xt colour: / Bac&kground colour:… | medium |
| 74 | Add/Edit Filter dialog — control groups | Assign tabs stops to all interactive controls, including read-only edit boxes. Exceptions: … Properly contain groups so that the arrow keys cycle both… | WS_GROUP is set exactly twice in the whole dialog: dialog.rs:2958 on ID_F_INCLUDE (opening the radio pair) and dialog.rs:2979 on ID_F_SCOPE (closing it). Nothing after that opens a new group, so the g… | Add WS_GROUP to the first control of each intended group — the ID_F_REGEX check-box pair and IDOK — so arrow-key travel stays inside the pair it belongs to and the commit row is its own group. | small |
| 75 | Define Format wizard — page structure | Resist the urge to bundle up multiple sub-tasks on a single page (the "burrito wizard") or to resort to tabs for presenting complex input requirements… | format_dialog_items (dialog.rs:3084-3190) emits 25 controls with 15 tab stops and 12 access keys (E, b, F, p, A, M, R, o, N, S, T, v), covering four distinct sub-tasks: carve the fields (example edit… | Wrap the three ungrouped regions in group boxes — "Fields", "Selected field", "Definition" — matching the existing Preview frame, so each sub-task reads as one thing. | medium |
| 76 | Search progress (status bar) | Use determinate progress bars for operations that require a bounded amount of time, even if that amount of time cannot be accurately predicted. Indete… | Finder::describe (main.rs:2899-2926) reports a running search two ways, neither determinate. With no match yet it appends only " — searching…" (main.rs:2908). The scan figure is added solely once matc… | Compute the same percentage the filter does from Finder::scanned against the index's line count, and show it whether or not any match has landed yet. | small |
| 77 | Go to line dialog — the out-of-range message | Omit needless words—don't use two or three words when one will do. — and: Because users often scan text, make every word count. Simple, concise senten… | dialog.rs:577 — format!("The file ends at line {last}, and Go will jump there. (Typed a line past the end of {total}.)"). Twenty words for one fact, and the parenthetical restates what the first claus… | "The file ends at line {last}. Go will jump there." Drop the parenthetical entirely — the range is in the label and the number is in the box. | small |
| 78 | Status bar — encoding disagreement marker | Choose object names and labels that clearly communicate and differentiate what the object does. Users shouldn't have to figure out what the object rea… | main.rs:1506 — when self.set.newest().disagreed() the description appends the literal " (mixed?)", which lands in the status bar directly after the encoding, e.g. `app.log: UTF-8 (mixed?)`. The word n… | Say what was found: "encoding varies between files" (for a set) or "some lines are not UTF-8" (for one file), without the question mark. | small |
| 79 | Welcome screen | User-focused. Write from the user's perspective and preferably from the perspective of what you can do for the user. Users should feel that they will… | main.rs:6786-6805 — four of the surface's lines, and both of the two most prominent, are about the program rather than about the user's work: "Watch your logs like a hawk" (:6786) and "Tailhawk never… | Keep "Drop a log file here, or press Ctrl+O" as the main instruction and drop the parenthetical. Keep the no-network claim — it is a real commitment SPEC §13.2 makes — but demote it to a footnote-weig… | small |
| 80 | Toolbar button labels | Use parallel grammatical constructions. Parallelism requires that words and phrases that have the same function have the same form. Use parallel langu… | toolbar.rs:127-161 — the nine labels are Open, Find, Filter, Follow, Collapse, Detail, Rules, Format, Export. Detail, Rules and Format are nouns; the other six are imperative verbs. This is the guide'… | Make the row one part of speech. The four toggles are states, so nouns suit them (Filters, Tail, Continuations, Detail); the five verbs stay verbs. Whichever way, Rules and Format should not sit besid… | small |
| 81 | Dialog layout — alignment and margin outliers | You know a layout is using grids effectively when: … There are no unnecessary vertical and horizontal alignment grid lines. — and: You know a layout h… | Left, top and right margins are a uniform 7 DLU across all ten dialogs, and four things break that grid. (1) Keyboard map: the read-only edit is (7, 7, 266, 168), right edge 273, against a 287-wide te… | Widen the Keyboard map edit to (7, 7, 273, 168); change the rules error line width from 400 to 406; move Go to x 71 and Cancel to x 126; set heights 252→253 (sources), 238→239 (apps), 94→95 (Find), 23… | small |

### Platform (1)

| # | Surface | What the guide asks | What we do | The fix | Size |
|---|---|---|---|---|---|
| 82 | Theme palette (theme.rs) — hardwired vs system colours | Whenever possible, choose colors by selecting the appropriate theme color or system color. By doing so, you can always respect users' color preference… | Theme::dark() (theme.rs:111-183) and Theme::light() (theme.rs:185-257) hardwire every one of roughly forty colours as fixed RGB literals — background, ink, chrome_bg, header_bg, field_bg, tab_bg, pane… | No API change is available; record it. The one gap that is not platform-imposed is that the light theme could derive background/ink/chrome_bg/field_bg from COLOR_WINDOW / COLOR_WINDOWTEXT / COLOR_BTNF… | small |

### Passes (6)

| # | Surface | What the guide asks | What we do | The fix | Size |
|---|---|---|---|---|---|
| 83 | Define Format wizard and Import Layout dialog — form of the… | Make your wizard a minimum of two pages. A one-page wizard should be redesigned as a dialog box instead. | Checked and right, and worth recording because the core module is still called `wizard` and the docs still call these §6.2 and §6.3 of a wizard. Both surfaces are real modal dialogs: DialogBoxIndirect… | No change. Keep the single-page dialog form; do not reintroduce pages. | small |
| 84 | Fault presentation — no modal dialogs | Does the problem relate to the status of a background task within a primary window? If so, consider showing the problem using a status bars. … General… | There is no MessageBox call anywhere in the program — grep -rn "MessageBox" crates/ returns nothing. The single TaskDialogIndirect (main.rs:8827) is the About box, with TDCBF_OK_BUTTON and nothing els… | Nothing. Worth recording because it is the guidance's own preference and the reason most of the fault findings above are status-bar findings rather than dialog findings — the two exceptions being the… | small |
| 85 | Define Format wizard and Import Layout dialog — Cancel seman… | Clicking Cancel means abandon all changes, cancel the task, close the window, and return the environment to its previous state, leaving no side effect… | Correct in a case that is easy to get wrong, because both procs mutate the caller's Wizard in place throughout the session — every verb writes through state.wizard, and import's "Use this one" does a… | No change. Compatible with the Cancel-confirmation finding above, which asks only that the loss be warned about, not that Cancel keep anything. | small |
| 86 | Native controls and D3D-drawn chrome (font selection) | The guidelines for making text accessible to users with disabilities or impairments can be boiled down to one simple rule: Respect the user's settings… | tabstrip.rs:542-581 — shell_font_for(dpi) reads SystemParametersInfoForDpi(SPI_GETNONCLIENTMETRICS, …) and creates from metrics.lfMessageFont; shell_font() is the same query at system DPI and is the d… | No change. Worth recording because it is the exact mechanism the colour half is missing — the same handler re-reads fonts and does not re-read the High Contrast palette — and because the dialog templa… | small |
| 87 | Filter panel chip marks | The best solution to the color interpretation and accessibility problems is to use color to visually reinforce the meaning of one of these primary met… | A chip's two states are text before they are colour. FilterRow carries mark: if chip.enabled { "[x]" } else { "[ ]" } and sign: '+' / '−' (filterpanel.rs:31-35), both asserted in a_rows_mark_sign_and_… | No change. Recorded because the reinforcing-rather-than-carrying pattern here is the one the severity glyph, the colour labels and the search highlight should copy. | small |
| 88 | Applications picker — main instruction | Express the main instruction in the form of an imperative direction or specific question. … Good main instructions communicate the user's objective ra… | dialog.rs:954 — the dialog opens with "Which applications should this source show?" above the list at :960. A specific question, in the user's terms, with the question mark the Punctuation section req… | No change. Use it as the pattern when adding main instructions to Remote sources, Define Format and Import Layout. | small |
