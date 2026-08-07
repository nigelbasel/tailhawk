# Research — Tailhawk, a modern Windows log tailer

**Status:** research complete, adversarially reviewed. Name chosen (see §1.1).
**Date:** 2026-07-28

---

## 0. How to read this document

This is the output of five multi-agent research workflows (26 agents, ~2.5M tokens) covering
competitors, file/performance engineering, log format catalogues, Unix `tail` semantics,
implementation stacks, cross-platform strategy and OpenTelemetry. **Every artifact was then
attacked by independent hostile critics**, who returned 24 blocking, 40 major and 14 minor issues
against the first research body alone.

Findings carry a confidence marker:

| Marker | Meaning |
|---|---|
| **[V]** | Verified — a primary source was fetched that states this |
| **[L]** | Likely — strong indirect evidence or sound derivation |
| **[U]** | Uncertain — plausible but unconfirmed; **do not put in the spec as a target** |
| **[REFUTED]** | A critic disproved this; recorded so it is not rediscovered |

**§11 is the most important section.** It lists what the critics killed. Several confidently-stated
research findings turned out to be confabulated, and they were about to become spec acceptance
criteria. Read §11 before quoting any number from this document.

---

## 1. The brief and what the user actually needs

### 1.1 Product name — **Tailhawk**

The working title `WinTail` **cannot be used**. WinTail is the live, currently-sold product name of
Hoo Technologies (hootech.com/WinTail/, v4.2, ~$49.95) — the very tool this project intends to
replace. Shipping a Windows log tailer under that name is a passing-off risk and an SEO dead end. **[V]**

**Chosen name: Tailhawk. Command: `tailhawk`.**

The requirement, from the owner: *the name must evoke monitoring log files and remind the user of the
Unix `tail` app.* Round 1 (Fanfold, Tailix, Ripsaw, Spatefall, Thalweg, Geophone, Greenbar, Cresset,
VTail, Nyctal) was rejected for optimising availability over meaning. Round 2 generated and hostilely
screened candidates carrying **both** signals.

Why Tailhawk: *"watch like a hawk"* is the most universally recognised sharp-sight idiom in English
and needs no explanation for any audience; the **tail-first** construction puts the heritage morpheme
first, so `tail(1)` is the first thing read and `tailhawk` sorts adjacent to `tail` in shell
autocomplete and package listings; 8 characters is dead centre of the ideal command-length band.

**The owner selected it specifically for the "watch like a hawk" idiom.** That settles the branding
direction and neutralises the judge's main objection — that *Tailhawk* could scan as modifier-plus-noun
("a hawk for tails") rather than as a peer of BareTail/SnakeTail. Leading with the idiom in the tagline
and visual identity makes the name decode correctly on first contact:

> **Tailhawk — watch your logs like a hawk.**

Known weaknesses, accepted knowingly:
- It inherits search noise from the **Tail\*** prefix cluster (Tailscale, Tailwind, Tailcall,
  Tailspin) rather than the BareTail/SnakeTail shelf. **[V]**
- It is one consonant from *Tailhook* (the carrier arresting hook, and the 1991 scandal) — an
  unfortunate near-miss when spoken in a noisy room. **[L]**
- The inverted form **Hawktail is dead** — `hawktail.com` is a live seed-stage VC firm with 120+
  portfolio companies and `github.com/hawktail` is an Organization created 2018-01-13. Anyone
  researching the name will find it. **[V]**

Verified this session: **no GitHub user or org account for `tailhawk`; GitHub repo-name search
returns 0 repositories.** From screening, not re-verified: crates.io 404, npm 404. **Unverified and
an action:** `tailhawk.com` was not checked by anyone in this round — treat domain status as unknown.

**Two systemic hazards** were found that apply to the whole `-tail` family and should inform how the
name is presented: in 2026 a `-tail` suffix on a sysadmin utility reads as **Tailscale** first; and
`<noun>tail` is literally the **Warrior Cats** fan-naming convention, which occupies many otherwise
clean compounds. **[V]**

The name is now the code-signing identity, the winget/scoop package ID, the GitHub org and the
SmartScreen reputation anchor. **None of these can be cheaply changed later** — SmartScreen publisher
reputation resets when the certificate subject changes. **[V]**

### 1.2 Requirements (given)

- Tiny, single self-contained executable, **no runtime dependencies**; copy-and-run, deployable from
  a URL or UNC share like a SysInternals tool
- Handles very large log files (multi-GB) with instant open and low memory
- Automatic encoding handling — Unicode and ASCII, transparently
- A **modern**-looking Windows UI, not an old-school MDI app, but tailing **multiple files at once**
- Highlighting by plain-text filters and regular expressions
- Columnising using standard formats from Serilog, log4net, NLog and similar
- At minimum, full feature parity with standard Unix/Linux `tail`
- Written in modern C++ or Rust
- **Open source**

### 1.3 The user's actual workload (stated directly)

The completeness critic's top finding was that the research contained **zero user research** — every
requirement was reverse-engineered from competitor feature lists. That has been partly corrected:

| Question | Answer |
|---|---|
| Log location and size | **Genuinely multi-GB local files**, **UNC / network shares**, **and Docker / Azure Container Apps** |
| Does ClearType vs greyscale matter? | **No — prioritise shipping** |
| v1 ambition | **Full vision, however long it takes** |
| Hoo WinTail features actually used | **Highlight rules + include/exclude filters**; **file sets and folder monitoring**; **multiple text filters** |

Three consequences follow immediately:

1. **The multi-GB engineering programme is justified.** Had the answer been "200MB local files", most
   of §5 would have been over-built by an order of magnitude.
2. **UNC is a first-class target, not an edge case.** This is the environment where change detection
   is least reliable and where a gating lab experiment is required (§5.4, §12).
3. **Container logs mean the app needs a *source* abstraction, not just a file path.** See §10.
4. **OutputDebugString capture and tray notification were *not* selected** — they can drop out of v1
   despite three incumbents having them.

---

## 2. Competitive landscape

### 2.1 The incumbents

| Tool | Status | Deployment | Runtime dep | Big files | Encoding | Columns | Multi-file |
|---|---|---|---|---|---|---|---|
| **Hoo WinTail** 4.2 | Frozen ~2014, still sold ~$49.95 | 1.43MB installer | None | Ring buffer over the tail only | BOM-ish | **No** | MDI + tabs |
| **BareTail** 3.50a | **2006**, unmaintained | **220KB single .exe** | None | Claims >2GB | Unicode/UTF-8/ANSI/ASCII | No | Tabs |
| **BareTailPro** 2.50a | 2006 | 298KB | None | as above | as above | Sortable result cols | Tabs |
| **LogExpert** 1.41.0 | **Very active** (8 releases Jun–Jul 2026), MIT | 7.0MB + runtime | **.NET 10 Desktop Runtime** | Poor (see below) | Weak (see below) | **Yes** (plugins) | MDI + tabs |
| **LogFusion** 7.2 | Active, commercial | 24.8MB | .NET + WebView2 | ? | ? | Pro only | Pro only |
| **SnakeTail** 1.9.8 | 2024, GPL-3.0 | 281KB portable | .NET Framework 2.0 | "low memory independent of size" | ? | No | MDI/Tabbed/Floating |
| **TailBlazer** 0.9.0 | **Dead since 2016**, 55,775 downloads | 4.2MB | .NET Framework | Author claims 47GB | ? | No | Tabs + side-by-side |
| **TextAnalysisTool.NET** | Active | .NET 4.8 | .NET Framework | **Loads all into RAM** | ANSI/UTF-8/UTF-16 | No | Single-file |
| **Tailviewer** 0.9.6.2 | Dormant | 4.98MB | .NET | ? | ? | No | **Merged by timestamp** |
| **Notepad++** monitoring | Active | — | None | **Dies ~2GB / 100k lines** | Good | No | Editor tabs |
| **klogg** v22.06 stable (2022-06-13); `continuous-win` nightly 2024-11-26 | **Last commit 2024-11-26** | 19.5MB **multi-DLL zip** *(measured from the 2024-11-26 nightly)* | Qt | **10+GB, >2³¹ lines** | **uchardet auto-detect** | **No** | Tabs only |
| **qlogexplorer** 1.2.1 | Active (23 stars) | 24MB | Qt6 | Chunk indexing | ? | **Yes, named captures** | Tabs |

**Provenance of this table:** release dates, asset byte sizes and licences are **[V]** from the GitHub
releases API and vendor pages. Capability cells marked `?` are **unknown, not "no"**. Cells describing
behaviour rather than metadata — big-file handling, encoding support — are **[L]** unless a primary
source states them, and *"Author claims 47GB"* (TailBlazer) is a vendor claim, not a measurement.
**There is no klogg release numbered 24.11**; an earlier draft manufactured that version string from
the last-commit date. The 19.5MB figure is measured from the nightly, not from any stable release.

### 2.2 Why Hoo WinTail is good — the parts to preserve

Verified from the vendor feature list **[V]**: real-time tail without loading the whole file;
highlight filters with foreground *and* background colour plus whole-line highlighting; **include and
exclude filters, both supporting regex** (v4.0+); **import/export of filter sets**; per-filter
enable/disable; **"file sets"** — open a named batch of files in one click, also from the command
line; **folder monitoring with auto-open of newly created files**; a red separator line between old
and new content when a tab loses focus; bookmarks and numbered bookmarks; read-only guarantee;
HTML export preserving highlight colours; write matching lines to a separate file.

The three the owner confirmed using — highlight rules, include/exclude filters, file sets and folder
monitoring — are non-negotiable for v1.

### 2.3 Why Hoo WinTail is dated

Win32/MFC MDI shell, no dark mode, no modern DPI story, OS list ends at Windows 10, site copyright
2004–2014. The "display buffer size" setting reveals the architecture: **a bounded ring buffer over
the tail, not a full-file random-access index** — the "100GB support" claim means it can tail the end
of such a file, not scroll or search it. No columnising of any structured format. No merged
multi-file view. SMTP as the only notification channel. **[L]**

### 2.4 Why LogExpert feels clunky and heavyweight — the concrete evidence

This matters because it defines what to avoid.

- **Mandatory .NET 10 Desktop Runtime** — a hard external prerequisite, ~7MB of app on top of a large
  runtime install. Not self-contained. **[V]**
- **Issue #143** (open since 2019-10-24, no maintainer resolution): *"I had over 30 files open and its
  taking 1GB of memory whereas BareTail is taking 2MB"* and *"The loading and closing of files, in my
  example 30 files takes minutes to open / close whereas in BareTail its instant."* **[V]**
  — but see §11 for why these numbers must **not** become targets.
- **Issue #129**: at ~25MB the file gets **fully reloaded** rather than incrementally appended, even
  with follow-tail disabled. **[V]**
- **Issue #634** (2026): follow-tail intermittently fails to scroll to bottom under high append rate —
  async `BeginInvoke` flooding the message queue. **[V]** *(Closed, maintainer-authored — see §11.)*
- **Columnizer friction, issue #50**: *"you have to set up a dev environment and compile them. And
  always recompile them when you need to slightly adjust the line format."* Custom columnizers are
  compiled .NET assemblies dropped in a plugin folder. **[V]**
- **Encoding is a genuine weak spot.** Open PR #671 (2026-07-27) documents that the memory-mapped
  reader performs **no BOM detection at all**, BOM-less files fall through to `Encoding.Default`, and
  legacy codepages can be rejected outright. Resolution order is BOM → persisted → preference →
  `Encoding.Default`, with **no statistical detection** for BOM-less UTF-8 or UTF-16. Still unfixed in
  a shipped release as of July 2026. **[V]**
- **No merged multi-file view** — issue #54 open since 2018. No word wrap — #33 open since 2018. **[V]**

### 2.5 The gap

Cross-referencing everything, **no tool combines all of**: (a) a truly self-contained single exe with
zero runtime install, (b) instant open and bounded memory on multi-GB files, (c) real charset
auto-detection including BOM-less UTF-8/UTF-16, (d) columnised structured-log parsing, (e) live
merged-by-timestamp multi-file viewing. **[L]**

- The only *fast* GUI viewers are Qt — klogg (19.5MB of DLLs) and qlogexplorer (24MB)
- Columnisation exists only in slow .NET/JVM tools or a 23-star Qt project
- klogg has **explicitly refused** structured parsing for a decade and its #2 most-requested issue,
  *cross-file search*, has been open since **2018-08-14** **[V]**
- Tailviewer *does* do live merged-by-timestamp on Windows — but is dormant .NET **[V]**
  *(This corrects an earlier claim that no incumbent has it — see §11.)*
- Observability-tool UX (timeline histogram scrubber, pattern clustering, dedup modes, field-level
  filter-for/filter-out) exists in **no** desktop file-based log viewer **[V]**

**TailBlazer is the most instructive datapoint**: modern-looking, dark-themed, virtual scrolling,
4MB — 55,775 downloads on its final release, then abandoned in 2016. That is direct evidence of an
unserved market. **[V]**

### 2.6 Competitors the research missed

The completeness critic flagged omissions that matter for a Windows product: **CMTrace/OneTrace**
(Microsoft's ConfigMgr log viewer, on a huge number of Windows admin machines), **DebugView** (the
exemplar of the single-exe model being copied), **VS Code with a log extension** (where the user
already lives — the true "good enough" competitor), and **WSL/Git Bash**, where a Windows developer
already has `tail`, `less`, `lnav` and `ripgrep` for free. The WSL and VS Code threats are zero-friction
and will determine whether anything gets adopted. **[V]**

---

## 3. Architecture and stack

### 3.1 The decision

> **One shared Rust core owns the entire log grid — indexing, decode, highlight, text layout *and*
> rendering — with per-platform *leaf* backends for font rasterisation and GPU presentation. Thin
> native shells own only window, chrome, menus, input, IME and accessibility. Windows only in v1.
> No public C ABI on day one.**

### 3.2 How that was reached

The stack research initially ranked **Rust + Win32 + DirectWrite/D3D11 with a custom ClearType blend
shader** first, on the grounds that ClearType/subpixel antialiasing is the decisive quality attribute
for an all-day text-dense viewer, and that no Rust GPU UI crate provides it.

Three things undermined that:

1. **The "existence proof" was mis-attributed [REFUTED].** The Zed Windows blog post confirms the
   Vulkan→DirectX 11 move and DirectWrite glyph rasterisation, but says **nothing** about subpixel
   rendering, ClearType, or a `text_rendering_mode` setting. That claim came from an unofficial
   community tips site. **[V]**
2. **The owner does not care** (§1.3), which removes the justification entirely.
3. **ClearType is not a function of "native shell" at all.** Per Microsoft's `D2D1_TEXT_ANTIALIAS_MODE`
   docs: *"By default, Direct2D renders text in ClearType mode."* It downgrades to greyscale only if
   the rendering mode is ALIASED/OUTLINE, if the render target has a live alpha channel not set to
   `D2D1_ALPHA_MODE_IGNORE`, or under a `PushLayer` without `D2D1_LAYER_OPTIONS_INITIALIZE_FOR_CLEARTYPE`.
   **[V]** Subpixel AA is therefore a function of *render-target alpha mode* and *which backend
   rasterises* — not of the surrounding app's architecture. **A log grid is an opaque rectangle of
   solid background: the ideal ClearType candidate.** Zed got greyscale because GPUI composited a
   single cross-platform rasteriser's atlas *with alpha*.

**Consequence:** a DirectWrite leaf rasteriser inside a shared core, drawing to an opaque target,
plausibly gets ClearType with **one** grid implementation and no custom blend shader. The owner's
"don't care" answer de-risks the schedule; we may get the quality anyway.

**Hard constraint that survives:** ClearType XOR translucency. Apply Mica/Acrylic to title bar, tab
strip and side panels only; keep the log grid on an opaque target. Windows Terminal hit exactly this
and closed it as not planned. **[V]**

### 3.3 Candidates rejected, with reasons

| Candidate | Verdict | Decisive reason |
|---|---|---|
| **egui / eframe** | Chrome only, never the grid | Text AA issue #2639 **closed as not planned** — greyscale only, no hinting, *monospace loses true fixed width*. `ScrollArea::show_rows` jitters above **2M rows** and "gets very broken" above **100M** (issue #1391, open, untouched since 2022-04-04). A 10GB log is ~100M lines — dead centre. **[V]** |
| **Slint** | No | Subpixel bug #5748 open across **all three** renderers; royalty-free tier requires visible attribution; Skia renderer has "heavy disk footprint" **[V]** |
| **iced** | No | Screen reader cannot read window content, accessibility issue open ~4.5 years; largest binaries measured (Sniffnet 20.2MB, Halloy 15.6MB) **[V]** |
| **Xilem / Masonry** | Track, don't build | Self-described *"alpha state… not a production-ready product"* **[V]** |
| **Tauri / Dioxus / WebView2** | No | WebView2 runtime is not guaranteed on older Win10; Linux WebKitGTK has officially-documented NVIDIA/DMABUF blank-window and resize-crash failures with silent software-rasteriser fallback **[V]** |
| **Qt (the klogg path)** | No | 12–19MB Windows, 37–42MB Linux AppImage, **not a single file**; won't look modern without heavy styling. *(The LGPL static-linking blocker cited in research does **not** apply now the project is open source — see §11.)* |
| **WinUI 3** | No | Loses on **size and startup**, and Rust interop. *(Both the "requires a runtime install" and "can't be single-file" claims are **[REFUTED]** — self-contained deployment and `PublishSingleFile` have been supported since Windows App SDK 1.5.)* **[V]** |
| **wgpu (as GPU abstraction)** | No | Measured ~576ms init before first pixel (Instance 202ms, Adapter 143ms, Device 85ms, Surface 140ms), closed as not planned. Documented adapter-enumeration failures on VMware VMs and driver-update `DXGI_ERROR_DEVICE_REMOVED` panics — exactly the RDP/VM/jump-box environments a log tool runs in. **[V]** |

### 3.4 Things true regardless of stack

- **The virtualised million-row grid must be hand-written under every candidate.** No toolkit provides
  one. This is a named 4–8 person-week component. **[L]**
- **The scrollbar must be driven by a `u64` line index, never an f32 pixel offset**, and **no f32
  accumulation anywhere in scroll state.** egui issue #1391 reports jitter above ~2M rows and
  breakage above 100M; the reporter *speculates* f32 precision but explicitly says *"I haven't been
  able to track it down yet."* **[V] as of 2026-07-29 — G7 reproduced it and identified the cause;
  see `experiments/g7-egui-scroll/RESULTS.md`.** The reporter's hypothesis was right in kind, and the
  specific answer changes what the rule has to say:
  - The culprit is **`ScrollArea::State::offset`, an f32 holding an absolute content-pixel
    coordinate**. The thumb mapping is *exonerated* — its forward f32 error measures exactly zero at
    every row count — and row-height accumulation is not a cause because egui never accumulates.
  - It breaks in **two independent ways**, and fixing one leaves the other. (1) `state.offset -=
    delta` discards any delta below half a ULP, so a 2 px/frame drag moves 0 px at 4M rows and the
    wheel stops responding entirely past ~100M. (2) `show_rows` positions rows as
    `(inner_top - offset) + min_row as f32 * row_h` — two content-magnitude f32s differenced to give
    a sub-row result, which flings the first row across a 512 px band at 160M rows.
  - **Failure (2) survives an exact scroll position.** This is the trap the caveat below anticipated,
    but worse than stated: it is not the delta accumulation, it is the *layout* conversion, so
    adopting "u64 thumb" and computing `row * row_h - offset_px` afterwards reproduces the entire bug
    inside an otherwise correct grid. `SPEC.md` §6.4 now carries the derived rule.
  - Both reported thresholds are **predicted from the arithmetic alone** — 2.0M for the onset and
    63M–300M for "very broken" — from a model never fitted to them.
- **Use D3D11, not D3D12/Vulkan/wgpu.** Guaranteed available Windows 7+, has a WARP software
  rasteriser fallback, smaller memory footprint. Plan an explicit chain: D3D11 hardware → D3D11 WARP →
  Direct2D/DXGI. Never panic on `DXGI_ERROR_DEVICE_REMOVED`. **[V]**
- **Detect RDP (`GetSystemMetrics(SM_REMOTESESSION)`) and switch rendering strategy.** Over RDP a DXGI
  swapchain Present pushes a full composed framebuffer through the remote protocol; GDI/D2D-on-DC lets
  RDP forward drawing primitives and use scroll-blit. At 5,000 lines/s over a 5Mbps VPN, a 60Hz
  swapchain saturates the channel. **This is a v1 requirement, not an optimisation.** **[L]**
- **Compile shaders offline** with fxc/dxc and embed bytecode; runtime compilation introduces a
  `d3dcompiler_47.dll` dependency that breaks the zero-dependency claim. **[L]**

---

## 4. Cross-platform, and the agentic development thesis

### 4.1 The thesis under test

Stated by the owner: *"In a traditional model we reach for cross-platform libraries and frameworks,
which tend toward a lowest-common-denominator result. We only do that because writing separate native
apps is too much work. In an agentic flow, we could probably more effectively write SEPARATE apps that
share common code in places, but which are NOT lowest-common-denominator."*

### 4.2 Verdict: half right, and the wrong half is instructive

A proponent, an opponent and an adversarial judge were run. The judge **cloned Ghostty** (commit
`a60cd15`, 27 Jul 2026) to check the proponent's evidence rather than accept it.

The proponent argued that a log viewer's shell is one hard custom control, so the grid should be
written three times against DirectWrite / Core Text / Pango — the narrowest possible target for
agent-assisted porting — and cited Ghostty as proof.

**Ghostty does the opposite. [V]** `src/renderer/` (Metal, OpenGL, WebGL, shaders) and `src/font/face/`
(coretext, freetype) are **all in the shared core**. Grepping `CTLine`/`CTRun`/`CTFrame`/
`NSAttributedString` across `macos/Sources` hits only `TerminalWindow.swift` (window styles),
`UpdatePill.swift`, `InspectorView.swift` and `TabTitleEditor.swift`. `SurfaceView_AppKit.swift`
(2,411 lines) is input, event and IME plumbing around a surface — **not a text renderer**. Ghostty
writes the one hard control **once**, behind per-platform leaf backends, and hands the shell a
drawable.

Measured splits (judge's own counts, reproducing the proponent's raw numbers exactly):

| | Lines |
|---|---|
| `src/` total Zig | 281,897 (535 files) |
| — of which inline tests | 89,361 (**31.7%**) |
| `src/terminal/` | 127,175 — **but 68,113 of that is test code (54%)** |
| macOS Swift shell | 36,039 (188 files) — 32,340 Sources / 2,792 Tests |
| Linux GTK4 shell | 22,481 |
| C ABI (`ghostty.h` + `include/ghostty/*.h`) | ~1,372 lines |

Production-code-only shell share: **24.6%**, not the 18.4% claimed.

Three further corrections from the judge:

- **The C ABI is thin *because* the core renders the pixels.** The moment Ghostty exposed grid model
  data to embedders, headers exploded — `terminal.h` 1,766, `selection.h` 1,061, `kitty_graphics.h`
  859, `render.h` 795. You can have a 1.2k-line C ABI *or* a shell-side renderer, **not both**. The
  proponent's design asks for both while citing the thin number. **[V]**
- **The LOC ratio does not transfer anyway.** Ghostty's favourable split is carried by its VT/ANSI
  state machine, which has no analogue here. Honest estimate for this app: ~18k engine vs ~11k per
  shell → three shells would be **~65% shell code**, the inverse of Ghostty.
- **Ghostty is weaker proof than claimed.** It officially distributes prebuilt binaries **for macOS
  only**, relies on community maintainers for Linux, and `src/apprt/` contains gtk, embedded, none and
  browser — **there is no Windows backend**, which happens to be this product's primary target. **[V]**

### 4.3 What survives of the thesis

The core insight holds and is worth stating plainly: **agents genuinely do collapse the cost of
authoring shell code, and the traditional lowest-common-denominator trade is therefore obsolete.**
What the evidence changes is *where the seam goes* — at the **rasteriser**, not at the widget layer.
That seam is far cheaper and it does not triplicate the risky code.

What does **not** collapse is verification. Agents collapse authorship, not reproduction, review, or
device-dependent verification. A solo Windows developer cannot review AppKit, cannot hear VoiceOver,
cannot see a Retina panel, and cannot feel trackpad momentum. **That is not an argument against the
core/shell split — the split pays for itself on one platform. It is an argument against shipping
shells you cannot verify.**

### 4.4 Cross-platform demand evidence

klogg ships per-platform assets on one identically-promoted release. Download counts on v22.06 **[V]**:

| Platform | Downloads | Share |
|---|---|---|
| Windows | 111,860 | **76.7%** |
| macOS | 17,920 | 12.3% |
| Linux | 16,133 | 11.1% |

For scale: lnav's *entire annual* Homebrew install volume is 12,780 — about a tenth of one klogg
Windows release. Debian popcon: lnav 1,747 installs (172 regular users), multitail 2,711 (403). **[V]**

Read honestly, this cuts both ways — the incumbent Unix tools have modest, mostly-dormant install
bases, so "lnav already exists" is a weak objection; but the addressable market on those platforms is
small in absolute terms. Neither number supports treating Linux/macOS as co-equal targets.

### 4.5 Cost of shipping macOS and Linux anyway

- **macOS breaks the single-executable promise outright.** The deliverable is necessarily a universal
  (lipo'd arm64+x86_64) `.app` bundle, Developer ID signed with Hardened Runtime, notarized via
  `xcrun notarytool`, stapled, in a signed DMG. Unsigned is not a fallback: arm64 macOS **SIGKILLs**
  unsigned binaries, and macOS 15 Sequoia removed the Control-click Gatekeeper override. **[V]**
- **Linux cannot be a fully static GUI binary.** libGL/EGL, libwayland-client and libxkbcommon are
  `dlopen`'d from the host. Flatpak's default sandbox has **no host filesystem access** and Snap needs
  manually-approved classic confinement — both fatal for a log viewer. Plain tarball + AppImage are the
  only honest channels, and AppImage needs libfuse2, which Ubuntu 22.04+ no longer installs by
  default. **[V]**
- **Recurring costs:** Apple Developer Program, an Apple-hardware CI runner (GitHub macOS minutes bill
  at **$0.062/min vs $0.006 Linux — a 10.3× multiplier**), notarisation in the release pipeline, and
  5–10 engineer-days/year of unplanned Apple/Ubuntu breakage. **[V]**

### 4.6 Decision

**Windows only for v1.** Build the core/leaf-backend seam because it is cheap and pays for itself on
one platform. Treat a second shell as a bounded, gated experiment against a pre-agreed demand signal —
not as a roadmap item.

---

## 5. File handling and performance engineering

### 5.1 Reading strategy — do NOT memory-map

This was the single largest contradiction in the research (§11) and it resolves decisively **against**
mmap.

The decisive reason is Windows-specific and product-fatal: **a `CreateFileMapping` section handle
blocks `DeleteFile`.** Len Holgate documented and shipped this deliberately as a *feature* to make a
log renameable but not deletable. **[V]** So a mapped view means:

> A log4net `RollingFileAppender` with `maxSizeRollBackups=5` tries to delete the oldest backup, gets
> `ERROR_SHARING_VIOLATION`, swallows it into its internal `LogLog`, and **rolling silently stops**.
> The customer's log directory fills the disk and this app is the cause.

"Tear the section down the instant rotation is suspected" is **not implementable** — you cannot detect
the writer's `DeleteFile` before it fails, because the failure *is* the detection event. **[V]**

Three more independent reasons:

- MSDN: *"A mapped view of a file is not guaranteed to be coherent with a file that is being accessed
  by the ReadFile or WriteFile function."* Log writers all use `WriteFile` — the live tail region is
  exactly where coherency is not guaranteed. **[V]**
- Truncation under a view raises `EXCEPTION_IN_PAGE_ERROR`, which **Rust cannot catch portably** (no
  stable SEH). Copy-truncate rotation triggers this routinely. **[V]**
- Mapping size is frozen at `CreateFileMapping` time — a growing file needs constant re-mapping, and
  `CreateFileMapping` fails on a zero-byte file with `ERROR_FILE_INVALID`. **[V]**

Sublime HQ published a post-mortem reaching the same conclusion for the same reasons and retreated to
`pread`, measuring it at ~2/3 the speed of mmap on Linux — irrelevant for a page-cache-hot tail. **[V]**

**Decision:** overlapped buffered `ReadFile` everywhere. `CreateFileW(path, GENERIC_READ,
FILE_SHARE_READ|FILE_SHARE_WRITE|FILE_SHARE_DELETE, NULL, OPEN_EXISTING,
FILE_FLAG_SEQUENTIAL_SCAN|FILE_FLAG_OVERLAPPED, NULL)`, with 4–8 outstanding 1–4MB requests on an IOCP.

### 5.2 Sharing modes — the highest-value correctness decision

Windows sharing is a **bilateral** check evaluated at every `CreateFile`. **[V]**

- `FILE_SHARE_DELETE` is **mandatory** — MSDN: *"Delete access allows both delete and rename
  operations."* Without it the app blocks the writer's own rotation. **This is a real, reported bug:
  klogg issue #36, blocking Python's `RotatingFileHandler`.** **[V]**
- `FILE_SHARE_WRITE` is mandatory — omitting it is the classic .NET bug (`FileStream` defaults to
  `FileShare.Read`).
- The app can only open a file whose **writer** granted at least `FILE_SHARE_READ`. Nothing on the
  reader side fixes this. On `ERROR_SHARING_VIOLATION (32)`, name the writer-side remedy explicitly:
  Serilog `shared: true`, NLog `keepFileOpen="false"` / `concurrentWrites="true"`, log4net
  `<lockingModel type="log4net.Appender.FileAppender+MinimalLock"/>`. **[V]**

**Product guarantee, and it is testable:** *this app never prevents your application from writing,
rotating or deleting its own logs.* Automated test: run rename-and-recreate and copy-and-truncate
rotation loops while attached, assert no `ERROR_SHARING_VIOLATION (32)` or `ERROR_USER_MAPPED_FILE
(1224)` on the writer side.

### 5.3 Line indexing

> ### ⚠ GPL contamination notice — read before using this section
>
> An earlier draft reproduced klogg private implementation identifiers (member-variable names and
> internal constants). That is evidence **GPLv3 implementation source was read**, in the same section
> that mandates a clean-room process — self-defeating, and it left no record of who read what. The
> identifiers have been removed.
>
> **Treat this section as contaminated.** Before implementation, the design must be **re-derived
> independently from published documentation only**, by someone recording the date and artifacts
> consulted in `CLEANROOM.md`. Do not implement from this section directly. Block-sparse
> delta-encoded offset indexes are a decades-old, independently-obvious technique — the risk is not
> the idea, it is the appearance of copying.

**Design shape** (obtainable from klogg's public documentation and the streamvbyte README): a
block-sparse index; a fixed number of lines per block; an absolute `u64` anchor per block; intra-block
offsets stored as **deltas**, exploiting the fact that end-of-line offsets increase monotonically;
constant-time lookup by decoding a single block. klogg's public documentation describes a 128-line
block and the use of Lemire's streamvbyte codec. **[L]** — downgraded from [V] because the verifying
read is exactly the read that must not inform the implementation.

Arithmetic for a 10GB file at ~200 B/line = ~50M lines **[L]**:

| Scheme | Index size |
|---|---|
| Naive `Vec<u64>` | **400MB** — unacceptable |
| Delta-encoded, 1 byte/line common case | ~50MB |
| Block-sparse (K=128, u64 anchor + deltas) | **~56MB (~0.56% of file)** |
| Fully sparse (anchor per 4096 lines + in-block rescan) | **~100KB** |

Two caveats the critics raised:

- **streamvbyte is a C library.** Pulling it via `cc-rs` reintroduces a C toolchain, contradicting the
  no-C-dependency advantage claimed over Hyperscan. Hand-rolling group-varint in safe Rust is a few
  hundred lines and decode speed is irrelevant at UI scroll rates. **[L]**
- **Unhandled overflow:** streamvbyte encodes u32, so a 128-line block spanning >4GB cannot be
  represented. Not hypothetical — 128 lines averaging 40MB (request/response body logging) is 5GB in
  one block. Specify a fallback to absolute u64 offsets, flagged in block metadata. **[L]**
- **GPL hazard:** klogg is GPLv3. Specify the algorithm from published header comments and
  documentation, **not** by reading the `.cpp`, and record who read what. **[V]**

### 5.4 Following — polling is the correctness mechanism

**The most important finding in the whole body**, and it is forced by Windows regardless of any
cross-platform ambition:

> MSDN, on `FILE_NOTIFY_CHANGE_SIZE` and `FILE_NOTIFY_CHANGE_LAST_WRITE`: *"The operating system
> detects a change in file size only when the file is written to the disk. For operating systems that
> use extensive caching, detection occurs only when the cache is sufficiently flushed."* **[V]**

A buffered .NET writer (Serilog, NLog, log4net) therefore produces **no notification at all** until
flush or close. Microsoft's own archived FileSystemWatcher guidance confirms it: *"The metadata is not
flushed until the FileStream.Close method is called; at which time notifications are picked up by the
native API ReadDirectoryChangesW."* **[V]**

**And you must poll the open handle, not the path.** Raymond Chen: NTFS replicates file metadata into
the directory entry lazily — since Vista, on last-handle-close. So `GetFileAttributesEx`/`FindFirstFile`
on the path return a **frozen size forever** for a file another process holds open. Poll
`GetFileSizeEx` or `GetFileInformationByHandleEx(FileStandardInfo)` **on our own handle**. **[V]**

Other documented `ReadDirectoryChangesW` failure modes **[V]**: buffer overflow returns TRUE with
`lpBytesReturned == 0` and **discards the entire buffer**; `ERROR_INVALID_PARAMETER` if the buffer
exceeds 64KB over the network; `ERROR_NOTIFY_ENUM_DIR` when changes could not all be recorded;
`ERROR_INVALID_FUNCTION` on redirectors that don't support it; does not report changes to the watched
directory itself.

**Decision:** poll as ground truth — 100ms local, 250–500ms UNC, adaptive back-off to 1s when idle
>30s, instant snap-back on change. **Cut `ReadDirectoryChangesW` from the v1 tail path entirely** — it
delivers nothing for the primary workload (buffered writers) while costing real complexity. Keep a
directory watch only for the case it is genuinely good at: noticing a **new file matching a glob**
has appeared, where `FILE_NOTIFY_CHANGE_FILE_NAME` does fire reliably. **[L]**

**Per-tick work budget, not just an interval.** At 10–50MB/s append, a 250ms tick delivers 2.5–12MB —
potentially 100k+ new lines to index, filter and lay out in one frame. Cap work per tick at a byte
budget (~4MB) and carry the remainder forward. Acceptance test: 50MB/s sustained for 60s. **[L]**

### 5.5 Rotation and truncation

Track `(VolumeSerialNumber, FILE_ID_128)` from `GetFileInformationByHandleEx(FileIdInfo)`. MSDN: *"The
file identifier and the volume serial number uniquely identify a file on a single computer."* The
older 64-bit `nFileIndexHigh/Low` is **not unique on ReFS**. **[V]**

| Case | Detection | Action |
|---|---|---|
| Copy-truncate | size < last read offset | Reset to 0, draw a "file truncated" separator |
| Rename-and-recreate | path's file ID ≠ held handle's | **Drain the old handle to EOF first**, then switch — this is where naive tools lose the last KB |
| Path disappears | open fails | Keep retrying (`tail -F` semantics), show a "waiting for…" placeholder |

**Never key tab identity or dedup on the path string.** Windows has per-directory case sensitivity
since Win10 1803 (and WSL-created directories are case-sensitive and cannot be made insensitive), while
macOS APFS is case-insensitive by default. Path-string comparison produces both false merges and false
splits. **[V]**

### 5.6 Encoding

**Pipeline, in strict order.** Encoding must be resolved **before** indexing, because UTF-16 newline
scanning looks for `0A 00`/`00 0A`, not `0A`.

1. **BOM sniff, longest match first** — `FF FE 00 00` UTF-32LE **before** `FF FE` UTF-16LE, or every
   UTF-32LE file is misread. `EF BB BF` UTF-8; `00 00 FE FF` UTF-32BE; `FE FF` UTF-16BE. **[L]**
2. **NUL-position statistics** over a 64KB head sample **and** a 64KB tail sample. chardetng — the
   detector Firefox ships — **does not detect UTF-16 or UTF-32 at all**; the Encoding Standard
   deliberately excludes them. This probe must be hand-written. **[V]** This matters on Windows because
   PowerShell 5.1's `>` and `Out-File` default to UTF-16LE.
3. **Strict UTF-8 validation** — essentially no false positives over a few KB. Pure ASCII → declare
   UTF-8.
4. **Only then** chardetng, falling back to `GetACP()`.

**Incremental decoding:** one long-lived `encoding_rs::Decoder` per followed file for the tail stream,
**never reset between appends** — it owns the partial-sequence carry across read boundaries. Separate
short-lived decoders for viewport reads, starting from a back-aligned safe boundary (UTF-8: back up
≤3 bytes to a non-continuation byte; UTF-16: align to even offset, handle a lone surrogate). **Store
byte offsets only** in the index. **[V]**

**Mixed/ambiguous files:** decide once from head+tail samples; if they disagree, prefer the tail (it's
what's live) and flag it in the UI. Decode with U+FFFD replacement rather than erroring. **[L]**

**Line terminators:** treat the set as `{CRLF, LF, CR}` with a one-byte pending-CR carry across chunk
boundaries. A fixed-size-chunk reader **will** eventually split CR from LF and emit a spurious blank
line at the exact moment a new chunk arrives — a bug that only manifests while tailing. **[L]**

### 5.7 Rendering

- **Word wrap OFF by default** with a fixed-pitch font, so line height is constant and vertical layout
  is O(1) at 50M lines. Wrap collapses this: visual-line count depends on every line's width, forcing
  measurement of all lines and re-measurement on every resize. **[L]**
- **Horizontal extent from `max_line_length × advance_width`**, capturing max line length per 128-line
  block **for free** during the index pass. Never from currently-visible lines — that causes the
  documented horizontal-thumb jitter (AvalonEdit #282, Zed #10809). **[L]**
- **Decouple ingest from render** with a bounded lock-free SPSC ring; coalesce to exactly **one
  invalidate per 16.67ms tick** regardless of arrival rate. Under overload drop *rendering*, not
  *data* — resync to current tail offset with a "skipped ahead" indicator. **[L]**
- **Batch per-line work into per-block work.** klogg measured ~20% purely from moving encoding
  conversion from per-line to per-1000-line blocks, and a further ~10% from swapping allocator (TBB →
  mimalloc). **Allocation per line is the dominant hidden cost in log viewers.** **[V]**
- **Text layout must be per-run, not per-glyph.** Building an `IDWriteTextLayout` per line per frame
  re-runs shaping and itemisation every frame. Use DirectWrite only to rasterise each unique glyph once
  into a persistent atlas, then one instanced draw per visible cell with colour as a per-instance
  attribute. A viewport is ~200×60 = ~12k cells. **[V]**

**The monospace cell model is not safe as stated [critic finding].** CJK and East Asian Wide characters
are double-width; combining marks and Devanagari/Thai clusters mean code points ≠ grapheme clusters ≠
cells; ZWJ emoji sequences are many code points rendering as one or two cells; **colour emoji cannot
come out of a monochrome alpha atlas at all** (DirectWrite needs `TranslateColorGlyphRun`). Font
fallback compounds it — Consolas has no CJK, and a fallback font's advance width will differ. Modern
CI, Node and Go logs are full of check marks, crosses and warning emoji. **Grapheme-cluster
segmentation and East Asian Width must be in the cell model from day one.** **[L]**

### 5.8 Search

- Use the Rust **`regex`** crate, not Hyperscan. Same author as ripgrep; lazy DFA, rare-byte memchr
  prefilters, Teddy SIMD multi-pattern — the exact tricks klogg imports Hyperscan for, with no
  Boost/Ragel/C++ dependency. Hyperscan doubled klogg's CI build time for ~2× over PCRE2. **[V]**
- **Engine policy must be explicit.** `regex` has **no timeout or step-limit API** — "clamp regex work
  per line with a timeout" is not implementable as written, and it doesn't need one (linear-time
  guarantee). `fancy-regex` (for lookaround/backreferences, which log4net/NLog users write routinely)
  **is** a backtracking engine: give it an explicit `backtrack_limit`, a hard per-line input cap
  (e.g. first 8KB), a cancellable worker thread, and a visible "pattern too slow, truncated"
  indicator. Set `RegexBuilder::size_limit`/`dfa_size_limit` explicitly. **[L]**
- **Parallel chunked search:** snap chunk boundaries to newlines using the line index — zero overlap
  needed for line-oriented patterns. Only multiline patterns need overlap ≥ max match length with
  start-offset dedup. **[V]**
- **No persistent search index.** A full re-scan is cheaper than index construction and any index is
  invalidated by every append. **[L]**
- **Search raw bytes where possible.** klogg's own profile shows line *decoding* becomes ~50% of search
  time once the regex engine is fast (907ms of an 1805ms search on 1GB). **[V]**

**Parallel newline indexing is NOT context-free [REFUTED].** The research asserted chunk boundaries can
be assigned arbitrarily. False for two encodings we differentiate on: a boundary at an **odd** byte
offset misaligns every UTF-16 code unit in that chunk (finding `0A` bytes that are the low half of an
unrelated code unit, missing real `0A 00` terminators); ~~and in DBCS codepages (932/936/950/949)
`0x0A` is a **legal trail byte**, producing phantom line breaks~~ — **the DBCS half is withdrawn,
session 8: measured false.** `a_0a_byte_is_never_consumed_as_a_trail_byte_by_any_decoder`
(`crates/tailhawk-core/src/encoding.rs`) drives all 65,536 two-byte prefixes into every byte-oriented
decoder; `0x0A` is never swallowed as a trail byte. See `SPEC.md` §5.3. Concretely: a 4GB PowerShell UTF-16LE
transcript indexed on 8 threads at 64MB chunks — seven boundaries land on even offsets by luck, one
does not, and ~500MB has garbage line offsets that only surface when scrolled into. **Chunk boundaries
must be aligned to the code-unit size relative to the BOM. That is the only constraint** — there is
no DBCS exception and the parallel path is disabled for no byte-oriented encoding. **[V]** *(The
original form of this requirement disabled the parallel path for codepages 932/936/950/949; that was
withdrawn in session 8 and the correction reached this document in session 15.)*

---

## 6. Log format catalogue and auto-detection

### 6.1 The formats (verified constants)

**Serilog — file sink default** **[V]**
`{Timestamp:yyyy-MM-dd HH:mm:ss.fff zzz} [{Level:u3}] {Message:lj}{NewLine}{Exception}`
No brackets around the timestamp; line starts with the date.
```
2026-07-28 09:14:02.117 +01:00 [INF] Started HTTP request GET /api/contacts responded 200 in 14.2010 ms
```

**Serilog — console sink default** **[V]**
`[{Timestamp:HH:mm:ss} {Level:u3}] {Message:lj}{NewLine}{Exception}`
Brackets around **both**, time-only. **Materially different shape — ship as a separate detector.**

**Serilog level tokens** **[V]** — `:u3` → `VRB DBG INF WRN ERR FTL`; `:u4` → `VERB DBUG INFO WARN
EROR FATL`. **`EROR` and `FATL` are Serilog-specific misspellings emitted by no other framework — a
near-certain positive fingerprint.**

**Serilog CLEF** **[V]** — NDJSON with `@`-prefixed reserved keys: `@t` (timestamp, **the only required
field**), `@m`, `@mt`, `@l` (**absent ⇒ Information**), `@x`, `@i`, `@r`, `@tr` (trace id), `@sp` (span
id). Detection: line starts `{` **and** contains the literal `"@t":` — a near-zero-false-positive
byte-level `memmem` test, done **before** any JSON parse.

**log4net** **[V]** — `DefaultConversionPattern = "%message%newline"`;
`DetailConversionPattern = "%timestamp [%thread] %level %logger %ndc - %message%newline"`.
`%date{ISO8601}` → `yyyy-MM-dd HH:mm:ss,fff` — **comma before milliseconds is a strong log4net/log4j
fingerprint**. Key discriminators: the ` - ` separator before the message, and 5-char padded levels
(`INFO `, `WARN `).

**NLog** **[V]** — default `${longdate}|${level:uppercase=true}|${logger}|${message:withexception=true}`.
`${longdate}` renders **`yyyy-MM-dd HH:mm:ss.ffff` — FOUR fractional digits**. Nothing else in .NET
emits `.ffff` by default; combined with pipe delimiters this is the strongest NLog fingerprint. Levels
are `Trace Debug Info Warn Error Fatal` — note `Info`/`Warn`, not `Information`/`Warning`.

**Microsoft.Extensions.Logging Simple console** **[V]** — the most distinctive .NET shape, and
**multi-line by construction**. Levels render as `trce dbug info warn fail crit` — **`fail` for Error
and `dbug` for Debug are unique fingerprints**. Header line is `level: Category[EventId]`, continuation
lines indented by **6 spaces** (`"info".Length + ": ".Length`).
```
info: Microsoft.Hosting.Lifetime[14]
      Now listening on: https://localhost:5001
```
Colours by default when not redirected — **strip ANSI CSI before format matching**.

**MEL Json** **[V]** — fixed PascalCase schema; the literal key `"{OriginalFormat}"` is unique to MEL
message templates. Note `JsonWriterOptions.Indented = true` (which Microsoft's docs actively
recommend) produces **pretty-printed multi-line JSON — not NDJSON**, requiring a brace-depth streaming
scanner.

**W3C Extended / IIS / ASP.NET Core `W3CLogger`** **[V]** — **self-describing; never guess.** Scan for
`^#Fields:\s*(.*)$`, split on whitespace, use verbatim as column names. **`#Fields` can appear
mid-file** — IIS re-emits it after config change or rotation — so re-read column definitions whenever
a new directive appears. Spaces in values are substituted with `+`.

Also catalogued with samples and extraction regexes: **log4net XmlLayout / log4j XML** (fragment
streams with no root element), **syslog RFC 3164/5424**, **MEL Systemd** (`<PRI>` prefix — disambiguate
from syslog by the absence of a version digit), **JSON Lines / NDJSON**, **logfmt**, **CSV/TSV**,
**Apache/nginx combined**, **Docker json-file**, **Kubernetes CRI**, **wevtutil text**, **systemd
journal export**, **OTLP/JSON**.

### 6.2 Auto-detection — five-stage pipeline

**Every stage bounded by BYTES, not lines**, so a 40GB file opens as fast as a 40KB one. (lnav's
head-only 15,000-*line* scan is its weakest point.) **[L]**

0. **Encoding** (§5.6) — must precede everything, since format regexes run on decoded text.
1. **Sample window** — 256KiB head, **plus ~64KiB from the middle and ~64KiB from the end**. File heads
   are systematically unrepresentative (startup banners, config dumps), and for a tailing tool the tail
   is what the user cares about.
2. **Self-describing short-circuits** — `memmem` tests, no scoring: `#Fields:` → W3C, STOP.
   `<log4net:event`/`<log4j:event` → XML stream, STOP. `^Event\[\d+\]:` → wevtutil, STOP. `__CURSOR=` →
   journal, STOP. `{` + `"@t":` → CLEF, STOP. `{` + `"{OriginalFormat}"` → MEL Json, STOP.
   `"resourceLogs"` → OTLP/JSON, STOP. Docker/CRI envelopes → **unwrap and recurse**.
3. **Scored regex matching** — four terms, not one:
   `score = match_rate × (0.5 + 0.5×field_validity) × specificity × (0.7 + 0.3×coverage)`
   **`field_validity`** (does the captured timestamp actually parse to a real datetime; is the captured
   level actually in the declared set) is the term that kills the classic false positive where a loose
   `<date> <word> <rest>` pattern matches but the "level" is the word `Starting`.
   **Accept only if score ≥ 0.75 AND the winner beats the runner-up by ≥15%.** Otherwise show a
   disambiguation chip. *Silent mis-columnising is worse than no columnising.*
4. **Specificity ordering** — RFC 5424 0.95 > NLog 0.90 = MEL Simple 0.90 > Serilog file 0.85 =
   Apache/nginx 0.85 > log4net 0.80 > MEL Systemd 0.75 > RFC 3164 0.70 > Serilog console 0.65 >
   NDJSON 0.55 > logfmt 0.45 > CSV 0.40 > generic timestamped 0.20 > plain text 0.00.
5. **Runtime resilience — do NOT copy lnav's permanent lock-on.** Track a rolling non-match rate over
   the last ~1,000 records excluding recognised continuations; above ~20% offer non-modal re-detection.
   Rotated and concatenated files genuinely change shape mid-stream.

**Ordering correctness becomes a unit test, not a runtime heuristic:** ship every format with `sample`
lines (lnav's best idea — self-testing definitions), and at **build time** cross-match every format's
samples against every other format's pattern. Fail the build if a generic format outscores a specific
one on the specific one's samples. **[V]**

### 6.3 Multi-line records

**The universal rule: a new record starts IFF the line matches the format's first-line anchor.**
Everything else is a continuation. **[V]**

- Every format regex must be `^`-anchored and match **only** a first line, so a stack-trace line
  containing a date mid-string doesn't falsely start a record.
- **One-byte dispatch table on the first non-space character** (digit, `[`, `<`, `{`, or a level token)
  eliminates >95% of continuation lines with zero regex work. **The single biggest parser perf win.**
- Ship per-family continuation predicates so exception blocks can be styled and **collapsed** — .NET
  `^\s*(at\s|--->\s)`, Java `^\s*(at\s|Caused by:)`, Python `(Traceback|^\s{2,}File ")`, MEL `^ {6}`.
  Neither Hoo WinTail nor LogExpert offers stack-trace collapse.
- **While tailing, the final record is ambiguous.** Hold it provisional, render it, revise in place —
  do not wait for a following first-line match, which would make the tail appear one record behind.

**Three classes of non-matching line, never conflated:** CONTINUATION (append, dimmed, inside the
expandable row); **INTERLEAVED FOREIGN** (matches a *different* known format — multi-format files are
the norm on Windows; parse it, badge the row); UNPARSEABLE (own row, raw line in Message, gutter
stripe, filterable). **Never drop a line.** Status bar shows parse health: `99.2% parsed · 812
continuation · 14 unparsed` — far more actionable than a confidence percentage the user can't check.

### 6.4 User-defined formats

Three tiers, no compiled plugins ever:

1. **Override** — a format chip showing detected format and confidence, remembered per path **and per
   glob**.
2. **Define from example** — right-click a line → proposed tokenisation → live preview over the next
   200 lines with a match-rate readout. Generates a **Loki-style pattern DSL** string
   (`<ts> [<thread>] <level> <logger> - <message>`, `<_>` discards), not a regex. Compiles to a linear
   scanner, no backtracking. "Edit as regex" as the advanced escape hatch.
3. **Template import — unique to the .NET audience and cheap.** Paste the layout string you already
   have — a Serilog `outputTemplate`, an NLog `layout`, a log4net `conversionPattern`, a Logback
   pattern — and compile it to the extraction pattern and column set. Better still: **"Scan for
   logging config"** walks up from the log file's directory for `appsettings.json`, `NLog.config`,
   `log4net.config` and offers the layouts it finds. **Nothing else on Windows does this.**

Storage: `<app>.formats.toml` next to the exe, plus `%APPDATA%`, exe-adjacent winning — **but see the
stateless-mode requirement in §10.**

---

## 7. Unix `tail` parity

### 7.1 Two-mode binary

`--stdout` (headless) mode must be a **byte-exact GNU `tail` clone** — same defaults (`-n 10`,
`--follow=descriptor`, 1.0s sleep), same `==> file <==` header rule, same stderr diagnostics (`file
truncated`, `has been replaced; following new file`, `no files remaining`), same exit codes. GUI mode
then gets sane defaults that deliberately differ, with every divergence reachable via an explicit flag.
This makes "full parity with Unix tail" **testable rather than marketing**.

### 7.2 Where GUI defaults must diverge

| | `tail` | The app |
|---|---|---|
| Initial load | 10 lines | Last ~1MB / ~10k lines — enough to fill the viewport plus scrollback |
| Follow mode | `descriptor` | **`name` (i.e. `-F`)** — 100% of real rotation cases want it |
| Missing file | error | **Wait** — a "waiting for `C:\logs\app.log`" placeholder that becomes content |
| Poll interval | 1.0s | 100ms local / 250–500ms UNC |
| Headers | `==> file <==` | Tab titles / source gutter in merged view |

### 7.3 CLI design

**Reserve every short letter GNU or BSD `tail` claims — `b c f F n q r s v z` — and never redefine
them.** Stealing `-F` for "fixed strings" or `-f` for "filter" would be the single most damaging
possible CLI decision. All app-specific options get long-only names.

**Subsystem:** `/SUBSYSTEM:WINDOWS`, with `AttachConsole(ATTACH_PARENT_PROCESS)` for
`--help`/`--version`/parse errors. Document honestly that the prompt returns immediately, `ERRORLEVEL`
reflects process creation, and scripted use needs `start /wait` or `--stdout`. Do **not** ship a
`.com`/`.exe` pair — it violates single-file deployment. **[L]**

**Globs must be expanded by the app** — `cmd.exe` does not, and neither does Rust. Define an unmatched
pattern as *"watch the directory for files matching this pattern and adopt them as they appear"*,
which turns glob support into the folder-monitoring feature the owner actually uses. **[L]**

**stdin:** spill to a temp file rather than holding in memory — gives scrollback (a pipe is unseekable
and consume-once), reuses the index path, and sidesteps the fact that a pipe handle cannot be usefully
forwarded to a single-instance server. Detect with `GetFileType` (`FILE_TYPE_PIPE`/`FILE_TYPE_DISK` →
read; `FILE_TYPE_CHAR` → don't block). Treat `ERROR_BROKEN_PIPE` as stream-complete, **not** app-exit,
and flush the trailing partial line. **[L]**

**Scope warning from the critics:** byte-exact GNU emulation is realistically 3–5 weeks including a
differential test harness, for a feature whose users already have WSL, Git Bash and uutils. Consider
cutting to what people actually type — `-n`, `-f`, `-F`, `-c`, file/glob operands, plus `--stdout` —
and pointing at uutils in the docs. **[L]**

---

## 8. OpenTelemetry

### 8.1 Verdict

| Candidate | Verdict |
|---|---|
| **1. OTel log data model as the internal normalised record** | **Yes — v1.** Spec is Stable; pays for itself on cross-format severity filtering alone |
| **2. OTLP receiver on localhost** | **Defer past v1**, gate on evidenced demand |
| **3. Read OTLP/JSON files** | **Yes — post-v1** parser-registry entry |
| **4. Query remote backends (Loki/Tempo/Azure Monitor)** | **No.** Pure scope creep |

**Do stdin/pipe ingestion BEFORE any OTLP receiver.** `docker logs -f svc | app -` and
`az containerapp logs show --follow | app -` deliver the owner's stated container use case with no
listening socket, no protobuf dependency, and no trace/metric expectation. **[L]**

### 8.2 Why the receiver is deferred

It collides head-on with the **.NET Aspire Dashboard** — a free, MIT-licensed Microsoft product that
runs standalone as a container, receives OTLP directly, and handles logs, traces and metrics. It is
also the single feature most likely to reclassify the product from "tailer" to "mediocre observability
tool": users who see "OTLP receiver" will expect traces and metrics. **Write into the spec,
non-defensively, that Aspire Dashboard is the right tool for live containerised OTel debugging, and
that this app's value is files, scale, persistence and no-runtime Windows deployment.** **[L]**

If ever built: **off by default, opt-in per launch, bound to 127.0.0.1 only, logs-only**, with Windows
firewall/EDR consequences treated as a first-class spec concern — they directly undermine copy-and-run
positioning.

### 8.3 Why the data model is worth adopting

Twelve fields, **all optional**: `Timestamp`, `ObservedTimestamp`, `TraceId`, `SpanId`, `TraceFlags`,
`SeverityText`, `SeverityNumber`, `Body` (AnyValue), `Resource`, `InstrumentationScope`, `Attributes`,
`EventName`. **[V]**

**SeverityNumber is 24 values in six bands** — TRACE 1–4, DEBUG 5–8, INFO 9–12, WARN 13–16, ERROR
17–20, FATAL 21–24. The spec's own error predicate: *"If SeverityNumber is present and has a value of
ERROR (numeric 17) or higher then it is an indication that the log record represents an erroneous
situation."* That is the default "errors only" filter, working identically across every format. **[V]**

The spec provides **non-normative *example* mappings** in **Appendix B**, whose heading is
*"Appendix B: `SeverityNumber` example mappings"* and whose containing file opens by stating *"this
document is NOT a spec, it is provided to support the Logs Data Model specification."* They cover
RFC 5424 syslog (Emergency→21, Alert→19, Critical→18, Error→17, Warning→13, Notice→10,
Informational→9, Debug→5), log4j, Zap, **Windows Event Log** (Verbose→5, Information→9, Warning→13,
Error→17, **Critical→ERROR2 (18)**) and Apache access logs. The Collector additionally defines
HTTP-status aliases (2xx/3xx/4xx/5xx) — **reusable for giving W3C/IIS/nginx rows a severity**. **[V]**

**Two corrections an earlier draft got wrong**, both caught in adversarial review:
- These are **examples, not normative**. Adopting the banding is therefore a **design choice this
  project is making**, not an obligation it inherits — the leverage argument is unaffected, but the
  authority claim was overstated.
- **Windows Event Log `Critical` maps to 18 (ERROR2), not 21 (FATAL).** With 21, Event Log Critical
  rows would sort above every Serilog `ERR` *and* tie with `FTL` in a merged view, so a user filtering
  "FATAL only" would see events the spec does not classify as fatal.

Three structural wins:

1. **Banding, not a flat enum.** Rule 1 of the spec's mapping guidance — *"If the source format has
   more than one severity that matches a single range… assign numerical values from that range
   according to how severe the source severity is"* — is what lets syslog NOTICE (10), log4net's extra
   levels and Zap's DPanic/Panic coexist with Serilog levels in **one sortable cross-format ordering**.
2. **Rule 3 explicitly sanctions empty severity** for formats that have none. So W3C/IIS/nginx/logfmt
   rows leave both severity fields empty and remain spec-conformant — rather than fabricating INFO,
   which is what Grafana/Loki's documented bugs (#14443, #15444) do wrong.
3. **Resource vs Attributes maps onto pane vs row.** The spec designs Resource to be recorded once per
   batch from the same source — so syslog HOSTNAME and APP-NAME belong to the **tab/pane header**, not
   to every row. That is a free, principled answer to the merged-view column problem.

**Design it as a superset with raw-line passthrough**, so Serilog message templates and W3C-only fields
are not lost.

### 8.4 Trace correlation

This is the answer to the completeness critic's blocking gap — *"I have RequestId 0HN4G2…, show me
every line for it across these four services"* — which no researched tool addresses. Serilog CLEF
already emits `@tr`/`@sp` as W3C trace and span IDs; `System.Diagnostics.Activity` TraceId flows into
MEL scopes; W3C `traceparent` appears in ASP.NET Core request logs. The data is sitting there unused.

### 8.5 Positioning warnings

- **Do not position on "Rust", "single binary" or "OTLP-native".** OpenObserve (Rust, single binary,
  OTel-native, AGPL-3.0, ~20.4k stars) already owns all three. Differentiate on **desktop GUI plus file
  tailing**, which nothing in the OTel field touches. **[V]**
- **Benchmark against klogg, not Aspire.** klogg handles 10+GB files and >2³¹ lines with no OTel, no
  columnisation and no structured-log handling — the beatable incumbent in the lane actually occupied.

---

## 9. Distribution and signing

- **EV certificates no longer bypass SmartScreen.** Microsoft Learn, updated 2026-05: *"this behavior
  no longer exists… Paying a premium for EV solely to avoid SmartScreen warnings is no longer
  justified."* Both OV and EV show "unrecognized app" until reputation accrues — *"several weeks and
  hundreds of clean installs from a wide audience"*, with **no mechanism to request review** for
  consumer endpoints. **[V]**
- **Azure Artifact Signing is unavailable to this project.** Microsoft's quickstart (updated
  2026-07-23) states verbatim: *"Public Trust certificates are available to organizations in the
  United States, Canada, the European Union, the United Kingdom, Australia, New Zealand, Japan, South
  Korea, Singapore, Switzerland, Norway, and Israel. Individual developers must be located in the
  United States or Canada."* **The owner is South African, which is in neither list** — so both the
  individual and the organization paths are closed, and no legal-entity decision unlocks it. **[V]**
  *(An earlier draft of this document said "UK-based". That was inferred from an email domain, not
  stated by the owner, and was wrong.)*
- The previously-cited ~$10/month price is **moot** for this project, and was **[U]** anyway — the
  Azure pricing page shows `$-` for both tiers.
- **The remaining route is a conventional OV code-signing certificate from a commercial CA.** Since
  1 Jun 2023 the private key must live on FIPS 140-2 L2 / CC EAL4+ hardware, so this means either a
  shipped hardware token or a CA-hosted cloud-signing service. **[V]**
- **Certum's "Open Source Code Signing" certificate** is the known low-cost option aimed at exactly
  this case: from **€69.00 gross**, supplied as a set including a cryptographic card and reader, i.e.
  **hardware-token signing**. Its product page does **not** state geographic eligibility, and
  availability to a South African individual is **[U] — must be confirmed with Certum directly.** **[V]**
  for the price and token model; **[U]** for eligibility.
- **Keep one signing identity forever** — reputation resets with the certificate subject.
- **Since 1 Jun 2023** all publicly-trusted code-signing private keys (OV *and* EV) must live on FIPS
  140-2 L2 hardware; tokens run $90–250. **[V]**
- **Windows 11 Smart App Control** may supersede SmartScreen and *"will block execution of unsigned
  files unless the file has a positive reputation"*, with *"signature checks apply to all executable
  files, not just those downloaded from the Internet"*. Signing is not optional long-term. **[V]**
- **MOTW** is an NTFS alternate data stream (`Zone.Identifier`) — it does not exist on FAT/exFAT, so a
  copy to a USB stick loses it; but many archivers propagate it from a downloaded ZIP to extracted
  files. **[V]**
- **Channels:** **scoop first** (bucket JSON, zero friction for a portable exe, no signing
  requirement), **winget** second (`InstallerType: portable`, supported since 1.3 — several third-party
  guides claiming otherwise are out of date), chocolatey optional. Package managers do **not** confer
  SmartScreen reputation but drive the download volume that eventually builds it. **[V]**
- **Sysinternals Live** is reproducible (`\\live.sysinternals.com\tools\<tool>`, a WebDAV share over
  UNC) but requires the **WebClient service**, which on Windows 10/11 is installed but frequently **not
  running**. UNC paths are not automatically trusted — without a ZoneMap entry mapping the host to
  Local Intranet, MOTW logic still applies. **[V]**

---

## 10. Gaps the research never covered

The completeness critic's central finding: *"an outstanding implementation-research body wearing a
product-research body's clothes"* — roughly 70% of the surface a spec must cover was untouched. These
must be resolved in `SPEC.md`, not rediscovered later.

| Gap | Why it matters |
|---|---|
| **Merged-by-timestamp view** | Recommended six times, **specified zero times**. Unaddressed: timezones and DST (Serilog file carries `zzz`, console has no date, log4net `%date` is local vs `%utcdate`, RFC 3164 has no year or zone, W3C is UTC); differing sub-second precision (`.fff` vs `.ffff` vs 9-digit); clock skew across machines; out-of-order arrival from async writers (Serilog batching, NLog `AsyncWrapper` — records arrive out of timestamp order by up to the flush interval, so a live merge must insert **above** the view bottom, making the viewport jump); untimestamped lines; whether backward scroll needs a global merged index (contradicting the O(viewport) promise). **Needs a bounded reorder window (~2s) with a visually distinct "settling" band.** |
| **Trace/request-ID navigation** | The most common modern debugging task, absent entirely. Conflicts with the no-persistent-index rule — click-to-next-occurrence across four files interactively is not a one-shot scan. §8.4 gives the data model; the index decision is open. |
| **Remote & container sources** | Docker/CRI researched only as *line formats*, never as *sources*. Given §1.3 this is probably backwards. A **process-spawn source** (`docker logs -f`, `kubectl logs -f`, `az containerapp logs show --follow`) covers all of them with one mechanism — but introduces command execution, needing its own security decision. |
| **Settings for a portable exe** | "File next to the exe, else `%APPDATA%`" contradicts running from a **read-only UNC share**. BareTail's answer — file, registry, **or memory** — is the right model. Needs an explicit **stateless mode** with a visible "settings will not be saved" indicator. |
| **Sort on typed columns** | Presented as free; it isn't. Sorting 50M records needs a full-file parse pass (7–35s) plus 1.2–1.6GB of sort keys, and "follow tail" in a sorted view is meaningless. **Separate filtering (a cheap scan — ship it) from sorting (an external sort).** Consider top-N instead, which serves the real use case ("show me the slowest requests"). |
| **Export / copy / share** | One clause in the entire body. Hoo WinTail's **HTML export preserving highlight colours** and **write-matching-lines-to-file** — features the owner has used for years — fell out of the analysis silently. |
| **Bookmarks** | Three incumbents have them; omitted from every recommendation. Anchoring is unexamined: line numbers and byte offsets are invalidated by truncation and rotation. Needs content-hash anchoring. |
| **Very long lines** | klogg's "deadly hang" (#803) on a multi-MB line is cited as a competitor weakness with no product answer. 40MB single lines are routine (ASP.NET body logging, serialised exceptions). Needs a display cap with expand affordance and a JSON pretty-print detail pane. |
| **Compressed/rotated archives** | "Yesterday's log is now a `.gz`" is routine with NLog `ArchiveEvery` and log4net rolling appenders. lnav, Toolong and hl all handle it. gzip needs a zran-style access-point index for random access. |
| **Security of shared config** | The design *encourages* sharing format files and filter sets. A shared file contains **regexes** (a hang payload) and **paths** — a session containing `\\attacker\share` triggers outbound SMB auth and **leaks NTLM credentials for relay**, without the user typing anything. |
| **ANSI, bidi and invisibles** | Log content is frequently attacker-influenced (User-Agent, URI). If ANSI is rendered, **whitelist SGR colour only** — never OSC 8 hyperlinks or OSC 52 clipboard. Unicode bidi overrides (Trojan Source) make a rendered line say something different from its bytes; for a security-adjacent tool, **a viewer that can be made to lie is a real defect**. |
| **Accessibility** | "Link AccessKit" is not an answer. AccessKit gives platform adapters, **not a virtualised accessibility tree** — a 10M-line grid cannot emit 10M nodes. Needs hand-written `ITextProvider`/`ITextRangeProvider` (the hardest UIA pattern) — a 6–10 week workstream. Also unexamined: what a screen reader does while tailing at 1,000 lines/s; **High Contrast mode** (forces system colours, breaking every highlight rule); colour-blind safety of red/amber/green severity. |
| **Testing** | No fixtures, no fault injection, no fuzzing, no perf gate — and a custom-drawn D3D11 grid has **no UI Automation surface to test through**, tying testability to the deferred accessibility work. Needs a seeded generator (not checked-in data), a virtual-filesystem trait for deterministic rotation/truncation/disconnect injection, `cargo-fuzz` on every parser, and a dedicated perf box. |
| **Updating a portable exe** | Zero research despite being the headline differentiator. A running exe cannot be deleted but can be renamed; an exe on a shared UNC path cannot be replaced while colleagues hold it open; a downloaded update **must** have its Authenticode signature verified before execution or it is an auto-RCE channel. Defensible answer: no in-app updater, check-for-update link only, let scoop/winget do updates. |
| **Telemetry & privacy** | Not mentioned once. The app's entire working set is customer log files containing PII, connection strings and bearer tokens — so a conventional minidump reporter **exfiltrates customer data**. Recommended stance: **zero network traffic by default**, which is also a competitive claim worth making. |
| **Licence** | Never decided. Recommended: **MIT/Apache-2.0 dual** (Rust ecosystem norm). Needs `cargo-deny` with an allow-list, and a **clean-room rule** for anything derived from GPL prior art (klogg, TailBlazer, SnakeTail are all GPL). |
| **Platform envelope** | Never stated, and it silently drives many findings — the body cites APIs with floors from Windows 7 to 22621. Recommended: **Windows 10 1809+, x64 and ARM64, no x86**, with Windows 11 chrome applied opportunistically via runtime version checks. ARM64 needs a NEON dispatch path, separate signed artefact and separate perf numbers. |
| **Build/fork/contribute** | Never posed. Forking klogg (GPLv3, stale) or contributing to LogExpert (MIT, actively shipping, and its defect list reads like a contribution roadmap) are the two cheapest options. **Both fail the single-exe/no-runtime requirement** — which means that requirement is what forces the from-scratch cost, and that trade should be consciously signed off. |

---

## 11. What the adversarial review refuted

**Read this before quoting any number from this document.**

| Claim | Status |
|---|---|
| ripgrep throughput "2,239 MB/s" / "3.7 GB/s" / "4.9 GB/s" | **[REFUTED]** The cited article publishes **timings only, no throughput figures**. Two topics derived contradictory values from the identical timing (a 65% disagreement). Only the raw timings are citable (0.268s vs 0.516s on ~1GB; 0.366s vs 4.084s case-insensitive Unicode). |
| "ripgrep searches a 13.5GB file in 1.664s" | **[REFUTED]** **Does not appear in the source at all.** Only ~1GB and ~1.6GB corpora are discussed. It implies 8.1 GB/s sustained — faster than NVMe can read. |
| "Target ≤6s for a warm 10GB regex search; treat slower as a defect" | **[REFUTED]** Derived from the fabricated figures. No evidential basis. |
| "Zed ships subpixel AA through Rust/DirectWrite/D3D11" | **[REFUTED]** The Zed blog says nothing about subpixel, ClearType or `text_rendering_mode`. Sourced from an unofficial community tips site. |
| "BareTail: 30 files at ~2MB RSS" as a target | **[REFUTED as a target]** A single 2019 forum comment, not a controlled benchmark, and implausible — a bare Win32 window exceeds that before opening a file. Meanwhile D3D11+DXGI+DirectWrite costs 30–60MB before reading a byte. **This comparison is unwinnable and must not be published.** Realistic and still competitive: *"under 120MB RSS with 30 files open totalling 200GB, flat as file size grows"* — **the flatness is the defensible claim, not the absolute number.** |
| "glogg: 700MB RSS on a 20GB file" → "35MB index per GB" | **[REFUTED as an anchor]** glogg-era, second-hand from a blog comment, for a different program. Compute from the now-verified scheme instead. |
| "Parallel newline indexing is context-free" | **[REFUTED for UTF-16 only]** False for UTF-16: odd-offset boundaries misalign code units. **The DBCS half of this verdict was itself withdrawn in session 8** — `0x0A` is never a trail byte in any byte-oriented encoding, measured exhaustively; chunk boundaries need code-unit alignment and nothing else. See §5.8 and `SPEC.md` §5.3. |
| "mmap is the default for few-huge-files" | **[REFUTED]** Contradicted by two other topics; sourced from a Linux benchmark never validated on Windows. §5.1 resolves against mmap. |
| "AccessKit collapses a11y to a single tree-provider" | **[REFUTED]** It gives platform adapters, not a virtualised tree. A 10M-row grid needs hand-written `ITextProvider`. |
| Qt's LGPL static-linking blocker | **[REFUTED for this project]** The blocker assumed a closed-source product. The project is open source, so static LGPL linking is fine. Qt still loses on size and modern appearance. |
| WinUI 3 "requires a runtime install" / "can't be single-file" | **[REFUTED]** Self-contained deployment and `PublishSingleFile` supported since Windows App SDK 1.5. It still loses — on size, startup and Rust interop. |
| "No incumbent has a merged-by-timestamp view" | **[REFUTED]** Tailviewer does exactly this on Windows and keeps it live. The accurate, narrower claim: no *fast native single-exe Windows* viewer does. |
| "klogg proves a solo maintainer can sustain a tri-platform Qt log viewer" | **[REFUTED]** Last commit **2024-11-26**; last stable release 2022-06-13. It is the **refutation**, not the existence proof. |
| memchr "84–175 GB/s" | **[U]** From a search-result summary, not an authoritative benchmark. 175 GB/s exceeds consumer memory bandwidth — it can only be L1/L2-resident. The conclusion (indexing is I/O-bound) holds without it. |
| Azure Artifact Signing "~$10/month" | **[U]** The cited pricing page shows `$-`. |
| LogExpert issue #634 as "an unresolved regression" | **[Reclassified]** **Closed**, and maintainer-authored. Better used as *design intelligence*: it documents the exact failure mode (async `BeginInvoke` flooding the message queue defeating scroll-to-bottom) that §5.7's coalescing rule prevents. |
| "Sysinternals DebugView v5.0 shipped a dark theme" | **[U]** Unverified, no primary source. The `DwmSetWindowAttribute` argument stands on its own docs. |
| Effort estimates and multipliers | **[REFUTED]** The named components of the recommended architecture consume the entire claimed Windows-only baseline before the indexer, regex engine or columniser exist. **Rebuild bottom-up from components in `PLAN.md`.** |

---

## 12. Gating experiments

These must run **before** the spec is signed. Each has a defined pass/fail.

1. **SMB stale size on an open handle.** Writer on host A appending to a share; reader on host B polling
   `GetFileSizeEx` on its own open handle. Under SMB2/3 handle leases the client may serve
   `FileStandardInformation` from cache, meaning **the one mechanism nominated as ground truth is not
   ground truth on UNC** — where a large share of real Windows tailing happens. Measure observed
   latency; determine whether periodic handle reopen is needed.
2. **mmap vs overlapped `ReadFile` on Windows/NVMe**, 10GB file, cold and warm. The 25% mmap advantage
   is a Linux figure never validated on Windows. *(Expected to confirm §5.1; run it anyway.)*
3. **Toolkit binary sizes, measured.** Build three hello-worlds (windows-rs+D2D, eframe+glow,
   eframe+wgpu) with `opt-level="z"`, `lto="fat"`, `panic="abort"`, `strip=true`, `+crt-static`.
   Record real `.exe` bytes and cold-start-to-first-pixel. Every size figure in this document is a
   compressed release asset from another project.
4. **ClearType through the chosen pipeline.** Render light-on-dark and dark-on-light monospace at
   96/120/144 DPI; pixel-diff against `D2D DrawTextLayout`. *(Lower priority now the owner has
   deprioritised it — but it determines whether the opaque-target claim in §3.2 holds.)*
5. **Re-measure the incumbents.** BareTail 3.50a and LogExpert 1.41.0 on identical hardware, defined
   30-file corpus, recording private working set **and** commit size plus wall-clock open/close. Every
   competitive memory claim currently rests on a 2019 forum comment.
6. **Hoo WinTail hands-on.** The owner's own installed copy is the only reliable source for its exact
   behaviour — especially its encoding detection, which is undocumented, and which of its 25 features
   are actually used in a week.

---

## 13. Sources

Approximately 200 distinct primary sources were fetched across the five workflows — Microsoft Learn
(Win32 file, DWM, Direct2D/DirectWrite, SmartScreen, winget), GitHub APIs (release assets, issue
states, raw source), vendor sites, the GNU coreutils and FreeBSD `tail` sources, the OpenTelemetry
specification, Qt/egui/Slint/iced/wgpu issue trackers, and the Zed, Sublime HQ and ripgrep write-ups.
Full source lists are preserved per topic in the workflow transcripts under
`.claude/projects/…/subagents/workflows/`.
