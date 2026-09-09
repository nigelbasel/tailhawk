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
| 1 | Menu bar | **Fails** | `Settings` is a menu category and `Preferences…` a menu item. The guide forbids both words by name. |
| 2 | Menu bar | **Fails** | `Preferences…` carries an ellipsis it should not have. |
| 3 | Context menu | **Fails** | The row context menu prints shortcut keys. Context menus never do. |
| 4 | Edit menu | **Partly** | No `Select all`; `Go to line` is under View where the standard puts it in Edit. |
| 5 | Toolbar | **Fails** | No `Open remote source` button, though it is among the most used commands. |
| 6 | Toolbar | **Fails** | `Collapse`'s icon is not self-explanatory — the owner could not tell what it was. |
| 7 | Toolbar | **Partly** | Right-clicking the toolbar does nothing; a toolbar with options owes a context menu. |
| 8 | Column header | **Fails** | No way to choose columns. The guide names the exact remedy. |
| 9 | Applications picker | **Fails** | Commit buttons are spread across the bottom instead of right-aligned in one row. |
| 10 | Highlight rules | **Fails** | Commit buttons are not right-aligned; `Close` carries an access key it should not have. |
| 11 | Highlight rules | **Decision** | Modeless by §5's design; the owner wants modal with Save/Cancel. |
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
