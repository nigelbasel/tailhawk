# Using Tailhawk

A one-page guide to the viewer as it runs today. Windows only. Everything here is reachable from
the **command palette** (`Ctrl+K`), which lists every command with its key — start there when in
doubt.

## Running it

```
tailhawk app.log                       # open at the tail, following
tailhawk app.log other.log             # two tabs
tailhawk C:\logs\                      # a watched folder: new files become tabs
tailhawk "C:\logs\app*.log"            # a watched glob
type app.log | tailhawk -              # a pipe
tailhawk -n 100 -f app.log             # tail's flags are accepted and mean the same
tailhawk --filter=error --exclude=heartbeat app.log
tailhawk --theme=light app.log         # dark | light | system
tailhawk --column-pattern="<ts> [<thread>] <level> <logger> - <message>" app.log   # your own columns
tailhawk --columns=none app.log        # no detection, plain lines
tailhawk --stateless app.log           # remember nothing
tailhawk --new-instance app.log        # a second window rather than a tab in the running one
```

A second `tailhawk file.log` opens the file as a tab in the running window (one instance per
session); dropping a file on the window does the same; `Ctrl+O` asks for one.

## Keys

| Key | What |
|---|---|
| `Ctrl+K` | Command palette — every command by name and key; type a number for *go to line* (`Ctrl+G`) |
| `Ctrl+O` | Open file… (a new tab) |
| `Ctrl+F`, `F3` / `Shift+F3`, `Esc` | Search (regex, case-insensitive), next / previous match, clear |
| `Ctrl+L` / `Ctrl+Shift+L`, `Enter` | Add an include / exclude filter chip; `Esc` clears them |
| click a chip / `×` | Toggle it / remove it; `Ctrl+click` or `Ctrl+Shift+E` edits it; drag reorders |
| `Ctrl+E` | Collapse continuation lines (stack traces) into their records |
| `Ctrl+D`, `F2` / `Shift+F2` | Bookmark the current row (amber mark in the gutter), next / previous bookmark |
| `Ctrl+Shift+1…9`, `Ctrl+Shift+0` | Colour-label every line containing the selection; clear labels |
| `Ctrl+Enter` | Record detail pane (fields, body, stack trace; JSON pretty-print from the palette) |
| `Alt+←` / `Alt+→` | Back / forward through views (filters, collapse, jumps) |
| `Ctrl+\` , `F6` | Split the pane (a second view of the file with its own filter); swap focus |
| `Ctrl+Tab` / `Ctrl+Shift+Tab`, `Ctrl+W` | Next / previous tab, close tab (middle-click closes too) |
| `Ctrl+C` / `Ctrl+Shift+C` | Copy raw / copy as TSV with columns |
| `Ctrl+I` | Reveal invisible characters |
| `Ctrl+Home` / `Ctrl+End` | Top of file / jump to the tail and follow |
| `PgUp` `PgDn` `Space` `b` `Home` `End` `↑↓←→` | Move; scrolling up pauses following, `Ctrl+End` resumes |
| wheel, `Shift+wheel` | Vertical (eased), horizontal |
| header boundary drag | Resize a column; to zero hides it; double-click resets one; palette resets all |
| header title drag / click | Drag a title onto another column to reorder; **click** to sort by that column — ascending, again descending, again off (`Esc` also clears it) |

## Filters

A chip is one predicate; include chips AND together, then exclude chips are subtracted.

```
error                          plain text, case-insensitive substring
"line 12"                      quoted: literal, even if it looks like an expression
/time(d)?out/i                 regex, i = case-insensitive
level >= Warning               a field comparison; level names work across formats
status in [500, 502, 503]
startsWith(logger, "Zenith.")
```

Fields are `level`, `timestamp`, `body`, `source`, `trace`, `span`, any column the detected format
names, or `attributes.<key>`. A field the format does not have makes an include chip exclude the row
and an exclude chip keep it, and the chip says so.

## Sort and top-N

Click a column title to sort the visible rows by it (`▲` after the title; click again for `▼`, again
for file order). Level sorts by severity, timestamps by instant, numbers as numbers, the rest as
text; rows the column is missing from come last. **Sorting holds the view still**: it is not
following, and rows that arrive while it is sorted are counted in the status bar (`↕ sorted by level
▼ · not following · 12 newer rows held`) and shown when the sort clears. A sort is over the rows a
filter keeps, needs 2 M rows or fewer (filter first), and drops when the chips change. The palette
also lists *Sort by <column>, ascending / descending* and **Top 100 by <column>** — the hundred
highest by that column (slowest requests, biggest responses), which has no size cap.

## Highlighting

Zero-config colouring (timestamps, levels, numbers, IPs, URLs, paths, durations, ids) is always on.
Your own rules live in **`tailhawk.rules.toml`** — palette: *Edit highlight rules…* creates it with
two examples and opens it; *Reload highlight rules* picks up an edit:

```toml
[[rule]]
name = "exceptions"
pattern = "\\bException\\b"      # a regex
fg = "#ff7b6b"                    # foreground, and/or
bg = "#3a1e1e"                    # background
whole_line = true                 # colour the line, not just the match
enabled = true
case_insensitive = true
```

Rules sit above the built-in colouring and below `Ctrl+Shift+n` labels; a background tints under
the ink, so a timestamp keeps its colour on a highlighted line.

## Export and tee

From the palette: **Export the visible rows to a file…** writes what the view shows (the filter's
survivors, or everything) as UTF-8 text with `\r\n`; **Tee: keep writing the visible rows…** does the
same and then appends each new line that passes the filter as it arrives (Hoo WinTail's tee); **Stop
the tee** stops it. The status bar shows the count.

## What is remembered

`tailhawk.settings.toml`, next to the exe if that is writable, else `%APPDATA%\Tailhawk\`: the
window's place, the theme, and per file its chips, collapse, bookmarks and labels. `--stateless`
writes nothing.

## Formats

The format is detected on open and named at the right of the bar (Serilog, log4net, NLog, MEL,
syslog, CLF, Python, JSON lines, logfmt, timestamped text, IIS W3C from its own `#Fields:`). A
Serilog / NLog / log4net template found in an `appsettings*.json`, `nlog.config`, `log4net.config`,
`web.config` or `app.config` beside the log is compiled and used. A detected file shows aligned
columns under a header; stack traces sit dimmed under their record.
