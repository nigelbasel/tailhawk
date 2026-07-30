# Handoff — resume here

**Paused:** 2026-07-30, session 7, after finishing M0.
**Everything below is on disk and pushed. Nothing is held in a chat session.**

---

## 🚀 M0 is done — 2026-07-30, session 7. There is a running application.

`cargo run --release -p tailhawk` opens a window. That is the command to use whenever you want to
start something up and look at it; it will show progressively more as M1–M3 land.

**Owner direction, session 7:** keep to the planned milestone order — **M2 is not deferred**. This is
a side project, it does not need to be in daily use, and **robustness is preferred over reaching a
usable UI sooner**. The dogfood-first reordering (skip M2, trim M3) was offered and declined.

| M0 criterion (`PLAN.md` §4) | State |
|---|---|
| Cargo workspace with the core/shell split from commit one | **Done** — `crates/tailhawk-core` (portable, owns rendering) + `crates/tailhawk` (window, message loop) |
| A window that opens | **Done** — verified opening, responding, titled with the driver it got |
| D3D11 device with the WARP fallback chain | **Done** — hardware → WARP; comes up on hardware here |
| An embedded shader | **Done** — `fxc` at build time via `build.rs`, DXBC embedded, CI asserts no `d3dcompiler` import |
| `+crt-static` | **Done** — 249,344 bytes, imports **only OS DLLs**; the dynamic build needs `vcruntime140.dll`, the static one does not |
| CI producing x64 and ARM64 under the size gate | **Done, x64 verified locally. ⚠ ARM64 unverified** — this machine's VS install has no ARM64 linker, so that leg is proven only when CI first runs. `fail-fast: false`, so x64 still reports. |
| **Done:** opens on a clean Windows 10 1809 VM with no runtime installed | **Not literally tested** — no such VM here. The dependency surface that criterion is really about *is* verified: only OS DLLs. |

**Choices worth not re-litigating:**

- **The core cannot name an `HWND`.** The drawable crosses the seam as an opaque `WindowHandle(isize)`,
  so `SPEC.md` §3.1's portability rule is enforced by the type rather than by discipline.
- **Leaf backends are modules inside the core, not separate crates.** Splitting them buys nothing
  until a second platform exists, and it is a mechanical move when one does.
- **Both paint stages take their colour from one constant** in the core, with a unit test asserting
  the `f32` and 8-bit forms agree. They are necessarily written twice — GDI wants bytes, the render
  target wants floats — and drift would show as a flash on every cold start.
- **Stage one is the class background brush**, which the system draws during `ShowWindow` before any
  handler runs. §3.2 measured a brush as *equivalent* to a `FillRect`, so this is the simpler of two
  equal options.
- **M0 draws through the shader rather than `ClearRenderTargetView`.** Same pixels, more cost — the
  point is that the offline-compile path M3 depends on is proven now rather than then.
- **`clippy` and `fmt` in CI are scoped to the product crates.** The experiment crates trip four
  pre-existing lints; they are throwaway measurement code whose hand-formatting is part of what they
  record. They are still compiled by the workspace build, so they cannot rot into not building.

**Next: M1 — read and decode** (`PLAN.md` §4). Headless: open a file, detect encoding, stream decoded
lines with correct carry across read boundaries. **Decode before index** — that ordering is the
dependency inversion adversarial review caught, and it is why M1 is not the indexer.

---

## ✅ Batched rasterisation is done — 2026-07-30, session 7

The highest-value remaining experiment ran, as `experiments/g4b-batched-raster`. Full write-up in
`experiments/g4b-batched-raster/RESULTS.md`. Machine quiet throughout (2–5% CPU, zero leaked
subjects). It answered its own question and then overturned the premise behind it.

**Batching works and is not the win.** Batched cells are **bit-identical** to per-glyph cells at any
inter-cell gap including zero, so it is safe for an atlas. It is worth **~1.8x on cold glyphs**,
saturating at **4 glyphs per analysis** — real, worth taking, but not the order of magnitude that made
it the top-ranked item. Past 256 glyphs per analysis it becomes a *pessimisation*. On warm glyphs it
wins nothing.

**What actually dominates: a cross-process, capacity-limited system font cache.**

| | µs/glyph |
|---|---|
| first process on the machine to rasterise a glyph | **86 – 108** |
| any later process, same glyph and size | **~3** |
| capacity | **8,000 – 16,000 distinct glyphs** (bracketed, not pinned) |

36x, and it survives process exit — so it is system state, not process state.
`Windows Font Cache Service` is the inferred mechanism (not verified; confirming it means stopping a
system service).

**Three earlier conclusions are corrected.**

1. **G4's 330–388 µs/glyph is a cache-thrash figure.** Its fixture cycles 20,992 distinct CJK glyphs
   to force *atlas* eviction — which was the point — and incidentally never fits the font cache
   either. Re-run today the same binary gives **92–97 µs/glyph**. The eviction conclusions (the O(1)
   LRU requirement, the 20–28x ratio) are untouched and stand.
2. **Session 6's anomaly is explained, and its mechanism was wrong.** Session 6 saw the quiet
   post-reboot re-take land at the *top* of the range and concluded "a cold run does not bound this
   cost favourably — the spread is not load". Right observation, wrong cause: **a reboot empties the
   font cache**, so that run was the most cache-cold and therefore the slowest. It was never about CPU
   load in either direction.
3. **`SPEC.md` §3.2's "rasterisation off the paint path" survives but needs re-scoping — owner's
   call.** It is a **first-run** cost, not a per-viewport one: a genuinely cold 1,500-glyph viewport is
   **162.5 ms** (8–10 frames, so placeholders stay a v1 requirement), but steady state is the same
   viewport in **4.4 ms**, inside one frame. The spec currently implies the former is permanent. Not
   edited — that is a normative change.

**Method note that cost the session an hour and a wrong answer:** the first version of this experiment
reported 2.5 µs/glyph and concluded G4 was wrong by 150x. It was measuring cache hits left behind by
its own previous run. An in-process "is there a cache?" probe cannot detect a cross-process cache and
returned a confident 0.99x. See the traps table.

---

## ✅ The cold set is taken — 2026-07-30, session 6

The reboot happened and the set was taken in the first ten minutes, on the existing
`target-verify-static\release\` binaries with no rebuild. **Six 11-run sets: D2D, D3D11 serial and
D3D11 earlypaint, each in two conditions** (boot churn at 36% CPU, then quiet at 0–6% CPU), plus a
quiet G4. Leaked-subject count verified at **0** before and after every set. Full write-ups are in
`experiments/g3-d3d11/RESULTS.md`, `experiments/g3-d2d/RESULTS.md` and
`experiments/g4-glyph-atlas/RESULTS.md`.

**The branch that held: background load explains the whole absolute spread.** Every quiet figure lands
at or below the fast end of session 5's range. So G5's reference machine is *not* needed to settle the
shape of the result — it is still needed before any number is published in `SPEC.md` §11.3.

| | session 5 (loaded) | quiet, session 6 |
|---|---|---|
| **earlypaint first pixel** | 54.7 / 66.3 ms p50 | **13.1 ms p50, 14.5 p90** |
| D3D11 serial, total | 126.4 ms p50 | 68.6 ms p50 |
| D2D, total | 156.7 ms p50 | 75.5 ms p50 |
| `CreateWindowExW` | 8.5 – 11.9 ms | 3.2 – 3.9 ms |

**Three conclusions changed.**

1. **The "~50–60 ms window-presentation floor" does not exist — it was load.** `first_pixel` measures
   `main()` entry → `FillRect` returning in the first `WM_PAINT`, so `ShowWindow` and paint dispatch are
   inside the measured region: they cost **~10 ms beyond window creation**, not 50–60. Session 5's
   "the window is the bottleneck" reframing is **withdrawn**.
2. **G3's 40 ms first-pixel criterion passes with the two-stage paint** — 13.1 ms p50, 14.5 ms p90,
   14.5 ms worst of 11 — and fails ~1.7x without it (68.6 ms). **The criterion is a test of paint order,
   not of the graphics stack**, and `SPEC.md` §3.2's two-stage requirement is now measured as
   sufficient rather than merely helpful. Still one machine, not the G5 reference machine, and
   time-to-`FillRect` rather than time-to-photon.
3. **G4 went the other way and this is the important one.** Rasterisation on the quiet machine is
   **494–582 ms/frame — the *top* of the earlier 227–582 range**, i.e. **330–388 µs/glyph**. "The spread
   tracks load" was wrong, a cold run does not bound the cost favourably, and the pessimistic end is the
   honest figure. **This raises the value of the batched-rasterisation experiment below**, which is now
   unambiguously the highest-value remaining item.

**Two method notes worth keeping:**

- **Post-reboot is not quiet, and uptime is the wrong thing to record.** The first four runs of the
  first set gave 174, 225, 264 and **1604 ms** before settling — boot-time service churn is itself load.
  The quiet window opened at about six minutes' uptime. **Record CPU load, and re-check it after every
  set.**
- **`-OutFile` must match the filename the binary hardcodes** (`g3-d2d-first-pixel.txt`,
  `g3-d3d11-<mode>.txt`). Any other name and `measure.ps1` polls for a file nobody writes, warns eleven
  times and throws — a whole set measured nothing before this was spotted.
- **G4 is `windows_subsystem = "windows"`, so PowerShell does not wait for it.** `& .\g4-glyph-atlas.exe`
  returns in 0.1 s while the process runs on for ~100 s. Poll for `%TEMP%\g4-glyph-atlas.txt`, then kill
  the process — it holds a D3D device and is otherwise a leaked subject by the same mechanism as G3's.

---

## Directory rename — half done, finish it while Claude is closed

The working directory is being renamed `WinTail` → **`Tailhawk`** (lowercase h, matching the docs and
the `github.com/nigelbasel/tailhawk` remote). **The context-preserving half is already done:** the
project-state directory has been copied to the new key, so memory and session history survive.

`C:\Users\nigel\.claude\projects\C--dev-git-Tailhawk\` already holds all 3 memory files and all 3
session transcripts including session 5's, plus the `workflows/`, `subagents/` and `tool-results/`
subdirectories from sessions 1 and 2.

**What remains** — the rename itself, which cannot be done from inside a Claude session because the
harness resets the shell CWD into the repo after every tool call, and Windows will not rename a
directory a process has as its CWD. **Session 6 was still in `C--dev-git-WinTail`, so it is still
outstanding.** With Claude closed — a log-out is a natural moment for it:

```powershell
cd C:\dev\git
Rename-Item WinTail Tailhawk
Copy-Item -Recurse -Force "$env:USERPROFILE\.claude\projects\C--dev-git-WinTail\*" `
                          "$env:USERPROFILE\.claude\projects\C--dev-git-Tailhawk"
```

The third line re-syncs session 5's final exchanges, which post-date the copy. Then reopen in
`C:\dev\git\Tailhawk`. **Delete `C--dev-git-WinTail` only after confirming the new location works** — it
is deliberately kept as a backup. A longer script with checks and verification was written to the
session scratchpad but is not in the repo.

**If the spelling is ever changed again** — even just the capitalisation — the project key changes with
it and the memory is orphaned again, because the key is the literal path string. Decide once.

---

## Where things stand

The project is **Tailhawk** (command `tailhawk`) — a Windows desktop log tailer/viewer.
Research, specification, UI design and development plan are complete and adversarially reviewed.
**Phase 0 has started and the first experiment has produced results.**

Repo: **`github.com/nigelbasel/tailhawk`, private.** Working directly on `master` — no branches, no
PRs (tried once, not worth it solo). Commit often; the history is the artefact.

### The documents

| File | State |
|---|---|
| `docs/RESEARCH.md` | Complete. §11 records every claim critics refuted. §12 lists the gating experiments. **§5.3 is GPL-contaminated — see `CLEANROOM.md`.** |
| `docs/SPEC.md` | Complete. §16 traces both review rounds. §17 lists open decisions. |
| `docs/UI-DESIGN.md` | Complete. Phase-tagged `[v1]`/`[v2]`/`[v3]` throughout — **`SPEC.md` §15 is authoritative on phasing, not this document.** |
| `docs/PLAN.md` | Complete. v1 = 81.5 person-weeks — **the owner considers the estimates extremely conservative and does not treat them as a schedule.** Gates are things to answer, not a timeline. |
| `docs/LOKI.md` | Complete. Loki-as-a-source design, adversarially reviewed. Decisions settled; not yet folded into `SPEC.md`/`PLAN.md`. |
| `CLEANROOM.md` | Live. Provenance rule, source allow-list, component register, append-only log. |

### Repo furniture in place

`LICENSE-MIT`, `LICENSE-APACHE` (dual, copyright asserted personally), `cargo-deny.toml`
(allow-list, so GPL/AGPL/LGPL fail without being named — rationale in `CLEANROOM.md` §7),
`.gitignore`, `rust-toolchain.toml`, workspace `Cargo.toml`, tracked `Cargo.lock`.

### Decisions locked in

- **Name: Tailhawk.** Chosen for the "watch like a hawk" idiom. Tagline: *watch your logs like a hawk*.
- **Stack:** Rust + windows-rs + D3D11 + DirectWrite. Shared core owns the whole grid *including rendering*; thin shell owns window/input/IME. Per-platform leaf backends for rasterisation and presentation.
- **Windows-only v1.** Cross-platform stays possible via the leaf-backend seam but is unpromised.
- **No memory mapping**, ever — a section handle blocks the writer's log rotation.
- **Polling is the correctness mechanism** for following; change notification is an accelerator that may fail silently.
- **Record model is OTel-shaped**, extended with `raw`/`format_id`/`parse_state`.
- **Merged timeline + trace correlation stay v2** (owner-confirmed).
- **Personal project**, personal GitHub, no employer identity anywhere in the repo.
- **Licence: MIT OR Apache-2.0.**
- **Loki is a client, not a viewer** (2026-07-29). The near-free `logcli | tailhawk -` path is
  rejected — depending on another CLI binary contradicts the self-contained copy-and-run promise.
  Staged client-lite → full client. Cost is not a constraint on this project.
- **v2 order:** §8.3 merged view (local only) → Loki client-lite → §9 trace correlation → Loki
  stages 2–3. The merged view goes first because it is entirely offline and keeps §13.2's
  "no sockets, ever" CI assertion at full strength until the last possible moment — a one-way door.

---

## What is worth doing next, as of session 5

Phase 0 is **4 of 7 done or dispositioned**. What remains, and an honest read on each:

0. ~~**Batched glyph rasterisation — the highest-value unblocked experiment**~~ — **done, session 7.**
   See the section at the top and `experiments/g4b-batched-raster/RESULTS.md`. Batching is
   bit-identical and worth ~1.8x saturating at batch 4; the order of magnitude is not there. The
   premise — that per-analysis granularity was the dominant cost — is refuted: a cross-process font
   cache is, and G4's figure was a thrash figure. **Nothing further is owed on this line of work.**
   What it leaves behind is one decision for the owner: whether to re-scope `SPEC.md` §3.2 from a
   permanent requirement to a first-run one.
1. **G2 — read throughput.** Unblocked and doable solo, but **it blocks nothing**: `PLAN.md` §3 marks it
   *"Informational — no pass threshold"* and its "if it fails" column reads *"Nothing."* The no-mmap
   decision is already locked on correctness grounds, so G2 only quantifies what that costs. Note it
   wants **10 GB of scratch disk**, and its cold half needs a reboot or an elevated standby-list purge —
   so it is realistically warm-only unless run right after a restart.
2. **G3's `eframe` legs — argue for skipping them.** G3 exists to *compare* three stacks, but the
   stack decision is already locked in (windows-rs + D3D11 + DirectWrite), `RESEARCH.md` §3.3 already
   rejects egui/eframe for the grid on text-AA grounds, and **G7 has now independently confirmed
   egui's scroll model breaks at exactly the row counts Tailhawk targets.** Building two eframe
   hello-worlds would measure binary size for a stack that is triply rejected. The honest move is to
   record the legs as **deliberately not run**, with that reasoning, rather than leave them looking
   outstanding. Owner's call.
3. ~~**Re-take G3's numbers** once the desktop C++ workload is installed~~ — **fully done.** Sizes
   (unchanged, byte-identical), the A/B comparisons, and now the **cold and quiet sets** (session 6,
   above). Nothing further is owed on this machine; the remaining absolute-figure work is G5's reference
   machine, which is open question 3.
4. ~~**Test the one surviving first-paint direction:** paint something cheap before the D3D device
   exists~~ — **done and it works.** It is `earlypaint` in `experiments/g3-d3d11`, it reaches first pixel
   in **13.1 ms p50** on a quiet machine, and it is what makes G3's 40 ms criterion pass. `SPEC.md` §3.2
   requires it for v1.
5. **G1** needs two hosts, **G5** needs a decision on downloading third-party binaries, **G6** is a
   week of the owner's real usage. All three are owner-gated, not work-gated.

~~**A free by-product worth collecting:** adding G3-style first-pixel instrumentation to the D3D11 +
DXGI path~~ — **done**, as `experiments/g3-d3d11`, and it paid for itself twice: it refuted the
worker-thread fix and showed the specified stack is ~30 ms faster than the D2D one G3 measured.

## Resume here tomorrow

### 1. ~~Finish the Loki source design~~ — **done 2026-07-29, see `docs/LOKI.md`**

The workflow was re-run and completed (4 agents, ~30 min). Results are written up in
**`docs/LOKI.md`** and are **not** yet folded into `SPEC.md` or `PLAN.md`, deliberately.

Headlines:
- **The architectural crux resolved cleanly.** A Loki source materialises pages into the §4.2 stdin
  spill as CLEF NDJSON; the grid, index, filters, search, sort and export run unchanged. The
  viewport does **not** become cursor-based.
- **One thing must land in v1** even though no Loki code ships in v1: `ExtentState` and an opaque
  `RowId(u64)` (§4 of `LOKI.md`). ~0.5 PW now against a ~3 PW rewrite after the grid ships.
- **Three research claims died under re-verification** — the binary-size objection to an HTTP stack
  (measured at +1.68 MB against a 15 MB gate), the choice of `rustls` (pulls a C toolchain via
  `ring`, contradicting §5.3), and the headline latency figure (an artifact of a pathological
  selector; a tight selector is 60–130x faster).
- **The credential design as researched is unsound** — keying by source name is an exfiltration
  primitive, and "DPAPI or Credential Manager" is not a design.
- **Decided 2026-07-29: Tailhawk is a Loki client, staged client-lite first.** The near-free
  `logcli … | tailhawk -` viewer path is **rejected** — a dependency on another CLI binary
  contradicts the self-contained copy-and-run promise. Cost is not a constraint on this project, so
  the "86–110% of the v2 budget" objection does not bind. The cost reconciliation is still owed to
  `PLAN.md` §2.3b, but it no longer gates proceeding. See `LOKI.md` §8 for the three stages.
- **Order within v2:** §8.3 merged view (local only) → Loki client-lite → §9 trace correlation →
  Loki stages 2–3. The merged view goes first because it is entirely offline and keeps §13.2's
  "no sockets, ever" CI assertion at full strength until the last possible moment — that weakening
  is a one-way door. Reasoning in `LOKI.md` §8.

### 2. Then, in order

**Owner direction, 2026-07-29: get the whole app running locally, put it in a GitHub repo, publish
later.** A **private** GitHub repo is not publishing, so it is compatible with everything below.
Everything genuinely publication-shaped is deferred, and the queue is reordered around that.

Three things follow from pushing to GitHub at all, private or not:

- **Git history is permanent.** Anything committed to a private repo is still in the history when
  that repo later goes public. The scrubbing rules — no employer identity, no customer names, no
  dogfood paths or sample lines (see below) — apply from the **first commit**, not from the publish
  date. There is no later cleanup that is not a history rewrite.
- **Branching starts.** The standing "commit straight to `master`" rule was explicitly scoped to
  "until there is a remote". Once the GitHub remote exists, branch-per-change and PRs apply.
- **Open question 2 (employment IP) gets closer.** It is easier to establish a position before code
  exists in a hosted repo than after. Still the owner's to resolve.

1. ~~**Populate `CLEANROOM.md`**~~ — **done 2026-07-29.** `CLEANROOM.md` is at the repo root. It
   records the rule, the allow-list, the component register and an append-only consultation log.
   The block-sparse line-offset index is registered **CONTAMINATED and not cleared to implement** —
   it needs a re-derivation entry plus an attestation in `CLEANROOM.md` §6 **before its first line
   of code**.
2. ~~**Add `LICENSE-MIT`, `LICENSE-APACHE`, `cargo-deny.toml`**~~ — **done 2026-07-29.** All three
   are at the repo root. `cargo-deny.toml` is an allow-list, so GPL/AGPL/LGPL fail without being
   named and an unreviewed licence fails closed; the reasoning is in `CLEANROOM.md` §7. (`git init`,
   the first commit and the push to `github.com/nigelbasel/tailhawk` — private — are also done.)
3. **Run Phase 0** (`PLAN.md` §3) — **in progress.** See the next section.

**Working agreement:** commit directly to `master`, no branches, no PRs — tried once, not worth it
for a solo repo. Commit often; the history is the artefact.

---

## Toolchain — resolved, 2026-07-30

**The desktop C++ workload is installed and `.cargo/config.toml` is deleted.** The whole OneCore
workaround is gone: toolset `14.51.36231` now has `lib\x64`, `lib\x86` and `include`, the version did
not bump, and `cargo build --release` links with no `LIB` override. If a future `LNK1104` appears,
that file no longer exists to be the suspect.

**Two things learned during the transition, worth keeping:**

- **Do not build or measure while a Visual Studio installer is resident.** A `+crt-static` link failed
  once with `LNK1104: cannot open file 'libucrt.lib'` for two crates while succeeding for the other
  two, then succeeded for all four on an immediate retry — files moving underneath the linker. Every
  session-5 timing was taken with `setup.exe` resident, which is why they are provisional.
- **⚠ A measurement subject that exits on its own is never reaped, and it corrupts the numbers.** This
  is the single most expensive thing session 5 learned. The dead process keeps its D3D device;
  `tasklist` lists it, `taskkill` says *"no running instance"*, `cargo build` fails with
  `Access is denied (os error 5)`, and — the part that actually costs you —
  **`D3D11CreateDevice` degraded from 55 ms to over 1200 ms as 49 of them piled up**, silently, with no
  error. It invalidated a whole round of G3 conclusions before it was spotted.
  **The fix is in the code: experiment binaries must not `PostQuitMessage` after reporting.** They
  report and wait to be killed by `measure.ps1`. `g3-d2d` did this by accident and leaked zero across
  22 runs; `g3-d3d11` self-terminated and leaked one per run until it was changed.
  `$p.Dispose()` in PowerShell does **not** help. If a measurement looks inexplicably slow, count the
  leaked processes before believing it. If some are already resident, they can only be cleared by
  killing the holder or rebooting — and meanwhile build into a scratch dir
  (`$env:CARGO_TARGET_DIR="target-verify"`, git-ignored) or verify with `cargo check --workspace`.

### Re-take procedure

Still owed: a **post-reboot (cold)** set and a **quiet-machine (warm)** set, because of the
reproducibility problem below. Sizes are done and did not move.

```powershell
cargo build --release            # or $env:RUSTFLAGS="-C target-feature=+crt-static"
.\experiments\measure.ps1 -Exe target\release\g3-d2d.exe   -OutFile g3-d2d-first-pixel.txt `
    -Columns "factory,window,target,draw,total"
.\experiments\measure.ps1 -Exe target\release\g3-d3d11.exe -OutFile g3-d3d11-serial.txt `
    -ExeArgs serial     -Columns "mode,window,device,swapchain,draw,total,driver"
.\experiments\measure.ps1 -Exe target\release\g3-d3d11.exe -OutFile g3-d3d11-concurrent.txt `
    -ExeArgs concurrent -Columns "mode,window,device_wait,swapchain,draw,total,driver"
```

**Quote `-Columns`** — PowerShell parses a bare comma-separated list as an array and the script
rejects it. Then re-run G4 (`cargo run --release -p g4-glyph-atlas`, ~100 s) and drop its OneCore
caveat.

---

## Phase 0 — where it got to

| Gate | State |
|---|---|
| **G3** — binary size floor + first pixel | **Size passes and is settled. First pixel now passes too — with the two-stage paint (13.1 ms p50 against 40 ms); it fails ~1.7x without it.** Absolutes settled as far as one machine allows; publishing them still waits on G5. Two legs done on the desktop CRT: D2D (`experiments/g3-d2d/RESULTS.md`) and D3D11+DXGI (`experiments/g3-d3d11/RESULTS.md`). `eframe` legs not started — see the argument below that they are moot. |
| **G1** — SMB stale size | Not started. Needs two hosts and a share; can't be done solo on one machine. |
| **G2** — read throughput | Not started. Informational only, no pass threshold. |
| **G4** — colour-glyph atlas | **Done. Passes**, and it refuted the objection it was built to test. See `experiments/g4-glyph-atlas/RESULTS.md`. Its rasterisation *absolutes* are superseded by G4b (session 7) as cache-thrash figures; its atlas and eviction conclusions stand. |
| **G4b** — batched rasterisation | **Done, session 7.** Not a `PLAN.md` gate — a follow-up closing G4's open caveat. Batching is bit-identical and worth ~1.8x; the dominant cost is a cross-process font cache. See `experiments/g4b-batched-raster/RESULTS.md`. |
| **G5** — incumbent re-measurement | Not started. Needs BareTail 3.50a and LogExpert 1.41.0 installed — **owner decision, involves downloading third-party binaries.** |
| **G6** — Hoo WinTail hands-on | Owner task, runs in the background across Phase 0. |
| **G7** — reproduce egui #1391 | **Done. Passes** — the cause is identified. See `experiments/g7-egui-scroll/RESULTS.md`. |

### G3 result in one line

**Size passes by ~8x. First pixel passes by ~3x if something paints before the device exists, and fails
by ~1.7x if you wait for it — so the gate measures paint order, not the graphics stack.**

The paragraphs below were written before session 6's quiet set and quote the loaded absolutes (126 ms
total, 117 ms graphics init, a 50–60 ms window-presentation floor). **The ratios and the A/B conclusions
all stand; the absolute numbers are ~1.8x too high and the window-presentation floor is refuted.**
Corrected figures are in the session-6 section at the top and in `experiments/g3-d3d11/RESULTS.md`.

**Size is done.** 243,712 bytes with `+crt-static`, 146,432 dynamic, against a 2 MB criterion — and
**byte-for-byte identical between the OneCore and desktop CRTs**, so the re-take changed nothing. The
15 MB CI gate has vastly more headroom than assumed. Static costs a flat +97,280 bytes.

**First pixel fails, and the shape of the failure changed completely.** On the specified stack
(D3D11 + DXGI, `+crt-static`, desktop CRT) it is **126 ms** against a 40 ms criterion. The breakdown
is the finding: **drawing is 2.6 ms** and **graphics initialisation is 117 ms, or 92% of the total** —
`D3D11CreateDevice` 60 ms plus swapchain and RTV creation 57 ms. So the conclusion *"graphics device
creation must come off the critical path"* is **strengthened**.

**The fix G3 proposed works, but only just.** On a **clean machine**, 10 **paired interleaved** trials:
concurrent device creation totals **145.7 ms** against serial's **154.2 ms** and wins **7 of 10** — a
**~8.5 ms, 5.5% saving.** Real, reproducible, and small, exactly what `min(window, device)` allows when
window creation is only ~10–25 ms. Worth taking; nowhere near enough on its own.

Under **GPU-context pressure** it matters far more: with ~35–49 leaked D3D devices resident, serial
device creation degraded to a **1155 ms** median while concurrent held at **135 ms**, winning 8/8. The
likely mechanism — `D3D11CreateDevice` stalling on the `HWND`-owning thread when the driver wants it to
pump messages — is unconfirmed, but it argues for off-thread device creation as cheap insurance on
loaded machines, which is where a log viewer lives.

**The bigger lever is measured and it works: paint before the device exists.** A GDI fill in the first
`WM_PAINT`, with D3D coming up on a worker, cuts first pixel to **48–54% of the naive order** — 12 of 12
paired trials, reproduced across two runs. `SPEC.md` §3.2 now requires this two-stage paint for v1.

Two results worth not re-litigating:
- **A class background brush is equivalent, not better** (4 of 12 pairs). The mechanism does not
  matter; only that *something* paints without waiting for D3D.
- **The residual ~50–60 ms floor is window presentation, not graphics.** Window creation is ~7 ms, yet
  first pixel lands at 55–70 ms even when the system does the fill during `ShowWindow` with no handler
  running. `ShowWindow` + DWM composition + first-paint dispatch is the cost. **G3 was built to ask
  whether the graphics stack could paint fast enough; once you stop waiting for it, the window is the
  bottleneck.** Further first-paint work belongs outside the renderer.

**⚠ Two earlier conclusions here were wrong.** This file previously said concurrency was *refuted* at
11% slower. That was an artifact of always measuring serial first while leaked D3D devices accumulated
between the arms. The opposite over-correction (8/8, ~5x) was real only for the heavily-contaminated
regime. **Rule that came out of it: never compare two configurations in a fixed order** when anything
can accumulate between them.

**Two of session 3's conclusions are withdrawn:**

- **The "~113 ms `CreateWindowExW` floor" does not exist.** The byte-identical binary on the same
  machine gives **9.3 ms** (7.3 – 12.5), a 13x discrepancy with non-overlapping ranges. The cause is
  not established — installer load is the leading hypothesis but session 5 also had an installer
  resident, which argues against it.
- **Using the spec's own stack is worth ~30 ms**, unprompted: D3D11 + DXGI reaches first pixel in
  126 ms where D2D's `HwndRenderTarget` takes 157 ms. The D2D leg was measuring a configuration
  Tailhawk was never going to ship.

~~**Still owed:** a post-reboot (cold) set and a quiet-machine set.~~ **Both taken, session 6.** Every
session-5 timing was taken with a VS installer resident, which is why they read ~1.8x high. Variance
still means any first-paint budget must be a percentile, not a mean — but **40 ms does not need
re-deriving**: it is met at 13.1 ms p50 / 14.5 ms p90 once something paints before the device exists.

### G4 result in one line

**The one-instanced-draw rule survives colour emoji.** `PLAN.md` §3 asserted that a premultiplied
colour atlas *"cannot share the mono atlas's blend state"*; it can, via **dual-source blending** —
`SV_Target0` premultiplied colour, `SV_Target1` per-channel coverage, `dest = src + dest*(1-cov)`,
which is simultaneously the correct ClearType blend and the correct premultiplied composite. Confirmed
by reading back the frame with a forced-neutral tint, where channel spread can only come from
per-channel coverage: **ClearType and colour glyphs both present in one draw**. No V2 re-costing owed.
Unified is also 25–32% cheaper on CPU than a two-pass split, and the split cannot do ClearType at all.

**Two findings the gate did not set out to make, both now in `SPEC.md` §3.2:**

- **Eviction passes only with an O(1) policy.** Scanning slots for the oldest costs 4–8 ms/frame under
  thrashing; an intrusive LRU list costs 0.17–0.37 ms. An earlier variable-width-span variant cost
  **106 ms/frame**. Uniform single-glyph slots are what make O(1) possible.
- **⚠ Superseded by session 7 — these are cache-miss figures, and the mechanism is a system font
  cache, not analysis granularity.** Cold is 86–108 µs/glyph and warm is ~3; see the top section. The
  eviction conclusion below is unaffected. Original text follows.
- **Rasterisation is the real stall: 145–210 µs per glyph, and 330–388 µs on a quiet machine.** A cold
  viewport of ~1,500 CJK glyphs needs 220–580 ms — 13–35 frames. So glyph rasterisation must be **off the
  paint path**, with placeholders filled in over later frames. That is now a v1 requirement and
  `SPEC.md` did not previously say it. Eviction is three orders of magnitude cheaper than the
  rasterisation it triggers. **Session 6 note: the quiet figure is the *slow* end, so this cost is not a
  load artefact** — and two phases doing identical work in one quiet process differed by ~18%, so the
  fixed-order rule applies inside G4's binary too.

### G7 result in one line

**The cause is `ScrollArea::State::offset` — an f32 holding an absolute content-pixel coordinate.**
The thumb mapping is exonerated (forward f32 error measures exactly zero) and row-height accumulation
is not a cause because egui never accumulates.

The finding worth having run the gate for: it breaks in **two independent ways, and fixing one leaves
the other**. Delta accumulation (`offset -= delta` discards a 2 px drag at 4M rows) was anticipated.
Row *layout* — `(inner_top - offset) + min_row as f32 * row_h`, two content-magnitude f32s differenced
to give a sub-row result — was not, and it **survives an exact scroll position**. So `SPEC.md` §6.4's
`u64` mandate is necessary and not sufficient; a plausible-looking `row * row_h - offset_px` in the
layout path reproduces the entire bug inside an otherwise correct grid. §6.4 now carries all three
derived rules and a CI assertion that catches them.

Both of the reporter's thresholds — 2M for onset, 100M for "very broken" — are **predicted from the
arithmetic alone**, by a model never fitted to them. `RESEARCH.md` §3.4 is now `[V]`.

### Reopened and then closed properly: the reboot happened, and it did settle something

**Superseded by session 6 — the owner did reboot, and the set was worth taking.** Session 5 wrote this
section off on the grounds that absolute figures were blocked on G5's reference machine rather than on a
reboot. That was half right: G5 is still required before any `SPEC.md` §11.3 figure is *published*, but
the quiet set refuted a design conclusion (the window-presentation floor) and flipped a gate verdict
(G3's first pixel). Neither of those needed a reference machine — they needed the load removed once.

The diagnosis below stands and explains the spread:

The cause of the instability is identified: **this machine carries a variable ~40% background load** from
a normal working set (Teams, Edge WebView2, Docker, OneDrive, Outlook), which is not going away. A 21-run
D2D set spanning it gave a p50 of 297 ms across a 117–783 ms range, with fast runs clustered wherever the
load happened to dip.

So the same static build has legitimately produced 96, 112, 126, 139, 154 and 297 ms — **and 75.5 ms
quiet, which is below all of them.** The lesson to carry: a quiet-machine set is cheap, and it is the
only way to tell a floor from a queue.

**What to do instead, and it is already the practice:** decide with **paired interleaved A/B** on this
machine — reliable, reproduced across runs, and immune to both load drift and accumulation effects.
Quote absolutes only as percentiles with the machine state stated, and never as a target.

The historical detail, retained because the discrepancy was large enough to mislead twice:

The G3 summary above records the 13x `CreateWindowExW` discrepancy. It is **still unexplained**, and it
is the reason no first-pixel figure has been promoted into `SPEC.md` §11.3.

| `CreateWindowExW`, same binary, same machine | median (range) |
|---|---|
| session 3, OneCore CRT | **113 ms** (21 – 144) |
| session 5, OneCore CRT, before the install | **8.5 ms** (6.7 – 11.4) |
| session 5, desktop CRT, `+crt-static` | **9.3 ms** (7.3 – 12.5) |
| session 5, desktop CRT, dynamic | **11.9 ms** (9.9 – 26.5) |

Three of four agree closely; session 3 is the outlier, and its range does not overlap the others. So
the CRT change is **not** the cause — the 8.5 ms measurement was taken before the install, on the same
OneCore toolchain that produced 113 ms.

**What settled it, session 6:** a post-reboot set and a quiet set (no VS installer, no Docker, no
browser) both give `CreateWindowExW` at 3.2–3.9 ms. With session 5's 8.5–11.9 ms that is three agreeing
sets against session 3's 113 ms. **Session 3 is the outlier, its cause is unexplained, and it is no
longer worth explaining.** The A/B comparisons remain the sound part of session 5 and are unaffected:
serial vs concurrent, D2D vs D3D11 + DXGI, static vs dynamic.

**Identity sweep: done 2026-07-29.** `RESEARCH.md`, `SPEC.md`, `PLAN.md`, `UI-DESIGN.md`, `LOKI.md`
and `HANDOFF.md` were swept for employer names, customer names, internal hostnames, service names,
email addresses, private IPs and local paths. **One hit** — a local profile path in this file's
Session artefacts section — now genericised. The surviving "employer" references are the policy
statements themselves (`SPEC.md` §2 and the rules below) and name nobody.

**Deferred until the app runs locally and going online is actually on the table:**

- **The code-signing route** (`PLAN.md` §7.1). Azure Artifact Signing is **ruled out** — its
  eligibility covers neither South African organizations nor individuals. Certum Open Source Code
  Signing eligibility for a South African individual is unconfirmed (from €69, hardware token
  shipped internationally). Not a blocker for v1 — ship unsigned via scoop; it blocks the first
  *signed* release only. **Note the linkage:** `LOKI.md` §7 argues signing becomes a prerequisite of
  the **network feature** specifically, because unsigned + portable + share-distributed + outbound
  TLS to file-specified hosts is a dropper signature that SmartScreen will never grant reputation
  to. That is a v2 concern, well after local.
- **`tailhawk.com` / `.dev` / `.io`** at a registrar. Never verified.

**Not deferred, because it decays:** claiming the **namespaces** — GitHub org `tailhawk`,
crates.io, scoop, winget. Verified free during naming, but nothing holds them. Losing `tailhawk`
after four documents and a repo are written around the name means a rename, not an inconvenience.
Cheap insurance, no publishing implied — an empty GitHub org and a reserved crate name are not a
release.

---

## Dogfooding — first runnable build

The owner has nominated **two real logs currently in daily use** as the first dogfood targets. The
build does not have to be good; it has to open these two files and follow them. Treat this as the
acceptance gate for "a first version that can actually run".

Actual paths are **deliberately not recorded in this repo** — they sit inside an employer source
tree and their content contains customer names. They are held in the private project memory
(the project memory directory, `~/.claude/projects/<project-key>/memory/`, where `<project-key>` is
the repo path with separators replaced by `-`). See the identity note below.

| | Corpus A | Corpus B |
|---|---|---|
| Size | ~450 KB | ~3.3 MB |
| State when sampled | idle — batch writer had exited | **live, actively appended** |
| Encoding | ASCII / UTF-8, no BOM | **UTF-8, no BOM, with non-ASCII** (U+2014 em dash) |
| Format | single, log4net-shaped | **heterogeneous — three formats in one file** |

**What each one actually tests:**

*Corpus A* — log4net layout, `yyyy-MM-dd HH:mm:ss,fff LEVEL␠␠Logger.Name message`. Note the
**comma** as the millisecond separator and the padded level field. Straight columnisation exercise
plus open-a-completed-file.

*Corpus B* is the valuable one, and it breaks several assumptions:

1. **BOM-less UTF-8 with non-ASCII content.** Windows PowerShell 5.1's `Get-Content` renders the em
   dashes as `â€”` — i.e. a CP1252 default gets this file wrong. This is a ready-made regression
   test for encoding detection; the correct answer is already known.
2. **Three formats in one file.** A banner line, then a **tab-delimited section with its own header
   row** (`TIME⇥INSTANCE⇥STAGE`) carrying **time-only timestamps** (`17:00:02`, no date), then later
   a different layout entirely (`yyyy-MM-dd HH:mm:ss␠␠[tag]␠␠message`). This confirms `format_id` and
   `parse_state` must be **per-record, not per-source** — already the model in `SPEC.md` §4, now with
   a real specimen behind it.
3. **Time-only timestamps.** `SPEC.md` has **no rule** for a record whose timestamp carries no date.
   Inherit the date from the last fully-dated record? Leave `timestamp` unset and rely on
   `observed_timestamp`? **Open — needs a decision.**
4. **Non-monotonic timestamps.** An `07:14:14` line appears *after* an `09:14:14` line. Nothing may
   assume within-source ordering. This matters most for the v2 merged timeline.

**What they do not test:** both are single-digit MB. Neither exercises the multi-GB path, the
block-sparse index, rotation, or UNC latency. A large file and a rolling set are still needed
separately.

**Identity flag — resolve before the first public commit.** Corpus A lives in an employer repo and
its log lines contain customer names; Corpus B names customer instances. This runs straight into the
locked-in "no employer identity anywhere in the repo" decision and into open question 2 below
(employment IP). Dogfooding against them privately is fine. **Never** paste sample lines, paths, or
screenshots from either into the repo, an issue, or a release note without scrubbing.

---

## Traps — do not rediscover these

Recorded because each cost real effort to find, and two of them were caught only by adversarial review.

| Trap | Where |
|---|---|
| **The index depends on encoding.** Decode *must* precede index — chunk boundaries need code-unit alignment, and `0x0A` is a legal DBCS trail byte. The first plan drafted them 13 weeks apart and would have thrown away tested work. | `PLAN.md` §4 |
| **`RESEARCH.md` §5.3 is GPL-contaminated.** The line index must be **re-derived from published docs only**, with `CLEANROOM.md` populated *before* writing code. Do not implement from that section. | `RESEARCH.md` §5.3 |
| **Serilog and NLog roll to *new filenames* by default.** Neither copy-truncate nor rename-and-recreate describes it. Rolling sets (§5.5b) are v1 for this reason. | `SPEC.md` §5.5b |
| **Every ripgrep throughput figure in circulation is fabricated.** The cited article publishes timings only. The search performance target is deliberately unset. | `RESEARCH.md` §11 |
| **"Beat BareTail's 2 MB" is unwinnable** and retired — D3D11+DXGI+DirectWrite costs 30–60 MB before reading a byte. The claim is *flatness*, not absolute size. | `SPEC.md` §11.2 |
| **`FILE_SHARE_DELETE` is mandatory.** Without it we block the writer's own rotation — a real, reported bug in klogg. | `SPEC.md` §5.1 |
| **Never poll file size by path** — NTFS only replicates size to the directory entry on last-handle-close, so a path stat on a live log returns a frozen size forever. | `SPEC.md` §5.4 |
| **Filtering is not O(viewport).** Every filter change is a full-file pass. Debounce, stream, and require Enter on network paths. | `SPEC.md` §7.3 |
| **Continuations collapsed by default**, and no `--wrap` in v1 — both destroy the O(1) `u64` scroll model, and MEL Simple logs are multi-line *by default*. | `SPEC.md` §6.4 |
| **Accessibility is the only automated UI-test surface** for a custom-drawn grid. The minimal chrome provider is v1 for that reason, not for compliance. | `SPEC.md` §14.1 |
| **A `u64` scroll position is not enough on its own.** G7 found the row-*layout* conversion `row * row_h - offset_px` reproduces egui #1391 in full even when the scroll position is exact. One plausible line, whole bug back. | `SPEC.md` §6.4, `experiments/g7-egui-scroll/RESULTS.md` |
| **`GetData` returns `S_FALSE` when a D3D11 query isn't ready — and `S_FALSE` is a *success* HRESULT.** `Result<()>` is `Ok`, so `is_err()` is useless as a readiness test. G4's first GPU timings were all 0.000 ms because of it. Detect readiness with a sentinel the driver must overwrite. | `experiments/g4-glyph-atlas/RESULTS.md` |
| **`DWRITE_GLYPH_RUN::fontFace` is `ManuallyDrop<Option<..>>`.** Writing into it is safe; copying it *out* into an owned interface releases a reference never added, and the underflow surfaces later as a use-after-free. Borrow the face out of a `DWRITE_COLOR_GLYPH_RUN`. | `experiments/g4-glyph-atlas/src/text.rs` |
| **`measure.ps1 -OutFile` must match the filename the subject hardcodes** — `g3-d2d-first-pixel.txt`, `g3-d3d11-<mode>.txt`. Any other name and the script polls for a file nobody writes, warns eleven times and throws, having measured nothing. | `experiments/measure.ps1` |
| **A `windows_subsystem = "windows"` exe does not block PowerShell.** `& .\g4-glyph-atlas.exe` returns in 0.1 s while the process runs on for ~100 s. Poll for its report file, then kill it — it holds a D3D device and is otherwise a leaked subject. | `experiments/g4-glyph-atlas` |
| **A quiet-machine set is how you tell a floor from a queue.** Session 5 derived a "~50–60 ms window-presentation floor" from loaded runs; the quiet re-take put first pixel at 13.1 ms. Conversely G4's rasterisation came in at the *top* of its range when quiet, so the load explanation was wrong in both directions. | `experiments/g3-d3d11/RESULTS.md` |
| **⚠ DirectWrite's glyph cache is cross-process, so "run it again" is a *warm* measurement.** The first version of G4b reported 2.5 µs/glyph and concluded G4 was wrong by 150x; it was reading cache hits left by its own previous run. Only the first process to touch a (glyph, size) pair since the cache lost it measures rasterisation. | `experiments/g4b-batched-raster/RESULTS.md` |
| **An in-process cache probe cannot see a cross-process cache.** Five identical passes gave pass0/pass4 = 0.99x — a confident, worthless "no cache". Vary **em** to get a cold population without a reboot: the cache key includes size. | `experiments/g4b-batched-raster/RESULTS.md` |
| **A reboot is not a neutral cold start for anything that draws text** — it empties the font cache service, so post-reboot text rendering is several times slower than a quiet warm machine. This is what session 6 misread as "a cold run does not bound the cost favourably". | `experiments/g4b-batched-raster/RESULTS.md` |
| **Derive an atlas cell from measured glyph bounds, never from em size.** Guessing 20×26 for em 14 clipped 1,086 of 1,500 glyphs — and the clipped cells then compared *equal* between arms, which reads as a correctness pass. Assert `overflow == 0` before believing any bitmap comparison. | `experiments/g4b-batched-raster/RESULTS.md` |
| **Never time a frame with `Present` inside the measured region.** The flip model blocks on the back-buffer queue, which pinned every G4 frame to exactly 16.669 ms and hid the real cost entirely. | `experiments/g4-glyph-atlas/RESULTS.md` |

---

## Still owed, not blocking

- **Fold `LOKI.md` into `SPEC.md` §4 and `PLAN.md` §2.3b.** The decisions are settled; the write-up
  is not yet integrated. The cost reconciliation between the two research rounds is owed to
  `PLAN.md` but gates nothing.
- **The highest-value Loki unknown:** is the Loki HTTP API reachable **directly** from a workstation,
  or only via Grafana's datasource proxy? Every measurement in `LOKI.md` went through the proxy. If
  proxy-only, that is a different URL shape, a different auth model and roughly +1 PW.
- **Namespace claims** — GitHub org, crates.io, scoop, winget. Deferred with the rest of the
  publication work, but unlike the rest **this one decays**: nothing holds `tailhawk` for us.

## Open questions the owner still needs to answer

1. **Certum eligibility** for a South African individual — needs a direct question to Certum.
2. **Employment IP position.** If any of this gets built on work equipment or work time, many contracts assign IP. Worth establishing **before the first public commit**, not after the repo has contributors.
3. **Reference perf machine** — must be fixed before any `[TBM]` target in `SPEC.md` §11.3 becomes a number.
4. **Hoo WinTail hands-on (G6)** — the owner's installed copy is the only reliable source for which of its features are actually used in a week, and for how its encoding detection really behaves.
5. ~~**Re-scope `SPEC.md` §3.2's rasterisation requirement?**~~ — **done, owner-approved, session 7.** §3.2 now states the cost as a first-run one (86–108 µs/glyph cold, ~3 µs warm, cache capacity 8,000–16,000 distinct glyphs), keeps placeholders as a v1 requirement, forbids deriving a §11.3 steady-state budget from the cold figure, and specifies batching at 4–64 glyphs per analysis. **The same edit also withdrew the refuted "~50–60 ms window-presentation floor"** from the first-paint bullet, which session 6 disproved but which was still stated as fact in the spec.

---

## Session artefacts

Workflow transcripts and scripts, if any reasoning needs re-checking:

```
%USERPROFILE%\.claude\projects\<project-key>\<session-id>\
  workflows\scripts\      ← re-runnable workflow scripts
  subagents\workflows\    ← per-agent transcripts and journal.jsonl
```

Session 1 was `c8d47c59-98d0-40f0-86b8-bcd47f6558a3`; session 2 (the Loki run) was
`c2757c25-1785-4b18-9f85-7283f401aaf1`. These are machine-local and are not part of the repo.

Seven workflows ran this session: competitor/tech/format research, cross-platform, the agentic-native-UI
thesis, naming (two rounds), OpenTelemetry, the four-artifact adversarial review, and the stopped Loki run.
