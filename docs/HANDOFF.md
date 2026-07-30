# Handoff — resume here

**Paused:** 2026-07-30, session 5.
**Everything below is on disk and pushed. Nothing is held in a chat session.**

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

1. **G2 — read throughput.** The only remaining gate that is both unblocked and doable solo. Purely
   informational (no pass threshold) and it measures the cost of the already-made no-mmap decision so
   the trade can be stated honestly. Cheap.
2. **G3's `eframe` legs — argue for skipping them.** G3 exists to *compare* three stacks, but the
   stack decision is already locked in (windows-rs + D3D11 + DirectWrite), `RESEARCH.md` §3.3 already
   rejects egui/eframe for the grid on text-AA grounds, and **G7 has now independently confirmed
   egui's scroll model breaks at exactly the row counts Tailhawk targets.** Building two eframe
   hello-worlds would measure binary size for a stack that is triply rejected. The honest move is to
   record the legs as **deliberately not run**, with that reasoning, rather than leave them looking
   outstanding. Owner's call.
3. ~~**Re-take G3's numbers** once the desktop C++ workload is installed~~ — **done for sizes
   (unchanged, byte-identical) and for the A/B comparisons.** What remains is a **cold (post-reboot)
   and a quiet-machine set** to pin down the absolute first-pixel value; see the open item below.
   Owner-gated, because a reboot is theirs to schedule.
4. **Test the one surviving first-paint direction:** paint something cheap — a GDI or
   `WM_ERASEBKGND` fill — before the D3D device exists, then swap in the real renderer. Concurrency has
   been measured and refuted, so this is the only lever left, and it now looks much better than it did:
   first pixel would approach ~9 ms of window creation plus a fill, not the ~113 ms floor session 3
   believed in. Unblocked and doable solo.
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
| **G3** — binary size floor + first pixel | **Size passes and is settled. First pixel fails and its absolute value is still unsettled.** Two legs done on the desktop CRT: D2D (`experiments/g3-d2d/RESULTS.md`) and D3D11+DXGI (`experiments/g3-d3d11/RESULTS.md`). `eframe` legs not started — see the argument below that they are moot. |
| **G1** — SMB stale size | Not started. Needs two hosts and a share; can't be done solo on one machine. |
| **G2** — read throughput | Not started. Informational only, no pass threshold. |
| **G4** — colour-glyph atlas | **Done. Passes**, and it refuted the objection it was built to test. See `experiments/g4-glyph-atlas/RESULTS.md`. |
| **G5** — incumbent re-measurement | Not started. Needs BareTail 3.50a and LogExpert 1.41.0 installed — **owner decision, involves downloading third-party binaries.** |
| **G6** — Hoo WinTail hands-on | Owner task, runs in the background across Phase 0. |
| **G7** — reproduce egui #1391 | **Done. Passes** — the cause is identified. See `experiments/g7-egui-scroll/RESULTS.md`. |

### G3 result in one line

**Size passes by ~8x and is settled. First pixel fails by ~3x and its absolute value is not settled.**

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

**Still owed:** a post-reboot (cold) set and a quiet-machine set. Every session-5 timing was taken with
a VS installer resident. Variance alone still means any first-paint budget must be a percentile, not a
mean, and 40 ms needs re-deriving around ~117 ms of unavoidable graphics init.

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
- **Rasterisation is the real stall: 145–210 µs per glyph.** A cold viewport of ~1,500 CJK glyphs needs
  220–310 ms — 13–19 frames. So glyph rasterisation must be **off the paint path**, with placeholders
  filled in over later frames. That is now a v1 requirement and `SPEC.md` did not previously say it.
  Eviction is three orders of magnitude cheaper than the rasterisation it triggers.

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

### Closed: absolute first-pixel figures are blocked on G5, not on a reboot

**Owner will not be rebooting, so this route is closed — and it turns out not to matter.** The cause of
the instability is identified: **this machine carries a variable ~40% background load** from a normal
working set (Teams, Edge WebView2, Docker, OneDrive, Outlook), which is not going away. A 21-run D2D set
spanning it gave a p50 of 297 ms across a 117–783 ms range, with fast runs clustered wherever the load
happened to dip.

So the same static build has legitimately produced 96, 112, 126, 139, 154 and 297 ms. **Absolute
first-paint numbers were never blocked on a reboot — they are blocked on `PLAN.md` §3 G5's fixed
reference machine**, which open question 3 already required before any `SPEC.md` §11.3 figure is
published. Nothing further is owed here.

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

**What would settle it:** a post-reboot set and a genuinely quiet-machine set (no VS installer, no
Docker, no browser). Both are owner-gated, since a reboot is theirs to schedule. Until then treat every
absolute first-pixel number as provisional and rely only on the A/B comparisons, which were each taken
minutes apart under identical load and are therefore sound: serial vs concurrent, D2D vs D3D11 + DXGI,
static vs dynamic.

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
