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
| `Alt`+drag, `Shift+Alt`+click | Select a rectangle: the same columns on every row. `Ctrl+C` copies it one row per line; `Ctrl+Shift+C` copies only the columns it touches |
| `Ctrl+I` | Reveal invisible characters |
| `Ctrl+Home` / `Ctrl+End` | Top of file / jump to the tail and follow |
| `PgUp` `PgDn` `Space` `b` `Home` `End` `↑↓←→` | Move; scrolling up pauses following, `Ctrl+End` resumes |
| wheel, `Shift+wheel`, tilt | Vertical (eased), horizontal, horizontal |
| header boundary drag | Resize a column; to zero hides it; double-click resets one |
| header title drag / click | Drag a title onto another to reorder; **click** to sort by that column — ascending, again descending, again off |

`Cut` and `Paste` appear in the Edit menu and are permanently greyed. That is the answer, not an
oversight: Tailhawk opens files read-only and nothing in it edits a log.

## Filters

**Every filter lives in one dialog**, and every way of asking for one opens it: `Ctrl+L`,
`Ctrl+Shift+L`, the toolbar's **Filter** button, **Edit ▸ Filter: include**, **Edit ▸ Filter:
exclude** and **Edit ▸ All filters…**. The first five arrive with the caret in an empty phrase box
and nothing selected, so adding a filter is still one gesture: the row appears in the list with
the first letter typed, carrying the polarity the command named, and nothing typed is nothing
added. The last opens on what is already in force.

Across the top is the selected filter: its **phrase**, whether it is **enabled**, whether it is a
**regular expression**, whether **case** matters, and whether it **includes** or **excludes**. The
list below holds every filter — its text, those two answers, and its type — with a check box that
turns it on and off. A filter that is off keeps its place and its text and is drawn dim, and it is
still off when the file is opened again.

**If a filter is hiding everything, the status bar says so** — `⚠ every row is hidden by a filter —
Ctrl+L to change it`, at the left of the bar. Filters are remembered per file and per remote source,
so a filter that made sense when you wrote it can come back over a file it now excludes completely,
and an empty window is otherwise indistinguishable from one with nothing in it.

**And the title bar says how much of the source is indexed**, whatever the filters do to the view:
`trigger-migration.log — 6.86 MB (7,197,054 bytes) — Tailhawk`. Scaled for a glance and exact for
the record, because the exact number is the one that moves while a file is being tailed. It is the
one fact a filter cannot change, so a blank view over a title reading `6.86 MB` is a filter to go
and look at, and one over `0 bytes` is a source with nothing in it yet.

It reads `at least 6.86 MB (7,197,054 bytes)` when the source holds more than has been indexed —
a rolled set too large to index eagerly, an answer Loki cut at its limit, or a pipe or remote tail,
whose spill keeps the most recent 512 MB and deletes older parts. For a single ordinary file it is
that file's length, less a byte-order mark if it has one, and up to 4 MB behind a writer the follow
pass is still catching up with.

The **…** button at the end of the phrase box opens the composer described below on the selected
row. Down the right, under **Selected filter**: **Remove**, **Up** and **Down** (order is display
order — it does not change which lines survive). Under the list, under **All filters**: **New**,
**Remove all**, and **Enable all include**, **Enable all exclude**, **Disable all include** and
**Disable all exclude**, which act on one polarity at a time, so every excluding filter can be
turned off to see what it was hiding and turned back on afterwards.

**Apply** shows the effect without closing the dialog; **OK** applies and closes; **Cancel** closes
without applying what is on screen. **Apply is a commit**, as it is everywhere in Windows: filters
already applied stay in force, and Cancel does not take them back.

Including filters AND together, and then excluding filters are subtracted.

**Match case** and **Regular expression** are readings of the filter's own text rather than
settings stored beside it — the text is always the filter. Ticking **Regular expression** puts the
phrase in slashes; ticking **Match case** alone also produces a regular expression, because that is
the grammar's only case-sensitive form, and the phrase box shows it happening. Over an expression
that is not a phrase at all — `level >= Warning` — both boxes are greyed and the list says `—`
rather than `No`.

The **…** button opens the **Filter dialog**, which is where a filter is composed against the
columns this file actually has: a column to scope it to (or *any column*), one of thirteen
operators — contains, equals, does not equal, less than, at most, greater than, at least, like,
starts with, ends with, is one of, is empty, is not empty — the value, an include/exclude choice,
and **Regex** and **Match case** for the any-column case. The value box lists the values the
chosen column carries in the window, most common first, so a status or a reason is one pick rather
than something to know; typing is still typing, and *is one of* takes a comma list (`403, 444`)
and writes `status in [403, 444]`. The last three operators ask about a column, so over *any
column* the list stops at ten. *Is empty* finds a row whose column is present and empty; a row
without the column at all answers *unknown*, which an include leaves out. The expression it builds is always
visible and always editable, and a line under it says whether what you have typed parses and
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

**Named sets, applied to a file or a file type.** Rules outside any set are the *Default* set and
apply everywhere. A `[[set]]` names rules that apply only where its `apply_to` says: a path or a
`loki://` tail, a glob (`*.log` matches by file name, `C:\logs\jobs\*.log` by the whole path), or
`format:<name>` for a detected or chosen format — which is the "file type" that means something,
since a Serilog file in any folder, or every `sb-edge` tail, is a format. A set with no `apply_to`
applies nowhere until the editor can choose it. The first set that takes a file wins ties with the
Default set; among the rules of one set, list order is precedence as always.

```toml
[[set]]
name = "Starberry edge"
apply_to = ["loki://starberry*", "format:JSON lines"]

[[set.rule]]
name = "attack"
where = 'reason in ["probe", "payload", "scanner", "proxy-abuse"]'
bg = "#7a1010"

[[set.rule]]
name = "oversize"
where = "status = 413"
bg = "#8a5a00"
```

**A rule that tests the record, not the text.** `where` takes a filter expression — the same
language the filter chips use — and colours the whole line of every record it holds for. It reads
any key of a JSON record, whether or not that key is a column, a number as a number, so a tail
can be tinted by an attribute the body never mentions. A record without the attribute is simply
not matched. A rule has a `pattern` or a `where`, never both. Write paths and expressions in
single quotes, as above: inside double quotes `\t` is a tab, which a Windows path does not want.
**In the editor** (`Ctrl+H`), the row across the top is the set in front: the switcher lists
the Default set and every named one; **New set** makes an empty one and puts the caret in its
name; **Apply to ▾** offers this file, files like it (the folder and the extension, or every tail
of the source), and this format, each ticked while it is on, with anything else the set is bound
to listed under them; **Delete set** removes the set and its rules. A rule's **Move to set ▾**
sends it to the end of another set, and **Where** flips it between a pattern over the text and
a predicate over the record, keeping what was typed. The status bar's format pane names the set
in force for the file in front.

Rules sit above the built-in colouring and below `Ctrl+Shift+n` labels; a background tints under
the ink, so a timestamp keeps its colour on a highlighted line. Under Windows' High Contrast
themes, rules are suppressed and the status bar says so.

## Export and keep saving

From **File**: **Export view…** writes what the view shows (the filter's survivors,
or everything) as UTF-8 text with `\r\n`. **Keep saving…** does the same and then appends each new
line that passes the filter as it arrives; **Stop saving** stops it. The status bar carries the
count, and says so if a write fails.

## Remote sources

**Tools ▸ Remote sources…** holds the Loki servers Tailhawk can tail and what it shows of each, as
a tree: a **source** is a server, and the **tails** underneath it are the named views you open.

```
telemetry.nurtur.tech
    ndc
    identity server
    job dispatcher
```

**A source is a connection; a tail is a view.** One Loki, one sign-in, one stored credential — and
as many tails of it as you have things to watch. `File ▸ Open remote` lists the **tails**, because a
tail is what a window shows and what a tab is called.

The tree has five verbs beside it and **Close**:

| | |
|---|---|
| **New source…** | A server. Asks where it is, and works the rest out. |
| **Add tail…** | A view of whichever source you are pointing at — or of the source above the tail you are pointing at, which is the same thing. |
| **Edit…** | Whichever row you are on: a source's screen for a source, a tail's for a tail. Double-clicking a row does the same. |
| **Clone** | A copy under a name of its own. A **source** brings its tails with it, which is the quick way to a second environment that differs in one URL. A **tail** is a second view of the same server — no second sign-in. |
| **Delete** | A **tail** goes at once. A **source** asks first, and says how many tails go with it, because they all read that server. |

A cloned source starts with no secret on purpose: a credential is filed under the source's *name*,
so an inherited one would belong to the original.

**Close saves.** Every change is made on a screen of its own that you accepted or cancelled there
and then, so by the time you reach Close you have agreed to each one already.

### A source's screen

**New source…** and **Edit…** on a source open the same screen, and it has three things on it:

| | |
|---|---|
| **Name** | What you call the server. It is also the key its credential is filed under. |
| **Server URL** | Where Loki is. The only thing you have to know. |
| **Set up** | One press: it asks that server where it signs people in, asks the Loki itself whether it answers, fills in what it learns, and says what it found in one sentence. |

There is no selector here, and no applications: those are what a *tail* is.

**Advanced** unfolds the rest: how Tailhawk signs in, the token and sign-in URLs, the client id, the
scope and the secret. You should not need any of it when **Set up** can reach the server, and it is
all there when it cannot — which is the only way some configurations can be set up.

The **sign-in** choice is the one that changes what the rest means:

| | |
|---|---|
| **Sign in with a browser** | You sign in as yourself. Needs a client id; never needs a secret. |
| **Use a client id and secret** | The one way that uses the secret box. |
| **No sign-in needed** | A Loki with authentication off. Needs nothing at all. |

The secret box is greyed out on the two that do not use one, and a source that stops using a secret
has its stored one deleted rather than left behind for a sign-in that will never send it. A source
that wants a secret and has none says so on its row in the tree.

### A tail's screen

**Add tail…** and **Edit…** on a tail open the same screen, and it has three things on it:

| | |
|---|---|
| **Reads** | Which source it reads. Stated, not asked — it is the row you came from. To point a view at another server, add a tail there; the two servers have different credentials and may hold different labels. |
| **Name** | What you call the view. This is what `File ▸ Open remote` lists and what the tab is called. |
| **Shows** | The LogQL selector, such as `{application="ndc"}`. |

**Choose…** asks that server which applications it holds and offers them as a list to tick. What you
tick is written into **Shows** as part of the selector — `app="job-dispatcher"` — rather than into a
field of its own, because `app` is one of your server's labels and not a concept the product owns.

**What Set up tells you**, and what to do about it:

| It says | What it means |
|---|---|
| Reached it, and it wants a token | The URL and its path are right. This is the answer you want. |
| Reached the server, but there is no Loki at that path | Usually a missing `/loki` on the end of the URL. |
| Reached it, and it answered without a token | Authentication is off, and the sign-in choice moves to **No sign-in needed**. |
| Found ‹issuer› at ‹host› | Where you will sign in, filled into Advanced for you. |
| No sign-in server answered | Fill in the token URL and the sign-in URL by hand. |

Tailhawk looks for the sign-in server on the host you named and, if it serves none, beside it — at
`identity`, `auth`, `login`, `sso` and `idp` in the same domain. It only guesses beside a host that
is *inside* a domain: `telemetry.nurtur.tech` looks in `nurtur.tech`, while a URL at a domain itself,
like `acme.com`, gets no guesses, because `identity.com` would be somebody else's machine.

**Set up refuses what an open would refuse** rather than letting you find out later: a URL that is
not `https://`, and the private and loopback addresses §7 forbids for a source read from a settings
file. So a Set up that says it reached something is a source that will open.

## What is remembered

`tailhawk.settings.toml`, next to the exe if that is writable, else `%APPDATA%\Tailhawk\` — and if
both exist the values are merged, the exe-adjacent one winning key by key. It holds the window's
place, the theme, the font and its size, the recent files, the search history, and per file its
filters, collapse, bookmarks, labels and column widths. `--stateless` reads them and writes
nothing.

A filter is written as its sign and its text — `+error`, `-retry` — with a leading `#` when it is
switched off, so `"#+error"` is an include filter that is not in force. Hand-editing a `#` in front
of the whole entry, `"#error"`, makes it a filter *for the text* `#error`, which matches nothing.

### A settings file written before this

**It is read and migrated, and nothing of yours is lost.** A file from before 2026-10-21 has one
`[[source]]` per view, carrying both the connection and the selector. Reading it splits them:
entries that agree about everything but the view become **one** source named after its host, with a
**tail** for each — and the tails keep the names those entries had, so `File ▸ Open Recent` and
every remembered application set go on working.

**Your sign-in follows the rename.** The credential store is keyed on the source's name, so the
first launch after the upgrade moves each migrated server's stored secret and sign-in onto its new
name. You stay signed in.

## Formats

The format is detected on open and named in the status bar, with the confidence:
`· Serilog (file) 92%`. **Format ▸ Log format** lists what was considered, with the chosen one
ticked, so a wrong guess can be overruled.

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
