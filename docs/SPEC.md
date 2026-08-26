# Tailhawk — Specification

**Version:** 0.1 (draft for adversarial review)
**Date:** 2026-07-28
**Companion documents:** [`RESEARCH.md`](RESEARCH.md) · [`UI-DESIGN.md`](UI-DESIGN.md) · [`PLAN.md`](PLAN.md)

---

## 1. Product definition

**Tailhawk is a Windows desktop application for reading, following and interrogating log files —
including files far too large for a text editor — with structured-log awareness, delivered as a single
executable that needs no installation and no runtime.**

Command: `tailhawk`.

### 1.1 Positioning

| | |
|---|---|
| **What it is** | A log *file* viewer and tailer. Files, scale, persistence, and zero-install Windows deployment. |
| **What it is not** | An observability platform, a metrics or tracing tool, a log shipper, or a text editor. |
| **Benchmark competitor** | **klogg** — handles 10+GB files and >2³¹ lines, but has no columnisation, no structured-log awareness, no cross-file search, no merged view, ships as a 19.5MB multi-DLL folder, and has been unmaintained since 2024-11-26. |
| **The tool to beat on feel** | **Hoo WinTail** — the incumbent the owner has used for years. Every feature they actually use (§1.3 of RESEARCH.md) is v1 scope. |
| **The tool to not become** | **LogExpert** — mandatory .NET runtime, full file reload past 25MB, compiled-DLL plugins for custom columns. |

**Explicitly not a differentiator, and must not appear in positioning copy:** "written in Rust",
"single binary", "OTLP-native". OpenObserve already owns all three. Tailhawk differentiates on
**desktop GUI + file tailing at scale**, which nothing in the observability field touches.

**On OpenTelemetry:** the spec states plainly that the **.NET Aspire Dashboard** is the correct tool
for live containerised OTel debugging. Tailhawk does not compete with it and no copy will imply
otherwise.

### 1.2 The five properties no existing tool combines

1. A genuinely self-contained single `.exe` with zero runtime install
2. Instant open and bounded memory on multi-GB files
3. Real charset auto-detection including BOM-less UTF-8 and UTF-16
4. Columnised structured-log parsing driven by the user's *actual* logging config
5. Live merged-by-timestamp multi-file viewing

### 1.3 Non-goals (v1 and beyond, unless explicitly revisited)

- Printing. Export covers the underlying need.
- Editing log files. Tailhawk is **read-only, always**, and this is a tested guarantee.
- ~~Querying remote backends (Loki, Tempo, Azure Monitor).~~ **Superseded for Loki. See `LOKI.md`.**

  This line was written 2026-07-28 and reversed by the owner on 2026-07-29: **Tailhawk is a Loki
  client, staged client-lite first** (`LOKI.md` §8; commits `fbd8cfb`, `1969929`). It sat here
  uncorrected for a month because `LOKI.md` was never folded back into this document, and it has
  since been quoted *to the owner* as though it were the current position. It is not.

  **Sharpened again on 2026-08-27, by the owner, after asking repeatedly: Tailhawk must be able to
  *tail* Loki, and that is the priority — ahead of §8.3's merged view.** `LOKI.md` §8 had recorded
  a merge-first ordering with the tail last of all, and a session reading it faithfully re-derives
  that ordering and offers it back; that has now happened more than once. Following is not a
  late-stage refinement here. A remote source you cannot watch live is not the thing that was
  asked for.

  **What still holds.** §13.2's zero-network guarantee applies to everything Tailhawk does on its
  own account — no telemetry, no update ping, no font or CDN fetch. A Loki source is an outbound
  connection **the user has explicitly configured and named**, which is a different thing, and
  §13.2's wording has to be amended to say so rather than quietly contradicted.

  **What is still true and useful.** §4.2's stdin pump means
  `logcli query --output=jsonl --tail '{app="x"}' | tailhawk -` works today at roughly zero
  incremental cost — `LOKI.md` §8 identifies it as the cheap 80% path and is explicit that it is
  *not* the decision: it delivers no label browser, no server-side pushdown, no timeline histogram,
  no `index/stats` pre-flight and no in-app time picker.

  **Tempo and Azure Monitor remain non-goals.** The reversal is Loki's alone.
- Metrics and traces as first-class data. Trace *IDs* are used for correlation only.
- ETW / WPP real-time sessions.
- `.evtx` binary parsing. Detect the file type and fail informatively.
- An in-app self-updater (§13.3).
- AI-assisted triage. Deliberately excluded: it is incompatible with the zero-network guarantee (§13.2).

---

## 2. Platform envelope, licence, distribution

### 2.1 Supported platforms

| | Decision |
|---|---|
| **Minimum OS** | **Windows 10 1809** (build 17763) |
| **Architectures** | **x64 shipped and supported. ARM64 built and published as best-effort.** No x86. |
| **Windows 11 features** | Applied opportunistically via runtime build-number checks — never assumed |

Rationale for 1809: `GetFileInformationByHandleEx(FileIdInfo)` needs Win8+; per-monitor-V2 DPI needs
1703+; 1809 is the oldest still-serviced LTSC baseline. `DWMWA_USE_IMMERSIVE_DARK_MODE` (22000+) and
`DWMWA_SYSTEMBACKDROP_TYPE` (22621+) are probed at runtime and silently skipped below those builds.

**ARM64 is deliberately *not* claimed as first-class**, and an earlier draft that did so was
incoherent: it committed to a NEON dispatch path, a separate signed artefact and *separately published
performance figures*, while §11.1 defines the single reference machine as x64 and the plan provisions
one x64 perf box. Under §11.1's own rule — *a target is not valid until it has a measurement behind
it* — those ARM64 figures would be unpublishable, and the NEON path would ship having never been
executed.

The honest position: **ARM64 binaries are built and published, cross-compiled and unmeasured**, the
newline scanner uses a portable fallback rather than a hand-written NEON path until a device exists,
and **no ARM64 performance figure is published**. Promoting ARM64 to first-class requires provisioning
a test device, which is a costed prerequisite, not an aspiration. There is currently no demand evidence
for ARM64 specifically.

### 2.2 Ownership and licence

**Tailhawk is a personal project.** It is published under the owner's **personal GitHub account** and
is unconnected to any employer, employer infrastructure or employer identity. Practical consequences
that are binding on this spec:

- No employer name, internal namespace, service name or domain appears anywhere in the repository,
  the sample data, the fixtures, the documentation or the screenshots.
- The code-signing identity (§17) is a **personal** identity, not an organizational one.
- Package IDs, the GitHub org, the domain and any funding link are personal.
- **Worth confirming independently:** many employment contracts assign IP for work created on company
  time or equipment. If any of this project is built on an employer machine or during work hours, that
  is worth checking before the first public commit — it is far cheaper to establish now than after a
  repository is public and has contributors.

**Licence: MIT OR Apache-2.0** (dual, the Rust ecosystem norm).

**Clean-room rule — mandatory.** klogg, TailBlazer, SnakeTail and Tail for Win32 are GPL. Any
algorithm adopted from them must be specified from **published documentation and header comments
only**, never by reading implementation source. A `CLEANROOM.md` records who read what and when. This
applies specifically to the compressed line index (§6.3), whose design is documented in klogg's
public header comment and documentation.

`cargo-deny` runs in CI with an explicit licence allow-list. An SBOM ships with every release.

### 2.3 Distribution

- **Primary:** GitHub Releases — a bare `tailhawk.exe` per architecture, plus a `.zip`.
- **scoop** first (bucket JSON manifest, zero friction for a portable exe, no signing requirement).
- **winget** second (`InstallerType: portable`, supported since Package Manager 1.3).
- **chocolatey** optional.
- **Code signing is required** and its identity is a **blocking open decision** — see §17.
- **Hard CI gate: the release `.exe` must be ≤ 15 MB.** Build fails above it. This is a budget, not a
  prediction; the honest expected range is 8–14 MB once `regex`, `encoding_rs`, format definitions,
  shader bytecode, static CRT and windows-rs bindings are counted.
- **No comparison against BareTail's 220 KB** appears in any marketing copy. That comparison is
  unwinnable and dishonest for an app with a GPU-composited UI.

---

## 3. Architecture

### 3.1 The seam

```
┌──────────────────────────────────────────────────────────┐
│  SHELL  (per-platform, Windows-only in v1)               │
│  window · chrome · menus · dialogs · input · IME ·        │
│  clipboard · drag-drop · DPI events · accessibility       │
└───────────────────────┬──────────────────────────────────┘
                        │ owns an HWND, hands the core a drawable
┌───────────────────────┴──────────────────────────────────┐
│  CORE  (portable Rust — the whole grid lives here)       │
│  ┌────────────────────────────────────────────────────┐  │
│  │ VIEW: virtualised grid · layout · selection ·      │  │
│  │       hit-test · scroll model · highlight paint    │  │
│  ├────────────────────────────────────────────────────┤  │
│  │ ENGINE: sources · index · decode · parse ·         │  │
│  │         highlight rules · filter · search · follow │  │
│  └────────────────────────────────────────────────────┘  │
│         │ LEAF BACKENDS (thin, per-platform)             │
│         ├─ glyph rasteriser  → DirectWrite               │
│         └─ presentation      → D3D11                     │
└──────────────────────────────────────────────────────────┘
```

**This follows Ghostty's verified structure, not the popular description of it.** The renderer and
font rasterisation live in the core; the shell does window, input and IME only. See RESEARCH.md §4.2.

**No public C ABI in v1.** The moment grid-model data crosses a C boundary, the header explodes
(Ghostty's grew from ~1.2k lines to 5k+ when it exposed the model). Adding a second shell later means
adding a leaf backend pair and a shell, not publishing an ABI.

### 3.2 Rendering

- **D3D11**, not D3D12/Vulkan/wgpu. Guaranteed Windows 7+, WARP software fallback, smaller memory
  footprint, degrades gracefully in VMs and RDP.
- **Fallback chain:** D3D11 hardware → D3D11 WARP → Direct2D/DXGI. `DXGI_ERROR_DEVICE_REMOVED` is
  handled by recreating the device. **Never panic on device-removed** — that is the exact bug that
  crashes wgpu apps on driver auto-update.
- **Glyph atlas, not per-line text layout.** DirectWrite rasterises each unique glyph once into a
  persistent atlas keyed by `(glyph id, style, dpi scale)`. The viewport renders as one instanced draw
  with foreground colour, background colour and style as per-instance attributes. Per-token colouring
  is therefore free. Never build an `IDWriteTextLayout` per visible line per frame.

  **How one draw serves both monochrome and colour glyphs** — measured in `experiments/g4-glyph-atlas`,
  which refuted the assumption that it could not. The pixel shader emits **two** outputs and
  dual-source blending consumes the second: `SV_Target0` carries premultiplied colour, `SV_Target1`
  carries per-channel coverage, and with `SrcBlend = ONE`, `DestBlend = INV_SRC1_COLOR` the hardware
  computes `dest = src + dest * (1 - coverage)`. That one equation is simultaneously the correct
  per-channel (ClearType) blend for a monochrome glyph whose mono path premultiplies in the shader,
  and the correct premultiplied composite for a colour bitmap. A per-instance `mode` selects between
  them. **Single-source straight alpha cannot do this** — the blend equation has one alpha per pixel,
  so it cannot consume three independent coverages; subpixel AA without dual-source blending costs a
  second pass over the same geometry. *(Trap: the alpha slots take `INV_SRC1_ALPHA`; a `*_COLOR`
  factor in an alpha slot fails `CreateBlendState` with a bare `E_INVALIDARG`.)*
- **The atlas is a fixed-slot LRU with uniform slots and an O(1) victim list.** Not a shelf packer and
  not variable-width spans. Every glyph occupies exactly one slot sized to the widest glyph accepted,
  so eviction never repacks, never searches for a free run, and never fragments. The victim comes off
  the head of an intrusive doubly-linked list. **Measured justification:** scanning all slots for the
  oldest costs 4–8 ms per frame under atlas thrashing against 0.17–0.37 ms for the list, and an
  earlier variable-width-span variant cost **106 ms per frame**. Slots touched in the current frame are
  never evicted, or the frame corrupts itself. The cost is atlas density — roughly 46% of the sheet
  goes to padding around narrow Latin glyphs — which is a good trade for a monospace grid.
- **Glyph rasterisation is off the paint path — a first-run requirement, not a permanent tax.**
  DirectWrite rasterisation is backed by a **cross-process system font cache**. The first process on a
  machine to rasterise a given `(glyph, size, rendering mode)` pays **86–108 µs**; every process
  afterwards pays **~3 µs** for it, and that survives process exit. So a viewport of ~1,500
  previously-unseen CJK glyphs costs **162 ms — 8 to 10 frames — the first time that machine ever
  renders them**, and **4.4 ms** every time after. A glyph that is not yet resident must therefore draw
  a placeholder and be filled in over subsequent frames; a frame must never block on rasterisation.
  **This is a v1 requirement**, because first run is what a user judges the product on. It is sized for
  the cold cost, and **no steady-state budget may be derived from the cold figure** — §11.3 targets are
  steady-state.
  **Cache capacity is 8,000–16,000 distinct glyphs**, so only a large CJK working set can sustain
  misses; a Latin log viewport is a few hundred distinct glyphs and never leaves the cache.
  **Rasterise 4–64 glyphs per `CreateGlyphRunAnalysis`** — measured bit-identical to per-glyph output
  at any inter-glyph gap including zero, and ~1.8x faster on the cold path. The win saturates at 4;
  batching a whole viewport is *slower* than batching four.
  Measured in `experiments/g4b-batched-raster`. *(Trap: the cache is cross-process, so re-running a
  measurement warms it — only the first process to touch a glyph population measures rasterisation,
  and a reboot empties the cache rather than providing a neutral cold start.)*
- **Cache the absence of ink.** A glyph with no raster — a space, or a codepoint absent from the face —
  is cached as a blank occupying no slot. Without this every space is re-rasterised every frame.
- **Shaders are compiled offline** with fxc/dxc and the bytecode embedded. CI asserts no
  `d3dcompiler_47.dll` import.
- **First paint is two-stage, and this is a v1 requirement.** Fill the client area with the solid
  background colour as soon as the window exists — a `FillRect`, or equivalently a class background
  brush — then create the D3D11 device on a **worker thread** and swap to the real renderer when it is
  ready. Both stages draw the same background, so the transition is invisible.
  **Measured in `experiments/g3-d3d11`:** this roughly halves time-to-first-pixel (48–54% of the naive
  order, 12 of 12 paired trials), and off-thread device creation is worth a further ~8.5 ms on an idle
  machine and *far* more under GPU-context pressure, where serial device creation degraded to over a
  second while the off-thread path held at ~135 ms.
  Two things that experiment also settled, so they are not worth re-litigating: a class background
  brush is **equivalent** to a `FillRect` in the paint handler, not better (4 of 12 pairs), so pick
  whichever is simpler; and ~~the residual floor of ~50–60 ms is window presentation~~ — **withdrawn.
  That floor does not exist; it was background load.** On a quiet machine the two-stage paint reaches
  first pixel in **13.1 ms p50 / 14.5 ms p90**, with `CreateWindowExW` at 3.2–3.9 ms, so `ShowWindow`
  plus paint dispatch costs about **10 ms** beyond window creation, not 50–60. The two-stage paint is
  therefore **sufficient** to meet the 40 ms first-pixel criterion, not merely helpful — it passes by
  ~3x with it and fails by ~1.7x without.
- **Text antialiasing: not a design driver, and no user setting.** The log grid renders to an
  **opaque** target, which per `D2D1_TEXT_ANTIALIAS_MODE` gets ClearType by default. We take that if
  it comes free and accept greyscale if it does not. **No `text_rendering` setting ships**, and no
  custom ClearType blend shader is in scope. The owner explicitly de-prioritised text quality in favour
  of shipping (RESEARCH §1.3), and an earlier draft reintroduced it as a setting and as a Phase-0
  experiment anyway. Mica/Acrylic applies to title bar, tab strip and side panels **only**.
- **RDP is a first-class render path, not an optimisation.** On `GetSystemMetrics(SM_REMOTESESSION)`
  Tailhawk switches to a Direct2D-on-DC path, caps repaint at ~15 Hz, and uses scroll-region blitting
  so only newly exposed rows are drawn. Rationale: over RDP a DXGI `Present` pushes a full composed
  framebuffer through the remote protocol; at 5,000 lines/s over a 5 Mbps link a 60 Hz swapchain
  saturates the channel.

  **WARP is *not* a trigger for this path.** An earlier draft listed it as one, which is unresolvable:
  WARP is simultaneously step 2 of the fallback chain and, on that reading, the signal to abandon the
  chain. WARP is a *rendering backend*; RDP is a *transport condition*. They are detected
  independently and a local WARP session renders normally.
- **Per-monitor-V2 DPI** declared in the manifest. All layout metrics recomputed on `WM_DPICHANGED`;
  the glyph atlas is rebuilt per scale factor. **Column advances are computed in integer device pixels
  at the current scale and re-derived on any scale change** — fractional per-glyph rounding accumulates
  drift and visibly misaligns columns across a wide window.

### 3.3 The cell model — grapheme clusters, not characters

**A cell column is a *logical* position — the Nth grapheme cluster in memory order — and never a
visual one.** Decided by the owner, session 15, when RTL placement forced the question. The two
coincide for Latin text and diverge for Arabic and Hebrew, and everything that speaks in columns
(`Position`, selection, copy, and later search hits and bookmarks) means the logical one.

Three consequences follow and are normative:

- **Copy stays exact.** A logical column range is one contiguous byte span, which is what §5.6
  requires; a visually contiguous range maps to a *discontiguous* set of byte ranges and could not be
  copied without reordering or splitting.
- **Painting and hit-testing convert.** Both use a per-row bidi map derived from resolved levels
  (UAX #9 rule L2). Neither the cell model nor the selection model knows about it. Because bidi
  cannot be resolved from a horizontal slice of a line, the renderer must resolve levels for the
  **whole line** — shaping only the visible slice is valid only while placement is logical.
- **A selection may be visually discontiguous.** One contiguous logical range crossing a direction
  boundary draws as several highlight rectangles. That is correct, and the selection renderer takes
  a list of rectangles per row rather than one span.

**Rectangular (block) selection is the exception**: "the same column band on every row" is inherently
visual, so it is expressed in visual columns and converted to per-row byte ranges when copied. It
must not reuse the logical column type.

The naive "one character = one fixed-width cell" assumption breaks on real log content and is
**rejected**. The cell model is defined as:

- The unit is the **grapheme cluster**, not the code point.
- **East Asian Wide** and **Fullwidth** clusters occupy **2 cells**.
- **Non-spacing combining marks** (`Mn`, `Me`) occupy 0 additional cells. **Spacing marks** (`Mc`)
  do not: a Devanagari or Bengali matra is part of the same grapheme cluster but carries its own
  advance, so `कि` is one cluster of **2 cells**, as is Thai SARA AM. *An earlier draft of this
  bullet said "combining marks occupy 0 additional cells" without the distinction, and a test of
  ours asserted the wrong answer for it — for a script this section's own acceptance test names.
  Corrected in `cell.rs` first; the correction reached here in session 15.*
- **ZWJ emoji sequences** are one cluster; width is determined by emoji presentation.
- **Colour emoji** render through `TranslateColorGlyphRun` into a separate colour atlas. A monochrome
  alpha atlas cannot represent them. Both atlases are sampled in the **same** instanced draw under one
  blend state — see §3.2. **Colour glyphs carry greyscale coverage, not subpixel**: a coloured layer's
  three channels cannot survive an alpha composite against a different colour, so layers are averaged
  before compositing. Harmless for pictorial glyphs, but it means mono and colour differ in AA quality
  by construction rather than by oversight.
- **Font fallback** follows a documented chain with a nominated monospace CJK fallback. When a fallback
  font's advance width disagrees with the primary, the cell grid wins and the glyph is centred within
  its cell.
- **Horizontal extent.** Each index block stores `max_byte_len` and an `all_ascii` flag — both genuinely
  free during the newline scan. Where `all_ascii` holds, `max_byte_len` **is** the cell count and the
  extent is exact. Where it does not, `max_byte_len` is an **upper bound** on cells (no encoding
  produces more cells than bytes), so the scrollbar is conservative and is refined lazily as blocks are
  actually laid out. Extent is never derived from currently-visible lines — that causes the documented
  horizontal-thumb jitter.

  **The extent the scrollbar uses is capped by §10.3's render cap**, not by the file's longest line: a
  41 MB line is truncated at 32 KB inline, so nothing past 32,768 cells is ever drawn and scrolling
  there would address pixels the grid cannot reach. The cap is also what keeps the horizontal axis
  inside `f32`'s exactly-representable integer range (32,768 cells × a bounded cell width < 2²⁴ px),
  which is why §6.4's rules 1 and 2 are not needed on this axis. **Raising the cap to "unlimited" is
  therefore not a rendering-only change** — it reintroduces the `(index, remainder)` obligation
  horizontally.

  *An earlier draft claimed max cell count was "captured free during the index pass". It is not:
  grapheme segmentation with East Asian Width costs 10–50× a newline scan, and it requires the cell
  model, which does not exist when the index is built.*

**Acceptance test:** a fixture log containing CJK, Devanagari, RTL Arabic, ZWJ emoji families and
box-drawing characters renders with columns aligned, and dragging the window between a 100% and a 150%
monitor produces no column drift.

---

## 4. Source model

A **source** is an abstraction, not a file path. This is a v1 architectural decision even though not
every implementation ships in v1.

| Source | Phase | Notes |
|---|---|---|
| **Local file** | v1 | The primary case |
| **UNC / network file** | v1 | First-class; different follow strategy (§5.4) |
| **Rolling set** (directory + pattern, one logical log) | **v1** | **§5.5b.** The default shape for Serilog and NLog |
| **Glob / watched directory** | v1 | Adopts new matching files as *separate* sources — distinct from a rolling set |
| **stdin / pipe** | v1 | Spilled to a temp file (§4.2) |
| **Process-spawn** | v2 | `docker logs -f`, `kubectl logs -f`, `az containerapp logs show --follow` |
| **Compressed archive member** | v2 | `.gz`, `.zip` (§4.3) |
| **OTLP receiver** | Deferred, gated on demand | §12.4 |
| **Windows Event Log (live)** | v3 | Via `EvtSubscribe`; no admin rights needed |

### 4.1 Process-spawn sources (v2) — security boundary

A process-spawn source executes a command line. This is the product's principal RCE surface.

- Commands come **only** from direct user entry or a built-in template (`docker`, `kubectl`, `az`).
- A command **may never** originate from an imported session, workspace or format file (§13.1).
- The resolved executable path and full argument vector are displayed before first launch and are
  visible in the source's properties thereafter.
- No shell interpolation. Arguments are passed as a vector, never as a command string.

### 4.2 stdin

- Detected with `GetFileType(GetStdHandle(STD_INPUT_HANDLE))`: `FILE_TYPE_PIPE` or `FILE_TYPE_DISK`
  → read; `FILE_TYPE_CHAR` (interactive console) or invalid handle → do not block.
- **Spilled to a temp file**, not held in memory. This gives scrollback (a pipe is unseekable and
  consume-once), reuses the same index path as a real file, and preserves the multi-GB promise.
- Read on a background thread with blocking `ReadFile`. `ERROR_BROKEN_PIPE` or a 0-byte read means
  **stream complete** — the window stays open and the trailing partial line is flushed. It is not an
  app exit.
- Encoding is detected on the piped bytes exactly as for a file — PowerShell's native-command pipeline
  has historically emitted UTF-16 and OEM codepages.
- When stdin is a pipe, single-instance forwarding is **disabled** (implies `--new-window`). A pipe
  handle cannot usefully be handed to another process.
- **Temp-file location and lifetime are a privacy concern** — see §13.2.

### 4.3 Compressed archives (v2)

`.gz` and `.zip` members are routine ("yesterday's log is now a `.gz`" — NLog `ArchiveEvery`, log4net
rolling appenders). v2 decision: **decompress to a temp file and index that.** It is simple, correct,
and reuses the whole file path. Streaming with zran-style access points is a v3 optimisation only if
open latency proves unacceptable. Archive members participate in rotation-aware source grouping so a
log and its archives appear as one logical stream.

---

## 5. File engine

### 5.1 Opening — the writer-safety guarantee

Every followed file is opened exactly once, with exactly:

```
CreateFileW(path,
            GENERIC_READ,                                            // never GENERIC_WRITE
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,  // all three, mandatory
            NULL, OPEN_EXISTING,
            FILE_FLAG_SEQUENTIAL_SCAN | FILE_FLAG_OVERLAPPED,
            NULL)
```

> **Product guarantee: Tailhawk never prevents your application from writing, rotating or deleting its
> own logs.**

This is **tested**, not asserted: an automated test runs rename-and-recreate and copy-and-truncate
rotation loops while Tailhawk is attached and asserts no `ERROR_SHARING_VIOLATION (32)` or
`ERROR_USER_MAPPED_FILE (1224)` on the writer side.

Consequences that follow from the guarantee:
- **No memory mapping of any followed file, ever** (§5.2).
- Never `LockFile`/`LockFileEx`. Never `FILE_FLAG_DELETE_ON_CLOSE`.

**When the writer locks us out:** `ERROR_SHARING_VIOLATION (32)` is a property of the *writer's*
handle and is unfixable from the reader side. Tailhawk detects it specifically and presents the
writer-side remedy by name:

| Writer | Remedy to show |
|---|---|
| Serilog file sink | `shared: true` |
| NLog `FileTarget` | `keepFileOpen="false"` or `concurrentWrites="true"` |
| log4net `FileAppender` | `<lockingModel type="log4net.Appender.FileAppender+MinimalLock"/>` |

No handle-stealing, no `NtDuplicateObject`. A Volume Shadow Copy read may be offered as an explicit
one-shot action in v3.

### 5.2 Reading — no mmap

**Memory mapping is excluded from the design entirely**, including for search.

Rationale, in priority order:
1. A `CreateFileMapping` section handle **blocks `DeleteFile`**. A log4net `RollingFileAppender` with
   `maxSizeRollBackups=5` would fail to delete its oldest backup, swallow the error, and **silently
   stop rolling** — filling the customer's disk, with Tailhawk as the cause. "Tear the section down
   when rotation is suspected" is unimplementable: the failure *is* the detection event.
2. Mapped views are documented as **not coherent** with `ReadFile`/`WriteFile` — and the live tail
   region is exactly where writers are appending.
3. Truncation under a view raises `EXCEPTION_IN_PAGE_ERROR`, which **Rust cannot catch portably**.
4. UNC execution turns a dropped SMB connection into `EXCEPTION_IN_PAGE_ERROR` inside the read loop.

**Read path:** overlapped buffered `ReadFile`, 4–8 outstanding requests of 1–4 MB on an IOCP. Random
access for scrollback uses `ReadFile` with an explicit `OVERLAPPED.Offset` (thread-safe on one handle,
no file-pointer contention).

`FILE_FLAG_NO_BUFFERING` is **not** used by default; it bypasses the cache, making re-opens slow. It
may be exposed for the initial cold index of a very large file where evicting the user's file cache is
undesirable.

### 5.3 Line index

> **Clean-room re-derivation, 2026-08-04.** This section was rewritten from first principles under
> `CLEANROOM.md` §2, which requires the index to be derived from §3 allow-listed sources only. The
> previous text shared its number and topic with the contaminated `RESEARCH.md` §5.3 and its
> provenance was unestablished. Neither section was read during this derivation. Sources relied on
> are recorded in `CLEANROOM.md` §5; the attestation is §6. **The design below differs materially
> from what it replaces** — see "What this replaces" at the end.

#### What the index owes

| | Requirement | Why |
|---|---|---|
| **R1** | line number → byte offset, in bounded time | §6.4's scroll position is a `u64` line number; the renderer must turn it into a read |
| **R2** | byte offset → line number | "go to offset", and re-anchoring the viewport after truncation (§5.5) |
| **R3** | append in O(1) amortised, without rebuilding | following a live writer is the normal state, not an edge case |
| **R4** | memory O(lines) with a **small** constant | §11.2 claims flatness as file size grows; the index is the structure most able to break it |
| **R5** | usable while incomplete | §11.3 requires the UI never block on indexing; a partial index must serve the viewport it has |
| **R6** | constructible in parallel | §11.3's throughput, and the whole file must be walked once regardless |

#### The structure

**Sparse anchors, with a forward scan between them.**

- Every `S`-th line's absolute byte offset is stored as a `u64` **anchor**. Nothing is stored for
  the lines between.
- To reach line `n`: take anchor `n / S`, read forward from it, and count `n % S` line terminators
  with a `memchr`-class scan.
- Anchors live in **fixed-size blocks of 4,096**, not one flat vector. This is an allocation
  decision, not a compression one: a followed file grows without bound, and appending to a flat
  vector of tens of millions of entries reallocates and copies the whole thing at exactly the moment
  the UI is trying to stay at 60 Hz.

`S` is a power of two, so `n / S` and `n % S` are a shift and a mask.

**Anchors are grouped into segments, each carrying its own base line number.** A parallel worker
cannot know the global line number its chunk starts at — that is settled only once every earlier
chunk has finished counting — so it anchors from its own first line and the merge records the base
(see "Construction" below). Reaching line `n` therefore finds the segment covering `n` first, by
binary search over one entry per chunk — about 1,300 entries at 10 GB, so a handful of steps — and
then takes anchor `(n − base) / S` within it. **A serially built or followed index is a single
segment**, where this degenerates to exactly the `n / S` above; the directory is what lets the
parallel and serial paths share one structure instead of two.

#### Deriving S

Measured mean line length on the two real corpora (`CLEANROOM.md` §5 — our own measurements, which
§3 prefers): **116.8 and 84.2 bytes**. The working figure below is 100 B/line, which puts a 10 GB
file at 10^8 lines.

| Design | bytes/line | index at 10 GB | forward scan per lookup |
|---|---|---|---|
| absolute `u64` per line | 8.000 | **800 MB** | none |
| `u32` relative to a block base | 4.060 | 406 MB | none |
| `u16` delta + escapes | 2.100 | 210 MB | block-local sum |
| `u8` delta + escapes | 1.100 | 110 MB | block-local sum |
| **sparse anchor, S = 16** | 0.500 | 50.0 MB | 1.6 KB |
| **sparse anchor, S = 64** | **0.125** | **12.5 MB** | **6.3 KB** |
| sparse anchor, S = 256 | 0.031 | 3.1 MB | 25.0 KB |
| sparse anchor, S = 1024 | 0.008 | 0.8 MB | 100.0 KB |

**Memory is not the binding constraint, and that is the finding.** Every sparse variant is already
negligible against §11.2's 120 MB whole-process claim — the difference between 12.5 MB and 0.8 MB
does not matter, so `S` should be chosen on *latency*, not size.

The binding constraint is the **cold** forward scan. Between anchors the bytes must be read (§5.2
forbids mmap), and on a random seek — a scrollbar drag — that read is not in page cache. At
`S = 1024` every dragged frame pays a 100 KB read; at `S = 64` it pays 6.3 KB, which is under two
pages and well inside a typical readahead. Warm, all of these are sub-microsecond `memchr` and the
choice is irrelevant.

**`S = 64`.** It costs 12.5 MB on a 10 GB file — 0.125% of it — and keeps the worst random-access
read small enough to be one I/O.

#### Delta encoding is not needed, and is rejected

A delta or block-relative scheme optimises the per-line cost of storing *every* line. Once lines
between anchors are not stored at all, there is nothing left for it to compress: the best delta
scheme in the table is **110 MB against sparse anchoring's 12.5 MB**, nearly 9x worse, while adding
a per-block escape mechanism, a variable-width decode and a block-local summation on every lookup.

**Sparse anchoring is both an order of magnitude smaller and materially simpler.** Long lines
(§10.3) also break delta schemes precisely where they are least welcome — a 1 MB line is an escape
in every one of them, and is unremarkable to an anchor.

#### Alignment, and the only encoding constraint

Chunk boundaries — for the parallel scan and for anchor placement — must be **code-unit aligned**
(`Charset::code_unit()`). Splitting a UTF-16 file at an odd offset swaps every subsequent byte pair,
so the scan hunts `0A 00` in a stream where it is now written `00 0A`.

**That is the only constraint.** There is no encoding-specific exception and in particular no DBCS
one: `0x0A` is always a line terminator, never a trail byte, in every byte-oriented encoding. This
is measured, not assumed — `a_0a_byte_is_never_consumed_as_a_trail_byte_by_any_decoder` drives all
65,536 two-byte prefixes into every supported decoder. An earlier draft asserted the opposite and
disabled the parallel path for codepages 932/936/950/949; that was withdrawn in session 8.

`ISO-2022-JP` constrains **viewport decode only, never the scan**. It is escape-driven, so a line
start is a character boundary but not a decoder boundary, and decoding must begin from a known
shift state rather than from an arbitrary anchor (`Charset::is_random_access_decodable()`).

#### Construction

**Parallel.** The file is divided into code-unit-aligned chunks, one worker each; each counts
terminators and emits anchors at its own local stride. A prefix sum over the per-chunk line counts
converts local anchor numbering to global. No worker needs any other worker's result to scan.

**Progressive (R5).** Anchors are published as they are produced. Until the scan completes, the
total line count is a lower bound and the scrollbar is proportional to bytes indexed rather than
lines — §11.3 requires the UI never block on this.

**Append (R3).** Following adds terminators to the end. Every `S`-th one appends an anchor; the rest
cost nothing. A block fills and the next is allocated. Nothing existing is rewritten, which is what
makes appending free rather than amortised.

**Truncation and rotation (§5.5)** discard anchors from the truncation point on. Because anchors are
absolute offsets and blocks are independent, this is a truncate of the anchor array, not a rebuild.

#### What this replaces

The superseded design was block-sparse with delta encoding, at 128 lines per block — a shape whose
provenance `CLEANROOM.md` §2 records as unreconstructable. The re-derivation **does not reproduce
it**: it keeps blocking (for allocation, not compression), rejects delta encoding on a ~9x memory
argument, and lands on a stride of 64 chosen against cold-read latency rather than 128 chosen
against nothing recorded. `SPEC.md` §11.2's memory line is updated to match.

### 5.4 Following

**Polling is the correctness mechanism. Change notification is an accelerator that is allowed to fail
silently.** This is forced by Windows, not by any cross-platform ambition:
`FILE_NOTIFY_CHANGE_SIZE`/`LAST_WRITE` only fire once the writer's cache is flushed to disk, so a
buffered Serilog/NLog/log4net writer produces **no notification at all** until flush or close.

- **Poll `GetFileSizeEx` (or `GetFileInformationByHandleEx(FileStandardInfo)`) on our own open
  handle.** Never `GetFileAttributesEx`/`FindFirstFile` on the path — NTFS replicates size into the
  directory entry only on last-handle-close, so a path-based stat on an actively-written log returns a
  frozen size forever.
- **Cadence:** 100 ms local, 250–500 ms UNC, adaptive back-off to 1 s when idle >30 s, instant
  snap-back on first change. Exposed as `-s`.
- **`ReadDirectoryChangesW` is cut from the v1 tail path.** It is retained only for
  `FILE_NOTIFY_CHANGE_FILE_NAME` on watched directories — noticing a *new* file matching a glob, where
  it does fire reliably and polling is expensive.
- **Per-tick work budget:** ≤ 4 MB of new bytes processed per tick, remainder carried forward. At
  10–50 MB/s append a 250 ms tick otherwise delivers 100k+ lines to index, parse and lay out in one
  frame.

**UNC mode** is detected up front (`GetDriveType() == DRIVE_REMOTE` or a UNC path) and surfaced in the
UI as "network mode — updates may lag". **The correctness of polling an open handle over SMB is an
unresolved gating question** (§17, experiment 1): under SMB2/3 handle leases the client may serve
`FileStandardInformation` from cache. If the experiment confirms staleness, UNC mode gains a periodic
handle close/reopen every N polls.

### 5.5 Rotation and truncation

File identity is `(VolumeSerialNumber, FILE_ID_128)` from `GetFileInformationByHandleEx(FileIdInfo)`.
The legacy 64-bit index is **not unique on ReFS** and is not used.

There are **three** rotation modes, not two. An earlier draft covered only the first two and would have
sat forever on a dead file under Serilog's and NLog's *default* configuration.

| Case | Detection | Behaviour |
|---|---|---|
| **Copy-truncate** | `size < last_read_offset` | Reset to offset 0; render a "file truncated" separator row; bookmarks orphaned (§10.4) |
| **Rename-and-recreate** (`app.log` → `app.log.1`, log4net default) | path's identity ≠ held handle's identity | **Drain the old handle to EOF first**, then switch to the new file at 0. This is where naive tools lose the last KB. |
| **Roll-to-new-name** (**Serilog and NLog defaults**) | A *new* file appears in the directory matching the set pattern, with a higher sort key than the current one | The written-to path **never changes identity and never shrinks** — nothing about the old file signals the roll. Detection is directory-side, not file-side. Drain the old file to EOF, then attach to the new one, **as a continuation of the same logical source**. |
| **Path missing** | open fails | Keep the old handle, retry on a timer, show "waiting for `<path>`" — `tail -F` semantics, never an error dialog |

### 5.5b Rolling sets — the unit of tailing is the set, not the file

> **A source may be a *rolling set*: a directory plus a pattern, presented as one continuous logical
> log with unbroken scrollback across its members, that automatically follows the roll.**

This is what makes the tool work against a default-configured .NET application, and no incumbent does
it — Hoo WinTail's folder monitoring opens new files as *separate documents*, which is not the same
thing.

**Set recognition.** Pointing at a single file offers to adopt the whole set, inferred from siblings:

| Writer | Default shape | Inferred pattern |
|---|---|---|
| Serilog `rollingInterval` | `log-20260728.txt`, `log-20260729.txt` | `log-<date:yyyyMMdd>.txt` |
| Serilog `rollOnFileSizeLimit` | `log_001.txt`, `log_002.txt` | `log_<seq:000>.txt` |
| Serilog both | `log-20260728_001.txt` | `log-<date>_<seq>.txt` |
| NLog `archiveFileName` `{#}` | `archive/log.2026-07-28.txt` | date or sequence in a sibling folder |
| log4net rolling | `app.log`, `app.log.1`, `app.log.2` | **descending** — `.1` is *newer* than `.2` |

**Ordering is the trap.** Date and ascending-sequence sets sort newest-*last*; log4net's rename-based
backups sort newest-*first* (`app.log` is current, `app.log.1` is the previous one). Getting this
backwards silently presents history in reverse. The set therefore carries an explicit
`order: ascending | descending | by-mtime` field, **inferred and then shown in the UI for
confirmation** rather than silently assumed. `by-mtime` is the fallback for unrecognised shapes.

**Behaviour:**
- **Continuous scrollback.** Scrolling up past the head of the current member continues into the
  previous one. Line numbers are per-member with the member named in the gutter; a separator row marks
  each boundary.
- **The index spans the set**, built newest-first so the live tail is usable immediately and history
  fills in behind.
- **A new member is detected by directory watch** — this is the one case where
  `FILE_NOTIFY_CHANGE_FILE_NAME` fires reliably (§5.4), so it is genuinely event-driven rather than
  polled.
- **Drain-then-switch**, exactly as for rename rotation: the old member is read to EOF before the new
  one is attached, so the last lines before a roll are never lost.
- **Archived members participate.** When a rolled member is later compressed to `.gz` or `.zip` by a
  retention job, it remains part of the set and is transparently decompressed (§4.3, v2).
- **Retention deletions are tolerated.** A member disappearing from the middle or tail of the set
  removes it from the scrollback with a marker; it is never an error.
- **Bounded history.** A set with 400 daily files is not fully indexed on open. The **newest N members
  (default 10, configurable) or M bytes** are indexed eagerly; older members are indexed on demand as
  the user scrolls into them, with a progress marker.

**Tab identity, dedup and rotation detection are keyed on file identity, never on the path string.**
Windows has per-directory case sensitivity since 1803 (WSL-created directories are case-sensitive and
cannot be made insensitive), so path-string comparison produces both false merges and false splits.

Follow mode is exposed per source as **"On rotation: follow the new file" (default, `-F`)** vs
**"stay with this file"** (`-f`, presented as *pin to this file even if it is rotated away*).

### 5.6 Encoding

Resolution order is strict, and **runs before indexing** because UTF-16 newline scanning looks for
`0A 00`/`00 0A`, not `0A`:

1. **BOM sniff, longest match first.** `FF FE 00 00` (UTF-32LE) **before** `FF FE` (UTF-16LE) — testing
   UTF-16LE first misdetects every UTF-32LE file. Then `EF BB BF`, `00 00 FE FF`, `FE FF`. The BOM is
   consumed and never rendered, but remains in the byte-offset index so offsets stay byte-exact.
2. **UTF-16/32 probe by NUL-position parity** over a 64 KiB head sample **and** a 64 KiB tail sample.
   chardetng does not detect UTF-16 or UTF-32 at all — the Encoding Standard excludes them — so this
   is hand-written. Required on Windows: PowerShell 5.1's `>` and `Out-File` default to UTF-16LE.
   **The test is NUL density *with consistent parity*, not raw NUL count**, and the hypothesis is
   sanity-checked against decoded plausibility — otherwise a UTF-8 log with an embedded binary blob is
   misdetected as UTF-16 wholesale, destroying the index.
3. **Strict UTF-8 validation** over the sample. Valid with at least one multi-byte sequence → UTF-8,
   high confidence. Pure ASCII → UTF-8 (superset, safest).
4. **chardetng**, falling back to `GetACP()`.

**Incremental decoding:** one long-lived `encoding_rs::Decoder` per followed source for the tail
stream, **never reset between appends** — it owns the partial-sequence carry across read boundaries.
Viewport reads use short-lived decoders started from a back-aligned safe boundary (UTF-8: back up ≤3
bytes to a non-continuation byte; UTF-16: align to an even offset, handle a lone surrogate).
**Only byte offsets are stored in the index.**

**Ambiguity:** if head and tail samples disagree, prefer the tail (it is what is live) and flag the
file in the UI. Decode with U+FFFD replacement, never error. A visible encoding indicator sits in the
status bar with a per-file and per-glob override; **re-run detection on rotation**, since the new file
may differ.

**Line terminators:** the set is `{CRLF, LF, CR}` with a one-byte pending-CR carry across chunk
boundaries. `\r` is part of the terminator and stripped from line content, never treated as content. A
whole-file "this is a CRLF file" decision is wrong — mixed files are routine.

**Binary content:** control bytes and NULs render as a visible replacement glyph, never silently
dropped. A "reveal invisibles" toggle (§13.4) renders them explicitly.

---

## 6. The record model

### 6.1 OTel-shaped, superset, lossless

The normalised record every parsed format maps into is **the OpenTelemetry log data model, extended**:

| Field | Source |
|---|---|
| `timestamp` | Parsed, timezone-aware instant |
| `observed_timestamp` | When Tailhawk read it |
| `severity_number` | 1–24, per the OTel banding |
| `severity_text` | The original string as it appeared |
| `body` | The message |
| `trace_id`, `span_id`, `trace_flags` | §9 |
| `resource` | Per-**source** constants (host, service) — belongs to the pane, not the row |
| `attributes` | Per-**record** varying values — these become columns |
| `event_name` | Where present |
| **`raw`** *(extension)* | The original bytes of the record, always retained |
| **`format_id`** *(extension)* | Which detector claimed it |
| **`parse_state`** *(extension)* | `parsed` \| `continuation` \| `foreign` \| `unparsed` |

The `raw` extension is what makes this lossless: Serilog message templates, W3C-only fields and
anything else without an OTel home survive intact and are always copyable and searchable.

### 6.2 Why OTel severity rather than a flat enum

- **Banding, not levels.** Six bands of four (TRACE 1–4, DEBUG 5–8, INFO 9–12, WARN 13–16, ERROR
  17–20, FATAL 21–24) let syslog NOTICE (10), log4net's extra levels and Zap's DPanic/Panic coexist
  with Serilog levels in **one sortable cross-format ordering**.
- **A universal error predicate.** The spec's own: `severity_number >= 17`. This is the "errors only"
  filter, working identically across every format in the catalogue.
- **Empty is legal and correct.** The spec explicitly sanctions omitting severity for formats that have
  none. W3C/IIS/nginx/logfmt rows leave both severity fields **empty** rather than fabricating INFO —
  avoiding the documented Loki bugs where a level word anywhere in the message hijacks detection.
- **Normative mappings ship verbatim** for RFC 5424 syslog, log4j, Zap, Windows Event Log and Apache
  access logs. HTTP-status aliases (2xx/3xx/4xx/5xx) give access-log rows a severity where wanted.
- **Resource vs Attributes maps onto pane vs row**, which is a free, principled answer to the
  merged-view column problem (§8.3).

**Level detection rules, learned from Loki's filed bugs:** field-name matching is **case-insensitive**;
the value table includes full word forms (`warning`, `information`, `critical`), not just
abbreviations; and **falling back to scanning the whole line for a level word is opt-out and always
marked low-confidence**.

### 6.3 Format detection

Five stages, **every stage bounded by BYTES, never by line count**, so a 40 GB file opens as fast as a
40 KB one.

**Stage 0 — Encoding** (§5.6). Gates everything.

**Stage 1 — Sample window.** 256 KiB head **plus ~64 KiB mid plus ~64 KiB tail.** File heads are
systematically unrepresentative (startup banners, config dumps), and for a tailing tool the tail is
what the user cares about.

> **Latency rule:** mid and tail sampling is **conditional on local paths and asynchronous**. Three
> seeks plus 384 KiB is ~30–45 ms on spinning rust and 200–600 ms on a WAN-mounted UNC share — against
> a 150 ms open budget, on exactly the deployment path Tailhawk targets. Paint from the head sample
> immediately; upgrade the format decision when the other samples land; re-render if it changed.
> A mid-file sample is decoded only from a code-unit-aligned boundary, discarding the first partial
> line.

**Stage 2 — Self-describing short-circuits.** `memmem`-class tests, no scoring, first match wins:

| Test | Format |
|---|---|
| `#Fields:` in the first 20 lines | **W3C Extended** — take columns verbatim |
| `<log4net:event` / `<log4j:event` / `<Event xmlns=…win/2004/08/events` | XML fragment stream |
| `^Event\[\d+\]:` | wevtutil text |
| `^__CURSOR=` | systemd journal export |
| `{` + `"@t":` | **Serilog CLEF** |
| `{` + `"{OriginalFormat}"` | MEL Json |
| `"resourceLogs"` in first 4 KiB | OTLP/JSON |
| `{"log":` + `"stream":` + `"time":` | Docker json-file → **unwrap and recurse** |
| `^\d{4}-…Z (stdout\|stderr) [PF] ` | Kubernetes CRI → **unwrap and recurse** |

W3C is never scored and never guessed. **`#Fields` recurs mid-file** — IIS and ASP.NET Core
`W3CLogger` re-emit it on rotation and config change — so column definitions are re-read whenever a
new directive appears and subsequent rows are re-keyed.

**Stage 3 — Scored matching.**

```
score = match_rate
      × (0.5 + 0.5 × field_validity)
      × specificity
      × (0.7 + 0.3 × coverage)
```

`field_validity` — does the captured timestamp parse to a real datetime, and is the captured level in
the format's declared set — is the term that kills the classic false positive where a loose
`<date> <word> <rest>` pattern matches but the "level" is the word `Starting`.

**Acceptance requires score ≥ 0.75 AND a ≥15% margin over the runner-up.** Otherwise a disambiguation
chip appears: *"Detected: Serilog (file) — also matched log4net. Change ▾"*. **Silent mis-columnising
is worse than no columnising.**

> **Amended 2026-08-17 (session 18), when the detector was built.** As first written, specificity was
> a factor of the score *and* the score had to reach 0.75, so every format below 0.75 specificity —
> generic timestamped text, logfmt, NDJSON, Python, Serilog console, RFC 3164 — could never be
> accepted. Stage 4 already calls specificity an *ordering*, and that is the reading implemented:
> the **acceptance threshold applies to the quality terms**
> (`match_rate × (0.5 + 0.5 × field_validity) × (0.7 + 0.3 × coverage) ≥ 0.75`), and
> **specificity multiplies in for the ranking and the 15 % margin**, which is where a generic
> pattern must lose to a specific one. `detect.rs`.

**Stage 4 — Specificity ordering.** RFC 5424 0.95 · NLog 0.90 · MEL Simple 0.90 · Serilog file 0.85 ·
Apache/nginx 0.85 · log4net 0.80 · MEL Systemd 0.75 · RFC 3164 0.70 · Serilog console 0.65 · Python
0.60 · NDJSON 0.55 · logfmt 0.45 · CSV/TSV 0.40 · generic timestamped 0.20 · plain text 0.00.

**Stage 5 — Runtime resilience.** **No permanent lock-on** (lnav's weakness). A rolling non-match rate
over the last ~1,000 records, excluding recognised continuations, above ~20% raises a **non-modal**
"this file may have changed format — re-detect?" affordance.

**Detector ordering is a build-time unit test, not a runtime heuristic.** Every format ships `sample`
lines with expected level; the build cross-matches every format's samples against every other format's
pattern and **fails** if a generic format outscores a specific one on the specific one's samples.

### 6.4 Multi-line records

**A new record starts if and only if the line matches the format's first-line anchor.** Everything
else is a continuation.

- Every format pattern is `^`-anchored and matches only a first line, so a stack-trace line containing
  a date mid-string cannot falsely start a record.
- **A one-byte dispatch table on the first non-space character** (digit, `[`, `<`, `{`, or a level
  token) eliminates >95% of continuation lines with zero regex work. This is the single largest parser
  performance win.
- **Per-family continuation predicates** enable styling and **collapse**: .NET `^\s*(at\s|--->\s|--- End of inner exception)`,
  Java `^\s*(at\s|Caused by:|\.\.\.\s\d+\smore|Suppressed:)`, Python `(Traceback \(most recent call last\):|^\s{2,}File ")`,
  MEL Simple `^ {6}`. "Collapse stack trace" is a feature neither Hoo WinTail nor LogExpert offers.
- **Row/line duality:** rows are logical records; **line numbers shown to the user are physical line
  numbers** so they match what every other tool reports.

**Scroll position representation — three rules, all measured.** `experiments/g7-egui-scroll`
reproduced egui #1391 and found that the `u64` rule alone is **not** sufficient. Two of the three
rules below do not follow from it, and each corresponds to a measured failure mode:

1. **Scroll position is `(u64 row, f32 sub_row_px)`**, with `sub_row_px ∈ [0, row_height)`. Never an
   absolute content-pixel coordinate, in any float width. Wheel and drag deltas are applied to the
   sub-row remainder and carried into the row index, so a small delta is never added to a large
   number. *(In egui, `state.offset -= delta` on an f32 content offset discards a 2 px drag entirely
   at 4M rows and stops responding to the wheel at all past ~100M.)*
2. **Row layout is computed from `(row − top_row)`, a small integer** — never
   `row * row_height − scroll_offset_px`. **This is the rule that does not follow from rule 1**, and
   the one most likely to be reintroduced by someone who believes the `u64` mandate settles the
   matter. It is a single plausible-looking line that reproduces the whole bug inside an otherwise
   correct grid: differencing two content-magnitude f32s to obtain a sub-row result flings the first
   drawn row across a 512 px band at 160M rows, *with the scroll position exact*.
3. **Never mix screen-space and content-space coordinates in one expression.** Resolve to
   viewport-relative first, then add the window origin. *(egui's `inner_rect.min − state.offset`
   destroys the window's y origin outright above ~100M rows.)*

**Required test, and it belongs in CI:** assert that the first drawn row's viewport-relative y stays
in `(−row_height, 0]` across a scroll sweep at 10⁸ rows, **and that every other visible row sits
exactly `i × row_height` below it**.

*~~That one assertion catches all three.~~ **Withdrawn — true of egui's architecture, not of ours.**
The first-row assertion alone does not catch rule 2 once the position is an exact `(u64, f32)`: the
two content-magnitude terms are then the same expression and cancel perfectly at `i = 0`, so a
faithful rule-2 reintroduction leaves the first row exact and misplaces every row after it. egui's
first row is misplaced only because its `offset` is an independently-rounded `f32`, which violates
rule 1 as well. Measured by mutation both ways in `grid.rs`.*

**Rules 1 and 2 are about an unbounded axis; rule 3 is not, and it applies to horizontal scrolling
too.** A line is bounded by §10.3's render cap, so the horizontal offset may be an ordinary pixel
count — but a click resolved as `x + offset_px` still hit-tested one column to the right at a 96,624 px
offset, because one ULP there is 0.008 px and a fractional `x` rounds up onto the next column
boundary. Both terms being exact does not make their sum exact. Resolve the offset to the leftmost
visible column once, then measure the click from there in viewport-sized numbers.

**Row height model — this is a load-bearing constraint, not a detail.** The `u64` scroll model gives
O(1) row→pixel mapping *only* under uniform row height. Two features threatened it:

| Feature | Resolution |
|---|---|
| **Expand-in-place** for continuations | **Continuations are collapsed by default**, so the base view is genuinely fixed-height. This matters most for MEL Simple logs, where multi-line records are the *default state*, not a user action. Expansions are held in a **capped side table** — a sorted `Vec<(row, extra_height)>` giving O(log N) offset lookup — with a hard cap on simultaneously expanded rows. |
| **`--wrap` / word wrap** | **Cut from the v1 surface.** Wrapping makes document height a function of window width, requiring recomputation across 50M rows on every resize and every `WM_DPICHANGED`. v1 ships **horizontal scrolling instead** (§3.3 gives the extent). If wrap is reinstated later it costs an additional 1.0–1.5 weeks on the grid component and needs an estimate-and-correct scrollbar model. |
- **While tailing, the trailing record is provisional.** It is rendered immediately and revised in
  place as more bytes arrive. Waiting for a following first-line match would make the tail appear one
  record behind — the classic annoyance.

**Three classes of non-matching line, never conflated:**

| Class | Behaviour |
|---|---|
| **Continuation** | Appended to the previous record, rendered dimmed and indented inside the expandable row |
| **Interleaved foreign** | Matches a *different* known format with high confidence — parsed with it, row badged. Multi-format files are the norm on Windows, not an edge case |
| **Unparseable** | Own row, raw line in Body, left gutter stripe, filterable via "show only unparsed" |

**A line is never dropped and never silently merged.** The status bar shows parse health —
`99.2% parsed · 812 continuation · 14 unparsed` — which is far more actionable than a confidence
percentage the user cannot verify.

### 6.5 User-defined formats — three tiers, no compiled plugins

1. **Override.** A format chip in the status bar shows the detected format and confidence, with a
   dropdown of built-ins plus "Plain text" and "Define new…". The choice is remembered **per path and
   per glob**, so `C:\logs\jobdispatcher\*.log` sticks.
2. **Define from example.** Right-click a representative line → Tailhawk proposes a tokenisation (it
   already recognises timestamps, bracketed groups, level words, quoted strings) → live preview grid
   over the next 200 lines with a match-rate readout → drag column boundaries, name columns, assign
   roles. Generates a **pattern DSL** string (`<ts> [<thread>] <level> <logger> - <message>`, `<_>`
   discards), not a regex — it compiles to a linear scanner with no backtracking. "Edit as regex" is
   the advanced escape hatch.
3. **Template import — the .NET differentiator.** Paste the layout string you already have:
   - Serilog `outputTemplate` — `{Timestamp:yyyy-MM-dd HH:mm:ss.fff zzz} [{Level:u3}] {Message:lj}{NewLine}{Exception}`
   - NLog `layout` — `${longdate}|${level:uppercase=true}|${logger}|${message}`
   - log4net `conversionPattern` — `%date{ISO8601} [%thread] %-5level %logger - %message%newline`
   - Logback pattern — `%d{HH:mm:ss.SSS} [%thread] %-5level %logger{36} - %msg%n`

   …and Tailhawk compiles it to the extraction pattern and column set.

   **The paste is the differentiator; the scan is a convenience.** `Wizard::paste` takes the layout
   as *text*, so it works wherever the text can be got — source control, a pull request, a message
   from whoever owns the service. That matters because the assumption behind the alternative is
   usually false: **a viewer normally has the log and nothing else.** Logs are copied off a server,
   read from a share, or sent by someone; co-location with the running application is the
   development machine and not much else, and it does not exist at all when the application ships
   its logs to Loki or Seq rather than writing beside its own config.

   **"Scan for logging config"** walks up from the log file's directory for `appsettings.json`,
   `NLog.config` and `log4net.config` and offers the layouts it finds. It turns format definition
   into two clicks **when the config happens to be reachable**, and is worth having for exactly
   that case — but nothing should be designed on the assumption that it is the common one.

   *Not supported:* Serilog `ExpressionTemplate`. Its `{#if}`/`{#each}` directives make optional
   segments appear and disappear line to line, so the confidence scorer must not penalise a format
   because only 60% of lines carry a parenthesised source context.

**Anti-requirement: no compiled plugins, ever.** LogExpert's compile-a-C#-DLL columnizer model is
precisely the heavyweight extensibility that makes it feel clunky and it is incompatible with
copy-deployment.

**Storage:** `tailhawk.formats.toml`, using **the single state-resolution scheme in §12.4** — no
separate rule. Every definition carries `sample` lines and a **Test** button.

---

## 7. Highlighting, filtering, search

### 7.1 Highlight rules

- Rules are **plain text or regex**, each with foreground colour, background colour, and a
  whole-line-vs-match-only toggle. Regex capture groups may be sub-highlighted.
- Rules live in named, **importable/exportable** sets (Hoo WinTail's and TextAnalysisTool's model —
  the best-liked in the category). Per-rule enable/disable.
- A rule set can be **bound to a file glob or a detected format**, so opening a matching file applies
  it automatically. **No incumbent does this.**
- **Zero-config semantic layer beneath user rules** (tailspin's model): timestamps, durations, numbers,
  IPv4/IPv6, GUIDs, URLs, Windows and UNIX paths, HTTP methods and status codes, `key=value` pairs,
  quoted strings, hex/pointer addresses, severity keywords. klogg's fatal UX flaw is an empty
  highlighter set on first run.
- **Stable derived colours for recurring identifiers** — the same request ID is the same colour
  everywhere, in every file.
- Highlights are computed **for visible rows only**, never for every ingested line.

### 7.2 The filter expression grammar

`level >= Warning` appeared in three UI mockups, `severity_number >= 17` was called the universal
error predicate, and `--filter=EXPR` shipped on the CLI — while **no document defined the language**.
It is defined here.

**Design constraint:** three of the owner's five daily-use features are *include filters*, *exclude
filters* and *multiple composing text filters*. A single text box cannot express them, so the filter
surface is a **row of chips**, each chip being one independent predicate object with its own
include/exclude polarity (§UI-DESIGN 3.1). The grammar below is what a single chip contains; chips
compose with implicit AND.

```
predicate  := bare_text | quoted | regex | comparison | membership | function
bare_text  := <any unquoted run>            case-insensitive substring over the whole record
quoted     := '"' <text> '"'                forces literal, even if it parses as an expression
regex      := '/' <pattern> '/' [flags]     flags: i (case-insensitive)
comparison := field op value                op: = != < <= > >= like
membership := field 'in' '[' value,… ']'
function   := ('startsWith'|'contains'|'endsWith') '(' field ',' value ')'
field      := 'level' | 'severity' | 'timestamp' | 'body' | 'source' | 'trace' | 'span'
            | <column name from the detected format>
            | 'attributes.' <key>
value      := number | quoted string | bare token | severity name | ISO-8601 instant
```

**Field resolution and the severity special case.** `level` and `severity` are aliases for
`severity_number`, and a severity *name* on the right-hand side resolves through the OTel banding, so
`level >= Warning` means `severity_number >= 13` and works identically across Serilog, log4net, NLog
and syslog. That is the whole point of §6.2.

**Unknown columns are the important edge case, and the rule is explicit:** a predicate naming a field
the current format does not produce evaluates to **unknown**, not false. An unknown predicate:
- **excludes** the row when the chip is an *include* chip,
- **does not exclude** it when the chip is an *exclude* chip,
- and renders the chip in a **warning state** naming the missing field.

This matters in the merged view, where sources have different column sets — an exclude chip scoped to
one source's column must not silently delete every row from the others.

**Precedence and composition.** Within a chip: functions and comparisons bind tighter than `and`,
which binds tighter than `or`; parentheses permitted. Across chips: include chips AND together, then
exclude chips are subtracted (TextAnalysisTool's model). Order of chips is display order and does not
affect the result.

**Provenance.** A chip created by UI-DESIGN §8's per-field *"filter for this value"* is a normal
comparison chip and is editable as text — there is no hidden second representation.

**CLI.** `--filter=EXPR` and `--exclude=EXPR` are repeatable; each occurrence creates one chip.

### 7.3 Filtering — and an honest account of its cost

Filtering is **not** an O(viewport) operation and the spec does not pretend otherwise. A
hide-non-matching view cannot render row *N* without knowing which records survive up to *N*, and
cannot size its scrollbar without the total match count. **Every filter change is a full-file pass.**

Therefore:
- **Debounce 300 ms**; cancel and restart the background pass on each edit.
- **Stream partial results** with a live match counter and a provisional scrollbar; never block the
  main view.
- Row indices during an incomplete pass refer to matches found so far and are explicitly labelled as
  such.
- **On network paths, as-you-type filtering is disabled** and an explicit Enter is required.
- Include filters and exclude filters are separate, with excludes applied after includes
  (TextAnalysisTool's model). Both support plain text and regex. Multiple text filters compose.

Two view modes, both shipped — klogg ships only the first and has an open request for the second:
- **Split view** — full log above, filtered matches below, selection synchronised.
- **In-place hide** — non-matching rows hidden in the main view.

### 7.4 Search

- **Engine: the Rust `regex` crate.** Lazy DFA, rare-byte memchr prefilters, Teddy SIMD multi-pattern
  — the same tricks klogg imports Hyperscan for, with no Boost/Ragel/C++ dependency.
- **Engine policy is explicit**, because `regex` has **no timeout or step-limit API** and does not need
  one (linear-time guarantee), while the lookaround/backreference escape hatch does:

  | Pattern class | Engine | Guard |
  |---|---|---|
  | Everything `regex` compiles | `regex` | `size_limit` and `dfa_size_limit` set explicitly |
  | Lookaround / backreferences | `fancy-regex` | Explicit `backtrack_limit`; **hard 8 KB per-line input cap**; cancellable worker thread; visible "pattern too slow, truncated" indicator |

  Log4net and NLog users write `(?<!DEBUG).*Exception` routinely, so rejecting lookaround outright
  would look broken — but an unbounded backtracking engine over a 500 KB serialised-object line is the
  exact hang class klogg is criticised for (#803).
- **Parallel chunked search** snaps chunk boundaries to newlines using the line index — zero overlap
  needed for line-oriented patterns. Multiline patterns add overlap ≥ max match length with
  start-offset dedup.
- **Search raw bytes where possible.** klogg's own profile shows line decoding becomes ~50% of search
  time once the regex engine is fast.
- **Results stream** — first match visible quickly, total count arriving later.
- **No persistent search index.** A re-scan is cheaper than index construction and any index is
  invalidated by every append.
- **Cross-file search** across all open sources with grouped results. klogg's #2 request, open since
  2018.

---

## 8. Multiple files

### 8.1 Layout

Flat tab strip with drag-to-reorder and drag-out-to-split. **No MDI child-window chrome.** Split panes
tile horizontally or vertically with independent or synchronised scroll.

**File sets** — a named batch of files opened in one click, and from the command line. This is a
feature the owner uses daily and it is v1.

**Watched folders** — a directory plus a glob; new matching files are adopted as they appear. Also v1,
also in daily use.

**Per-tab change indication** — a coloured dot on tabs with new content, and a **new-data separator
line** rendered where content arrived while the tab was unfocused.

### 8.2 Synchronised comparison (v2)

Three distinct features that the research conflated. Only the first two are in scope:

| Feature | Scope | Anchor |
|---|---|---|
| **Synced-scroll panes** | v2 | Timestamp where both panes have one; line offset otherwise. When formats differ or timestamps are absent, sync degrades to proportional and says so. |
| **Set-difference filtering** ("lines in A not in B") | v3 | Normalised record body |
| **True structural diff** | **Out of scope** | That is WinMerge's job |

### 8.3 Merged-by-timestamp view (v2) — fully specified

Recommended six times in research and specified nowhere. It is specified here because it is the
flagship differentiator and it is where naive implementations become visibly buggy.

**Timestamp normalisation.** Every record's timestamp is parsed to a **timezone-aware instant**.

| Format | Zone information | Policy |
|---|---|---|
| Serilog file default | `zzz` offset present | Use it |
| Serilog console default | No date at all | **Cannot participate** unless a date is supplied; the source is flagged |
| log4net `%date` | Local, no zone | Per-source timezone setting, defaulting to the local machine |
| log4net `%utcdate` | UTC | Use it |
| W3C / IIS | UTC by definition | Use it |
| RFC 3164 syslog | No year, no zone | Infer year from file mtime; per-source timezone setting |
| CRI | UTC nanoseconds | Use it |

A **per-source timezone override** is available in the UI. Parsing is **invariant-culture** with an
explicit per-format day/month order setting; a file where every day value is ≤12 is genuinely
ambiguous and Tailhawk **flags it and asks** rather than guessing.

**Sub-second precision differs** (`.fff` vs `.ffff` vs 9-digit). All timestamps normalise to
nanosecond resolution internally; display precision is a user setting (hidden / ms / µs / ns).

**Out-of-order arrival is the hard part.** Serilog's periodic-batching sink and NLog's `AsyncWrapper`
emit records whose timestamps are out of order relative to write order, routinely by up to the flush
interval. A naive live merge must therefore insert **above** the current bottom, making the viewport
jump under the user's cursor.

> **Bounded reorder window.** Merged records are held for **W (default 2 s, configurable)** before
> being committed to the view. The uncommitted tail renders in a visually distinct **"settling" band**.
> The merged view therefore **lags real time by W**, and this is documented, not hidden.

**Clock skew across machines cannot be corrected after the fact** and is documented as a limitation.
An **arrival-order** merge mode is offered as the alternative.

**Records with no parseable timestamp** are placed at their arrival position and badged. They are never
dropped.

**Scrollback and memory.** A merged view over N sources needs a merge permutation to scroll backwards.
This is the structure that would blow the memory budget (§11.2), so:

> **The merged view maintains a bounded scrollback window of the most recent 1,000,000 merged records.
> Scrolling beyond it streams a fresh k-way merge from the per-source indexes rather than holding a
> global permutation.** This limitation is in the spec, not discovered in month six.

**Columns in a merged view** are the **union** of participating sources' columns, with a **Source**
column always present. Where column names coincide (Timestamp, Level, Body) they share a column.
Per-source `resource` values (host, service) render in the pane header, not per row.

---

## 9. Trace and identifier correlation

The single most common modern debugging task — *"I have RequestId 0HN4G2…, show me every line for it
across these four services"* — which no researched tool addresses.

**Identifier detection.** GUIDs/ULIDs, W3C `traceparent`, Serilog `RequestId`, CLEF `@tr`/`@sp`,
`System.Diagnostics.Activity` TraceId flowing through MEL scopes, and any column marked
`identifier: true` in a format definition.

**Interactions:**
- Click an identifier → **highlight every occurrence** with a stable derived colour, across all open
  sources
- **Next / previous occurrence** navigation, scoped to this file or all files
- **Filter to this identifier** in one click
- **Group by operation ID** (lnav's `opid` model) — collapse a request's lines into one expandable unit

**Index decision, reconciling the "no persistent index" rule:** a **per-value index is built only for
columns explicitly designated as identifiers**, only for currently open sources, held in memory, and
discarded on close. It is not persisted. Full-text search remains index-free. This is a narrow,
bounded exception and is costed in §11.2.

---

## 10. Workflow features

### 10.1 Bookmarks

v1. Anchored on **content hash plus offset**, not raw offset, because truncation resets to 0 and
rotation invalidates line numbers. On truncation, bookmarks whose anchors no longer resolve are marked
**orphaned** (shown in a bookmarks panel with their original content) rather than silently dropped.
Numbered bookmarks (Ctrl+Shift+0–9) match Hoo WinTail. Persisted per file identity where settings are
writable (§12).

### 10.2 Export and copy

Explicitly enumerated because the research let features the owner values fall out silently:

| Action | Phase |
|---|---|
| Copy selection as raw text, **preserving original bytes and encoding** | v1 |
| Copy as TSV with columns | v1 |
| Save filtered view to a file | v1 |
| **Write matching lines to a separate file, live** (Hoo WinTail parity) | v1 |
| **HTML export preserving highlight colours** (Hoo WinTail parity) | v2 |
| Copy as Markdown / JSON for a ticket | v2 |
| Copy N lines of context around a selection | v2 |
| **Shareable view descriptor** — filters + highlights + column layout as a short string | v2 |

Selecting 2M lines and pressing Ctrl+C shows a confirmation with the estimated size before allocating.

### 10.3 Very long lines

A 40 MB single line is routine (ASP.NET body logging, serialised exception aggregates). klogg hangs
"deadly" on this (#803). Tailhawk's policy:

- **Render cap: 32 KB inline** (configurable), with a visible `line truncated — 41.2 MB · expand ·
  open in viewer · copy full` affordance
- A **single-record detail pane** shows the full record and pretty-prints JSON
- **Search and copy operate on the untruncated bytes**
- Regex work on a line is capped per §7.3
- The pathological case is a checked-in test fixture

### 10.4 Alerts and actions (v2)

Hoo WinTail's most-praised workflow feature is alerting; its SMTP model is dated but the need is not.
Extensibility splits into three questions with three answers:

| | Decision |
|---|---|
| **Custom formats** | Declarative data — decided (§6.5) |
| **Custom sources** | Fixed built-in set plus the process-spawn source (§4.1) |
| **Custom actions** | A **fixed declarative set**: toast notification, tray flash, sound, write-matching-lines-to-file. **Run-command is the security boundary** — if it ships at all it requires explicit per-file user confirmation before being armed, and can never be enabled by an imported file (§13.1). |

Also v2: **alert when a file stops updating** for N minutes, with a suppression period (Hoo WinTail
parity).

### 10.5 Deferred deliberately

**OutputDebugString capture** is **v3, not v1** — the owner did not select it. When built it must be
specified properly, not as "a small amount of code": the `DBWIN_BUFFER`/`DBWIN_DATA_READY` objects are
**single-owner per session**, so Tailhawk would fight DebugView and the Visual Studio debugger and
silently capture nothing depending on start order. It must detect that another monitor holds the
objects and say so; `Global\` capture from services/session 0 requires elevation and is a separate
action; captured strings route through the same untrusted-content sanitisation as file content (§13.4).

---

## 11. Performance and memory budgets

### 11.1 Rules for numbers in this section

Every figure here is either **measured on the reference machine** or marked **[TBM]** — to be
measured. **No figure derived from the research's fabricated throughput numbers appears here.** A
target is not valid until it has a measurement behind it, and every published figure must state corpus
size, cache state and pattern class.

**Reference machine (to be fixed before any target is set):** a dedicated perf box, x64, NVMe, stated
RAM, Windows 11. Shared CI runners cannot defend throughput claims.

### 11.2 Memory budget

The research budgeted only the line index — the cheapest structure. This is the whole-system budget,
and features are constrained to fit it.

| Structure | Order | 10 GB / 50 M lines | Notes |
|---|---|---|---|
| Line index (sparse anchors, S = 64) | O(lines) | **~6.3 MB** | 0.125 B/line — re-derived, §5.3. Was ~56 MB under the superseded block-sparse/delta design. |
| Decoder state | O(1) per source | negligible | |
| Viewport + overscan | O(viewport) | < 1 MB | |
| Column parse cache | O(viewport) | < 1 MB | Parsed lazily, visible rows only |
| Glyph atlas | O(unique glyphs × scale factors) | ~4–16 MB | Rebuilt per DPI |
| D3D11 device + DXGI swapchain | O(1) | **~30–60 MB** | Before a byte of log is read |
| **Filter match set** | **O(matches)** | 40 MB per active filter at 5 M matches | **Per filter, per source** |
| **Identifier index** (§9) | O(distinct values) | Bounded; discarded on close | Only for designated columns |
| **Merged-view window** | **O(window)** | Bounded at 1 M records | **Not O(all records)** — §8.3 |
| Sort keys | — | **Not held** | Sort is scoped, §11.4 |

**Published claim — and this is the honest, defensible one:**

> **Under 120 MB RSS with 30 files open totalling 200 GB, and flat as file size grows.**
> **The flatness is the claim, not the absolute number.**

The "beat BareTail's 2 MB" framing is **retired**. It is unachievable with a GPU-composited UI, it
rests on a 2019 forum comment, and it is the first thing a hostile reviewer would measure.

### 11.3 Latency budgets

Per §11.1's own rule, every row states its **clock origin** and **cache state** — an earlier draft
omitted both, which made the numbers unfalsifiable.

| Operation | Clock origin | Cache | Budget | Status |
|---|---|---|---|---|
| Window visible | `CreateProcess` return | warm | < 100 ms | [TBM] |
| First content painted, local file, any size | `CreateProcess` return | warm | **< 150 ms** | [TBM] — tail read + head sample only |
| First content painted | `CreateProcess` return | **cold** | [TBM] | Separate budget; dominated by I/O |
| Format detected, columns applied | first paint | warm | < 300 ms | [TBM] |
| Full index available (scrollbar exact) | first paint | either | Background; UI never blocks | Progressive |
| Follow latency, local | writer's `WriteFile` return | warm | ≤ poll interval (100 ms) + one frame | [TBM] |
| Follow latency, UNC | writer's `WriteFile` return | n/a | ≤ poll interval (250–500 ms) | [TBM] — gated on the SMB experiment |
| Frame budget while tailing | — | — | One invalidate per 16.67 ms tick regardless of arrival rate | Design rule |
| Sustained append without frame drops | — | — | **50 MB/s for 60 s** | Acceptance test |
| Full-file regex search, 10 GB | search invocation | warm | [TBM] | **Deliberately unset** — the research figure was fabricated |

### 11.4 Operations with explicit caps

| Operation | Cap | Behaviour at the cap |
|---|---|---|
| **Column sort** | Filtered result set ≤ **2 M rows** | Header shows sort affordance only when eligible; **sorting disables follow** and says so |
| **Top-N by column** | Always available | A heap over a scan — serves "show me the slowest requests" without an external sort |
| Inline line render | 32 KB | Truncation affordance (§10.3) |
| `fancy-regex` input | 8 KB per line | "Pattern too slow, truncated" indicator |
| Merged scrollback | 1 M records | Beyond it, streams a fresh k-way merge |
| Clipboard copy | Confirmation above 100 MB estimated | |

A sortable-looking column header that becomes a 30-second freeze is a defect, not a feature.

---

## 12. CLI, instance model and state

### 12.1 Two modes

- **GUI mode** (default) — `/SUBSYSTEM:WINDOWS`, with `AttachConsole(ATTACH_PARENT_PROCESS)` for
  `--help`, `--version` and parse errors. Documented honestly: the prompt returns immediately,
  `ERRORLEVEL` reflects process creation, scripted use needs `start /wait` or `--stdout`.
- **`--stdout`** — headless, byte-exact GNU `tail` behaviour: same defaults (`-n 10`,
  `--follow=descriptor`, 1.0 s sleep), same `==> file <==` header rules, same stderr diagnostics, same
  exit codes 0/1. This is what makes "full parity with Unix tail" testable rather than marketing.

No `.com`/`.exe` pair — it violates single-file deployment.

### 12.2 Option surface

**Reserved and never redefined:** `-b -c -f -F -n -q -r -s -v -z`, plus the obsolete `-NUM`/`+NUM`
positional forms. Stealing `-F` (which means `--follow=name --retry` to every Unix user) would be the
single most damaging possible CLI decision. **Every Tailhawk-specific option is long-only.**

**v1 tail surface:** `-n K` / `-n +K`, `-c` with suffixes, `-f`, `-F`, `--follow=`, `--retry`, `-s`,
`--pid` (via `OpenProcess` + `RegisterWaitForSingleObject`), `-q`, `-v`, `--help`, `--version`, `--`.
Full GNU surface including `--max-unchanged-stats` and `-z` lands with `--stdout` in v2, where the
differential test harness against real GNU `tail` also lands.

**Tailhawk options (selection):** `--new-window`, `--reuse`, `--tab|--tile|--merge`, `--profile=NAME`,
`--highlight=REGEX[:COLOUR]` (repeatable), `--filter=EXPR`, `--exclude=EXPR`, `--regex|--literal`,
`--columns=auto|serilog|log4net|nlog|clef|none`, `--column-pattern="…"`, `--theme=dark|light|system`,
`--encoding=…`, `--goto=N`, `--session=FILE`, `--stateless`, `--watch-dir=DIR --match=*.log`,
`--stdout`, `--register`/`--unregister` (§12.4). **`--wrap` is not in the v1 surface** — see §6.4.

**Globs are expanded by Tailhawk** — `cmd.exe` does not, and neither does Rust. An unmatched pattern
means *"watch this directory and adopt matching files as they appear"*, which turns glob support into
the folder-monitoring feature the owner uses.

**Paths are parsed Windows-aware before colons are interpreted:** drive prefix, `\\?\`, `\\?\UNC\`,
`\\server\share`, and only then a remaining colon as an NTFS stream separator. `--stream=NAME` opens
an alternate data stream — no mainstream Windows tail does this.

### 12.3 Single instance

A per-user, per-session named mutex detects a running instance; a **named pipe** carries the argv
(UTF-16, length-prefixed) plus the working directory so relative paths resolve. WM_COPYDATA is
rejected — it is subject to UIPI and requires finding the target window. The receiver calls
`AllowSetForegroundWindow`/`SetForegroundWindow`. Exit code **3** means "handed off to an existing
instance", so scripts can distinguish forwarding from doing the work.

### 12.4 State, settings and portability

**One resolution scheme for all persisted state.** An earlier draft defined this twice — once here and
once, differently, for format definitions in §6.5 — and left `--stateless` ambiguous between
*suppress reads* and *suppress writes*, which are opposite products.

**Read order** is always: exe-adjacent → `%APPDATA%\Tailhawk\` → built-in defaults, **merged**, with
the earlier tier winning per key. This is what makes a curated rule set travel next to the exe on a
share while a user keeps personal additions locally.

**Write target** is the first writable tier, tested by an actual write probe at startup:

| State | Read order | Write target | Notes |
|---|---|---|---|
| Settings | exe-adj → APPDATA → defaults | first writable | `tailhawk.settings.toml` |
| Format definitions | exe-adj → APPDATA → built-ins | first writable | `tailhawk.formats.toml`; **same scheme as everything else** |
| Highlight rule sets | exe-adj → APPDATA → built-ins | first writable | The artefact the owner curates most |
| File sets | exe-adj → APPDATA | first writable | |
| Bookmarks | APPDATA only | APPDATA | Keyed by file identity, one file per source, to minimise contention |
| Format / encoding overrides | APPDATA only | APPDATA | Keyed by file identity and by glob |
| Window layout, recent list | APPDATA only | APPDATA | |

**`--stateless` is precisely defined: it suppresses all *writes*, and leaves *reads* untouched.** You
can still ship a curated rule set beside the exe and have it load; nothing you change in the session is
persisted. The status bar shows the *"settings will not be saved"* chip **only** when a write was
actually attempted and no tier was writable, or when `--stateless` is active — **not** merely because
the exe directory is read-only, since in that case tier 2 silently and correctly absorbs the writes.

Concurrent instances use atomic replace-on-write with last-writer-wins.

**No on-disk index cache in v1.** It undermines the portability claim for a marginal second-open win.

**Shell integration registers nothing by default.** `--register` / `--unregister` add the Explorer
"Tail with Tailhawk" verb, a `SendTo` entry and `.log` association, run deliberately by the user. This
is what portable tools conventionally do.

---

## 13. Security, privacy and trust

### 13.1 Imported configuration is untrusted input

Tailhawk actively encourages sharing format files, filter sets and sessions. Each is a threat vector:

| Vector | Control |
|---|---|
| **Regex hang payload** in a shared format or filter set | Compile with explicit `size_limit`/`dfa_size_limit`; reject or hard-bound backtracking constructs; all rule evaluation on a cancellable worker with a per-frame budget — a pathological rule degrades to "highlighting paused", never a frozen window |
| **UNC path in a shared session → NTLM credential leak.** Opening a session containing `\\attacker\share` triggers outbound SMB authentication with no user action | **Any path that arrived from an imported file and is UNC or remote requires explicit per-path confirmation before opening.** Paths typed by the user do not. |
| **Command injection** via an imported file specifying an action or source | See the field taxonomy below |
| Malformed config | All config parsing is fuzzed (§14.3) |

**Field taxonomy for imported files — replaces an earlier intent-based deny-list.** The previous
wording said an imported file "can never specify a command, program path or **network destination**",
while the row above it required *per-path confirmation* for imported UNC paths. Those are
contradictory, and the stricter reading ships an importer that **rejects every real shared file set**,
because a team file set necessarily contains UNC paths. File sets are v1, owner-daily and explicitly
shareable, so this must be exact:

| Field class | Examples | On import |
|---|---|---|
| **Inert data** | Colours, names, enable flags, column layouts, severity thresholds, glob *patterns* | Accepted silently |
| **Patterns** | Regexes, pattern-DSL strings, format definitions | Accepted, but compiled with explicit `size_limit`/`dfa_size_limit`; backtracking constructs hard-bounded |
| **Local paths** | `C:\logs\app.log`, relative paths | Accepted; resolved but not opened until the user opens them |
| **Remote paths** | UNC, mapped network drives | **Accepted, listed, and opened only after explicit per-path confirmation** naming the host. Not rejected — this is the file-set case. |
| **Executable intent** | Command lines, program paths, action definitions, `--register` directives, environment variables | **Rejected at parse time**, with the offending field named. Never ignored silently. |
| **Unknown fields** | Anything not in the schema | Rejected, not skipped — forward-compatibility is handled by an explicit schema version, not by tolerance |

The distinction is **executable intent vs data**, not local vs remote. A UNC path is data that requires
consent; a command line is intent and is never accepted from a file.

### 13.2 Privacy — zero network by default

Tailhawk's entire working set is customer log files containing PII, connection strings, bearer tokens
and session IDs.

- **No telemetry. No update ping. No font or CDN fetch. No outbound connection of any kind** unless the
  user explicitly opens a remote source. This is a **testable assertion** in CI and a competitive
  claim worth making.

  > **The assertion does not exist yet, and saying so here is the point.** Checked 2026-08-27: CI
  > asserts the binary carries no runtime shader compiler and no CRT redistributable, and nothing
  > about sockets. The claim above has been true by construction — no HTTP code has ever been
  > written — and construction stops being a guarantee the moment the Loki source lands. **Writing
  > the test is therefore a prerequisite of the first HTTP call, not a follow-up to it**, and it is
  > the only thing that keeps the *conditional* form of this claim ("unless the user explicitly
  > opens a remote source") checkable rather than merely asserted. `LOKI.md` §13.2 carries the
  > shape: a run over a local file must open no socket, and the network path must be reachable only
  > from an explicitly configured source.
- **No conventional crash reporter.** A minidump would exfiltrate customer log content. If crash
  reporting exists, it is **local-file only**, user-initiated, with a **visible preview of exactly what
  will be sent**, and never contains buffered log content or full file paths without redaction.
- **Tailhawk's own diagnostic log** is off by default, enabled by a flag, written exe-adjacent if
  writable else `%TEMP%`, opened from a UI action, and **never contains log file content**.
- **stdin spill files** (§4.2) are created with a restrictive DACL in `%TEMP%`, deleted on clean exit,
  and reaped on next launch if orphaned. The spill location is displayed in source properties, because
  a user piping production logs deserves to know where they landed.

### 13.3 Updates

**No in-app self-updater.** A check-for-update link only, and package-manager updates via scoop and
winget. Rationale: a running exe cannot be deleted (though it can be renamed); an exe on a shared UNC
path cannot be replaced while colleagues hold it open; and a downloaded update that is not Authenticode
-verified before execution is an auto-RCE channel. If a self-updater is ever added, signature
verification before execution is non-negotiable.

### 13.4 Rendering untrusted content

Log content is frequently attacker-influenced (User-Agent, URI, echoed user input). **A viewer that can
be made to lie is a real defect** for a security-adjacent tool.

- **ANSI:** CSI sequences are stripped before format matching (MEL's Simple formatter colours by
  default, so files captured via `tee`, Docker or a PTY carry escapes that break every regex). If ANSI
  rendering is enabled, **only SGR colour and intensity are honoured** — never OSC 8 hyperlinks, never
  OSC 52 clipboard, never title-setting, never DCS.
- **Bidi:** each line's bidi context is **isolated** so a U+202E override cannot escape its line
  (Trojan Source).
- **"Reveal invisibles"** toggle renders bidi controls, zero-width characters, other Cf-category code
  points, stray control characters and NUL bytes visibly.

### 13.5 Supply chain

`cargo-deny` and `cargo-audit` in CI with a licence allow-list (which also enforces the GPL clean-room
decision). An SBOM ships with each release. Dependencies are pinned in `Cargo.lock`, vendored for
release builds, and MSRV is pinned via `rust-toolchain.toml`. **Stable releases only** — no release
candidates, applied uniformly.

---

## 14. Accessibility, internationalisation, testing

### 14.1 Accessibility — a named workstream, not a linked crate

AccessKit provides platform adapters; it does **not** provide a virtualised accessibility tree, and a
10 M-row grid cannot emit 10 M nodes. Windows UI Automation has `ItemContainer` and `VirtualizedItem`
patterns designed precisely for this.

**The workstream is split in two, because an earlier draft simultaneously said accessibility "cannot
be deferred" (it is the only automated UI-test surface), described it in the present tense as shipping,
and deferred all of it to v2:**

| Part | Scope | Phase |
|---|---|---|
| **Chrome provider** | `IRawElementProviderSimple` over tabs, buttons, status chips, filter-panel controls, dialogs — enough for names, values, focus order and **automated interaction testing** | **v1**, ~1.5–2.0 weeks, in M7 |
| **Grid text provider** | Virtualised `ITextProvider`/`ITextRangeProvider`/`IScrollProvider` with caret and selection eventing over tens of millions of rows | **v2**, 6–10 weeks |

Without the v1 half, the largest hand-written subsystem in the product — tabs, drag-out-to-split,
palette, rules editor, format wizard, eleven text fields — ships with **zero automated interaction
coverage**, validated forever by one person dragging a mouse. Golden-image tests cannot cover
drag-resize, focus order, rectangular selection, palette keyboard navigation or IME composition.

- **Live-tail announcement policy:** naive live-region announcement at 1,000 lines/s is unusable.
  Follow mode is **quiet by default**, with an explicit "read new lines" action and an optional
  "announce matches of rule X only" mode.
- **High Contrast:** system colours are respected and user highlight rules are **suppressed** (they
  would be invisible or illegible), with a visible indicator explaining why.
- **Colour-blind safety:** the default severity palette is colour-blind safe and **severity always has
  a redundant non-colour channel** (a gutter glyph and weight), not hue alone.
- Full keyboard operation with a defined focus order across a custom-drawn surface.
- **Accessibility is also the only automated UI-testing surface** for a custom-drawn D3D11 grid, which
  is why it cannot be deferred indefinitely.

### 14.2 Internationalisation

Covered structurally in §3.3 (grapheme clusters, East Asian Width, colour emoji, font fallback) and
§8.3 (invariant-culture timestamp parsing with explicit day/month order). UI strings are externalised
from v1 even though only `en-GB` ships.

### 14.3 Testing

| Concern | Approach |
|---|---|
| **Large-file fixtures** | A **checked-in seeded generator**, never checked-in data. Tiers at 1 MB / 1 GB / 10 GB; the 10 GB tier runs on the dedicated perf box, not shared CI. |
| **Rotation, truncation, network drop** | A **virtual-filesystem trait** so these are injected deterministically in unit tests. A small number of real end-to-end rotation tests run against the real API. |
| **The writer-safety guarantee** | Rotation-loop test asserting no `ERROR_SHARING_VIOLATION`/`ERROR_USER_MAPPED_FILE` on the writer side |
| **Parsers** | `cargo-fuzz` targets for every one: format detector, encoding detector, CLEF/JSON, W3C header, logfmt, CSV, session and format files |
| **Format detection** | A golden corpus with expected labels, plus the build-time cross-matching test (§6.3) |
| **Rendering** | Golden-image tests per DPI scale; the CJK/RTL/emoji fixture; the 100%↔150% monitor-drag column-drift test |
| **Cold-cache timings** | Standby list dropped between runs (`EmptyStandbyList`); cold and warm reported separately |
| **`--stdout` parity** | Differential harness against real GNU `tail` (available on the runner via WSL or a vendored uutils build) |
| **Performance** | Regression gate on the dedicated perf box. Shared runners cannot defend throughput claims. |

---

## 15. Phasing

The owner's stated appetite is the **full vision, however long it takes**. Phasing therefore sequences
the vision rather than cutting it — but the cut lines are explicit so that a coherent, shippable
product exists at the end of each phase.

### v1 — "Better than the tool I use today"
Single/multi file with tabs and splits · instant open with background index · follow with correct
rotation and truncation · encoding detection · **highlight rules with import/export** · **include and
exclude filters** · **file sets** · **watched folders** · full-file and cross-file search · format
detection and columnisation for the Serilog / log4net / NLog / MEL / W3C / CLEF families · template
import and config scanning · pattern-DSL custom formats · bookmarks · stdin · core tail CLI
(`-n -f -F -c`) · dark and light themes · settings with stateless mode · zero-network guarantee.

**Phasing of the two differentiators is an owner-confirmed decision, not an omission.** Merged
timeline and trace/causality navigation were explicitly considered for v1 and kept in v2: merged view
depends on timestamp normalisation being correct across every format, which is not proven until M6, so
building it earlier would build it on sand. v1 already beats every incumbent on tabs, splits and
split-view.

### v2 — "Things nothing else does"
**Merged-by-timestamp view** with the bounded reorder window · **trace/identifier correlation** ·
process-spawn sources (docker/kubectl/az) · compressed archive members · synced-scroll comparison ·
alerts and actions · HTML export and shareable view descriptors · full `--stdout` GNU parity with the
differential harness · **accessibility workstream**.

### v3 — "Depth"
Timeline histogram scrubber · dedup modes (Exact / Numbers / Signature) · Loki-style pattern mining
over a bounded sample · live Windows Event Log source · OutputDebugString capture · set-difference
comparison · OTLP/JSON file reading.

### Deferred, gated on evidence
**OTLP receiver** — gated on evidenced demand, and only ever off-by-default, opt-in per launch,
`127.0.0.1`-bound, logs-only. **A second platform shell** — gated on a pre-agreed demand threshold, not
on a roadmap date.

---

## 16. Traceability to the adversarial findings

| Finding | Resolved by |
|---|---|
| mmap contradiction across topics | §5.2 — excluded entirely, with the rotation-blocking rationale |
| "Parallel indexing is context-free" is false for UTF-16/DBCS | §5.3 — alignment invariant stated. **Half-retracted 2026-07-31:** true for UTF-16/32, where alignment is the fix; **false for DBCS**, measured. The finding was right that the research claim needed challenging and wrong about which half failed. |
| Memory model budgets only the line index | §11.2 — whole-system table with per-feature O() |
| "Beat BareTail's 2 MB" | §11.2 — retired; flatness claim substituted |
| Fabricated ripgrep throughput → search targets | §11.1, §11.3 — search target deliberately unset, [TBM] |
| Filtering is not O(viewport) | §7.2 — stated honestly with debounce and streaming |
| Sort presented as free | §11.4 — capped, disables follow, top-N alternative |
| AccessKit is not a virtualised tree | §14.1 — named 6–10 week workstream |
| Monospace cell model breaks on CJK/emoji | §3.3 — grapheme clusters and EAW from day one |
| `fancy-regex` reintroduces the klogg hang class | §7.3 — explicit engine policy with caps |
| Settings storage contradicts read-only UNC | §12.4 — three tiers with stateless mode |
| Shared config as an attack surface | §13.1 — including the NTLM leak vector |
| ANSI/bidi injection | §13.4 — SGR whitelist, bidi isolation, reveal-invisibles |
| No testing strategy for the 10 GB path | §14.3 — generator, VFS trait, perf box |
| Merged view specified nowhere | §8.3 — fully specified with reorder window and scrollback bound |
| Trace-ID navigation absent | §9 — with a bounded index exception |
| Compressed archives relegated to an open question | §4.3 |
| Very long lines had no product answer | §10.3 |
| Platform envelope never stated | §2.1 |
| Licence and GPL contamination never decided | §2.2 — MIT/Apache-2.0 with a clean-room rule |
| Updating a portable exe unresearched | §13.3 |
| Telemetry/privacy unmentioned | §13.2 |
| OTel receiver over-scoped | §15 — deferred; pipes cover the container case |

**Second review round** (9 critics against the four written artifacts):

| Finding | Resolved by |
|---|---|
| Encoding and cell model scheduled *after* the index that depends on them | PLAN §4 resequenced: decode → index. §3.3 drops the "cell count captured free" claim for `max_byte_len` + `all_ascii` |
| Filter grammar shown in three mockups, defined nowhere | **§7.2** — full grammar, field resolution, unknown-column rule |
| One filter box cannot hold include + exclude + multiple text filters (3 of 5 owner-daily features) | §7.2 + UI-DESIGN §2.1 — chip surface |
| Expand-in-place and `--wrap` destroy the O(1) scroll model | §6.4 — continuations collapsed by default, capped side table, `--wrap` cut from v1 |
| `text_rendering` setting contradicts the owner's stated answer | §3.2 — setting removed; PLAN G4 repointed at colour-glyph atlas composition |
| WARP is both a fallback step and the trigger to abandon the fallback chain | §3.2 — WARP is a backend, RDP is a transport condition; detected independently |
| Import deny-list rejects every real shared file set | §13.1 — explicit field taxonomy; executable intent vs data |
| Two state-storage schemes; `--stateless` ambiguous | §12.4 — one table, read/write separated, `--stateless` = suppress writes only |
| Accessibility "cannot be deferred", written as shipping, deferred entirely | §14.1 — split: chrome provider v1, grid text provider v2 |
| ARM64 "first-class" with no device and no reference machine | §2.1 — best-effort, cross-compiled, unmeasured, no published figures |
| Latency budgets with no clock origin or cache state | §11.3 — both stated per row, per §11.1's own rule |
| Long-line cap 32 KB vs 40 KB across documents | UI-DESIGN aligned to 32 KB |

---

## 17. Open decisions

These are **not** deferred design work — they are decisions that require input or a gating experiment
before implementation can safely proceed.

1. **Code-signing route — blocking.** **Azure Artifact Signing is unavailable.** Its Public Trust
   certificates are restricted to organizations in the US, Canada, the EU, the UK, Australia, New
   Zealand, Japan, South Korea, Singapore, Switzerland, Norway and Israel, and individual developers
   to the US and Canada. **The owner is South African — in neither list — so no legal-entity decision
   unlocks it.**

   The remaining routes, in order of preference:

   | Route | Cost | Key handling | Status |
   |---|---|---|---|
   | **Certum "Open Source Code Signing"** | from €69 gross | Hardware token (card + reader) shipped | **Eligibility for a South African individual unconfirmed — confirm with Certum first** |
   | Conventional OV cert (Sectigo / DigiCert / GlobalSign / SSL.com, direct or via reseller) | Higher; plus $90–250 token | Hardware token, or a CA-hosted cloud-signing service where offered | Generally available internationally; needs per-CA confirmation |
   | **Ship unsigned initially** | £0 | — | Viable: scoop imposes no signing requirement, and SmartScreen shows "unrecognized app" for *signed-but-new* binaries anyway |

   Since 1 Jun 2023 all publicly-trusted code-signing private keys must live on FIPS 140-2 L2 /
   CC EAL4+ hardware, so a token or a cloud HSM service is unavoidable for any signed route.

   **The identity, once chosen, can never change** without resetting SmartScreen publisher reputation
   to zero. **Windows 11 Smart App Control** blocks unsigned executables without positive reputation
   regardless of download origin, so unsigned is a launch posture, not a permanent one.
2. **`tailhawk.com` / `.dev` / `.io` availability** — never checked. Registrar confirmation needed.
3. **SMB stale-size experiment** (RESEARCH.md §12.1) — if polling an open handle over SMB returns
   cached sizes, §5.4's UNC design needs a periodic handle reopen.
4. **Reference perf machine** — must be fixed before any [TBM] target becomes a number.
5. **Hoo WinTail hands-on** — the owner's installed copy is the only reliable source for its exact
   encoding behaviour and for which of its features are used in a typical week.
6. **Does the owner's workflow actually want an OTLP receiver**, given they already run Grafana/Loki
   and `az containerapp logs` is pipeable? The answer may collapse the deferred item entirely.
