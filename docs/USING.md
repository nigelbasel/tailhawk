# Using Tailhawk

A one-page guide to the viewer as it runs today. Windows only.

There are two ways to reach a command. The **menu bar** holds everything — seven menus, and a test
asserts that every command in the register appears under one of them, so nothing is reachable only
by a keystroke you would have to already know. And **right-click knows where you are**: a column
header, a line of the grid and a filter row each have their own menu, offering things that are
awkward to reach any other way.

Every menu item names its key beside it, and **Help ▸ Keyboard map** lists them all on one page.

## Running it

```
tailhawk app.log                       # open at the tail, following
tailhawk app.log other.log             # two tabs
tailhawk C:\logs\                      # a watched folder: new files become tabs
tailhawk "C:\logs\app*.log"            # a watched glob
type app.log | tailhawk -              # a pipe
tailhawk --filter=error --exclude=heartbeat app.log
tailhawk --theme=light app.log         # dark | light | system
tailhawk --column-pattern="<ts> [<thread>] <level> <logger> - <message>" app.log   # your own columns
tailhawk --columns=none app.log        # no detection, plain lines
tailhawk --stateless app.log           # remember nothing
```

`--filter`, `--exclude` and `--column-pattern` may each be given more than once.

In a column pattern, `<ts>` is any timestamp, `<level>` a level word, `<message>` the rest of the
line, `<_>` a word to discard, and anything else names a column. **`<<` is a literal `<`**, for a
line that has angle brackets of its own:

```
tailhawk --column-pattern="[<ts>] <<<instance>> [<level>]  <message>" app.log
#          matches: [11:19:32.064] <bym2013> [Information]  Request starting
```

You rarely need this. A Serilog, NLog or log4net template found in your own configuration is
compiled automatically, and there `<` is ordinary text.

`tail`'s flags are **accepted and ignored**: `-f`, `-F`, `-q`, `--quiet`, `--silent`, `-v`,
`--verbose`, and `-n`, `-c`, `-s`, `--sleep-interval`, `--pid` along with their values. They are
there so a command you already have in your fingers does not fail — not because they do anything.
Following is the default, and the window height decides how much is shown. There is no `--help`
and no `--version`, and an unrecognised `--flag` is swallowed silently.

A second `tailhawk file.log` opens a **second window**. To add a file to a window you already have,
drop it on that window, or press `Ctrl+O`.

## Keys

| Key | What |
|---|---|
| `Alt`, `F10` | The menu bar, with its mnemonics underlined |
| `Ctrl+G` | Go to line… |
| `Ctrl+O` | Open file… (a new tab) |
| `Ctrl+F` | Find — the standard modeless dialog: match case, whole word, regular expression, wrap |
| `F3` / `Shift+F3` | Next / previous match; with no results it re-runs the last query |
| `Ctrl+L` / `Ctrl+Shift+L` | Add an including / excluding filter, through the Filter dialog |
| `Ctrl+Shift+E` | Edit the last filter |
| `Esc` | Unwinds one step at a time: the search first, then the sort, then the filters |
| `Ctrl+E` | Collapse continuation lines (stack traces) into their records |
| `Ctrl+D`, `F2` / `Shift+F2` | Bookmark the current row (amber mark in the gutter), next / previous bookmark |
| `Ctrl+Shift+1…9`, `Ctrl+Shift+0` | Colour-label every line containing the selection; clear labels |
| `Ctrl+Enter` | Record detail pane (fields, body, stack trace) |
| `Ctrl+H` | Highlight rules editor |
| `Alt+←` / `Alt+→` | Back / forward through views (filters, collapse, jumps) |
| `Ctrl+\` , `F6` | Split the pane (a second view of the file with its own filter); swap focus |
| `Ctrl+Tab` / `Ctrl+Shift+Tab`, `Ctrl+W` | Next / previous tab, close tab (middle-click closes too) |
| `Ctrl+C` / `Ctrl+Shift+C` | Copy raw / copy as TSV with columns |
| `Ctrl+I` | Reveal invisible characters |
| `Ctrl+Home` / `Ctrl+End` | Top of file / jump to the tail and follow |
| `PgUp` `PgDn` `Space` `b` `Home` `End` `↑↓←→` | Move; scrolling up pauses following, `Ctrl+End` resumes |
| wheel, `Shift+wheel`, tilt | Vertical (eased), horizontal, horizontal |
| header boundary drag | Resize a column; to zero hides it; double-click resets one |
| header title drag / click | Drag a title onto another to reorder; **click** to sort by that column — ascending, again descending, again off |

`Cut` and `Paste` appear in the Edit menu and are permanently greyed. That is the answer, not an
oversight: Tailhawk opens files read-only and nothing in it edits a log.

## Filters

Filters live in a **panel docked above the status bar**, which appears as soon as there is one and
can be toggled from **View ▸ Filter panel**. Its title row carries **Add…**, **Edit…**, **Remove**
and **Clear all**; Edit and Remove act on the selected row. On a row itself:

- the `[x]` mark turns the filter off and on, and `Ctrl+click` on it opens the filter for editing
- the `+` / `−` sign flips it between including and excluding
- `×` removes it
- the text selects the row, and dragging a row reorders the list (display only — order does not
  change which lines survive)
- right-click offers **Edit…**, **Make excluding**, **Enabled** and **Remove**

Including filters AND together, and then excluding filters are subtracted.

**Add…** and `Ctrl+L` open the **Filter dialog**, which is where a filter is composed: a column to
scope it to (or *any column*), one of ten operators — contains, equals, does not equal, less than,
at most, greater than, at least, like, starts with, ends with — the value, an include/exclude
choice, and **Regex** and **Match case** for the any-column case. The expression it builds is
always visible and always editable, and a line under it says whether what you have typed parses and
whether it names a column this format does not have.

Typed by hand, the expression language is wider than the dialog's operators:

```
error                          plain text, case-insensitive substring
"line 12"                      quoted: literal, even if it looks like an expression
/time(d)?out/i                 regex; i is the only flag, and means case-insensitive
level >= Warning               a field comparison; level names work across formats
status in [500, 502, 503]      no equivalent in the dialog
startsWith(logger, "Zenith.")  also contains(...) and endsWith(...)
level = Error and status = 500 and / or combine two predicates
```

Fields are `level` (or `severity`), `timestamp`, `body`, `source`, `trace`, `span`, any column the
detected format names, or `attributes.<key>`. A field the format does not have makes an including
filter exclude the row and an excluding filter keep it.

The grid's own right-click menu offers **Filter to this text** and **Filter out this text** from
the selection, and a column header offers **Filter on this column…** with the dialog already
scoped.

## Sort and top-N

Click a column title to sort the visible rows by it (`▲` after the title; click again for `▼`,
again for file order), or use the header's right-click menu — **Sort ascending**, **Sort
descending**, **Top 100 by this column**, **Clear sort**. Level sorts by severity, timestamps by
instant, numbers as numbers, the rest as text; rows the column is missing from come last.

**Sorting holds the view still**: it is not following, and rows that arrive while it is sorted are
counted in the status bar and shown when the sort clears.

```
↕ sorted by level ▼ · not following · 12 newer rows held
↕ top 100 by duration ▼ · not following
↕ sorting by level… 40%
```

A sort is over the rows a filter keeps, needs 2 M rows or fewer, and drops when the filters change.
Past that size the status bar says so and asks you to filter first or take a top-N — **Top 100 by
\<column\>** has no size cap, which is what makes it the tool for the slowest requests or the
biggest responses in a very large file.

## Highlighting

Zero-config colouring (timestamps, levels, numbers, IPs, URLs, paths, durations, ids) is always on.

Your own rules live in **`tailhawk.rules.toml`**, and there are two of those: a curated set beside
the exe, which outranks a personal one. `Ctrl+H` opens the **rules editor** in the app (`Ctrl+S`
saves). **Rules ▸ Open rules file** creates the file with two examples and opens it in a text
editor instead, and **Rules ▸ Reload rules** picks up an edit made outside.

```toml
[[rule]]
name = "exceptions"
pattern = "\\bException\\b"      # a regex, unless literal = true
fg = "#ff7b6b"                    # foreground, and/or
bg = "#3a1e1e"                    # background
whole_line = true                 # colour the line, not just the match
literal = false                   # true matches the pattern as plain text
enabled = true
case_insensitive = true
```

Rules sit above the built-in colouring and below `Ctrl+Shift+n` labels; a background tints under
the ink, so a timestamp keeps its colour on a highlighted line. Under Windows' High Contrast
themes, rules are suppressed and the status bar says so.

## Export and keep saving

From **File**: **Export view…** writes what the view shows (the filter's survivors,
or everything) as UTF-8 text with `\r\n`. **Keep saving…** does the same and then appends each new
line that passes the filter as it arrives; **Stop saving** stops it. The status bar carries the
count, and says so if a write fails.

## What is remembered

`tailhawk.settings.toml`, next to the exe if that is writable, else `%APPDATA%\Tailhawk\` — and if
both exist the values are merged, the exe-adjacent one winning key by key. It holds the window's
place, the theme, the font and its size, the recent files, the search history, and per file its
filters, collapse, bookmarks, labels and column widths. `--stateless` reads them and writes
nothing.

## Formats

The format is detected on open and named in the status bar and the window title, with the
confidence: `· Serilog (file) 92%`. **Format ▸ Log format** lists what was considered, with the
chosen one ticked, so a wrong guess can be overruled.

Detected without configuration: syslog (RFC 5424 and RFC 3164), NLog, MEL Simple, Serilog (file and
console), Apache/nginx access, log4net (full and compact), Python logging (both shapes), JSON
lines, logfmt, timestamped text, and IIS W3C from its own `#Fields:` line.

Better than any of those, a **Serilog, NLog or log4net template found in your own configuration**
is compiled and used: `appsettings*.json`, `nlog.config`, `log4net.config`, `web.config` or
`app.config`, searched in the log's own directory and up to three above it.

When neither fits, **Format ▸ Define from a line…** opens a dialog that builds a format from a line
you select — the same one the grid's right-click offers — and **Format ▸ Import layout…** takes a
layout pasted from a configuration file. A detected file shows aligned columns under a header;
stack traces sit dimmed under their record.
