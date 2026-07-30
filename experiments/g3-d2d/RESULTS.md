# G3 — binary size floor and first-pixel latency: windows-rs + Direct2D

First subject of `PLAN.md` §3 G3. **Partial: the `eframe+glow` and `eframe+wgpu` subjects are not
built.** Both CRT configurations *are* covered — see the size table below. (An earlier revision of
this line said `+crt-static` was outstanding; it was applied in the same session and the line was
left stale.)

Measured 2026-07-29.

## Method

Minimal Win32 window, `D2D1CreateFactory` → `CreateHwndRenderTarget` → `Clear` → `EndDraw`, no text,
no content. `QueryPerformanceCounter` sampled at five points, first paint only. Timing starts at
`main()` entry, so **process creation and loader time are excluded — the real figures are worse than
these**.

Release profile as G3 specifies: `opt-level="z"`, `lto="fat"`, `panic="abort"`, `strip=true`,
`codegen-units=1`. Seven cold process starts, ~250 ms apart.

## Binary size — re-taken on the desktop CRT, byte-identical

| Build | Bytes | vs 2 MB criterion |
|---|---|---|
| Dynamic CRT | 146,432 (0.14 MB) | **passes, ~14x** |
| `+crt-static` (as G3 specifies) | 243,712 (0.23 MB) | **passes, ~8x** |

**Both figures are byte-for-byte identical between the OneCore CRT and the desktop CRT** (re-measured
2026-07-30 after the desktop C++ workload was installed and the `LIB` override deleted). The standing
warning that "every G3 number must be re-taken" was right to exist, but for **size** the answer is
that nothing moved. Static costs **+97,280 bytes**, and that delta held to within 512 bytes across all
four experiment binaries.

Static CRT costs **+97,280 bytes (+66%)** and is the configuration G3 asks for, since a
copy-and-run single exe should not depend on a redistributable being present.

Worth carrying forward: with a 0.23 MB floor, the 15 MB CI gate has far more headroom than assumed.
This independently reinforces `docs/LOKI.md` §5 — the claim that an HTTP stack threatened the gate
was already refuted at +1.68 MB measured, and against this floor it is not close.

## First pixel — and a reproducibility problem

Seven cold process starts per configuration. Medians, with range. **The session-3 column is retained
because it did not reproduce, which is itself the finding.**

| Phase | Session 3, OneCore CRT | Session 5, desktop CRT, `+crt-static` |
|---|---|---|
| `D2D1CreateFactory` | 0.18 ms (0.13 – 0.65) | 0.11 ms (0.08 – 0.14) |
| `CreateWindowExW` | **73 – 113 ms (21 – 144)** | **9.27 ms (7.3 – 12.5)** |
| `CreateHwndRenderTarget` | 171 ms (156 – 216) | 142.55 ms (126 – 163) |
| `Clear` + `EndDraw` | 6.0 ms (5.5 – 6.2) | 4.59 ms (3.7 – 5.8) |
| **Total** | **249 ms (194 – 303)** | **156.65 ms (140 – 178)** |

**Against G3's 40 ms criterion: still FAIL, by ~4x** (was ~6x). But the shape of the failure changed:

- **`CreateWindowExW` is not expensive.** Session 3 measured 73–113 ms with a 21–144 ms range and this
  document previously concluded *"A floor of ~113 ms for an empty window on a current machine is the
  number to build the budget around."* Re-running the **byte-identical binary on the same machine**
  gives **9.27 ms**, with non-overlapping ranges — a 13x discrepancy. **That floor does not exist**,
  and the trap note about window creation being unexpectedly expensive is withdrawn.
- **Render-target creation is now ~91% of the total**, not ~64%. So the central conclusion —
  *graphics device creation must come off the critical path* — is **strengthened**, not weakened.
- **The cause of the discrepancy is not established.** The leading hypothesis was installer load, since
  Visual Studio was mid-update when session 3 ended; but session 5's numbers were taken with a VS
  installer *also* resident, which argues against it. `docs/HANDOFF.md` carries the open question and
  the re-take procedure requires a post-reboot set before any of this becomes a `SPEC.md` §11.3 target.

Static vs dynamic on startup: the session-5 dynamic-CRT run gave 147.12 ms (136 – 176) against
156.65 ms static, i.e. the ordering flipped versus session 3 and both are inside each other's noise.
The original advice stands unchanged: **treat the two as equal on startup and choose `+crt-static` on
deployment grounds, not performance.**

## What the breakdown actually says

Drawing is **6 ms**. Everything else is initialisation:

1. **`CreateHwndRenderTarget` (176 ms) is the dominant cost.** This is where D2D creates the
   underlying D3D device and the driver initialises. It is per-process and does not warm up across
   runs.
2. **`CreateWindowExW` (113 ms) is unexpectedly expensive and highly variable (21–144 ms).**
   DWM composition setup and first-touch theme/GDI loading are the likely contributors. This was not
   anticipated anywhere in `SPEC.md` or `PLAN.md`.
3. **Variance is wide enough that any budget must be stated as a percentile, not a mean.** A 109 ms
   spread on an empty window means a mean-based target would be met about half the time.

## Consequence for the architecture

The naive sequential order — create window, then create device, then paint — cannot meet an
aggressive first-paint budget on this hardware. Two directions were offered here; **the first has now
been measured and refuted**, in `experiments/g3-d3d11/RESULTS.md`:

- ~~**Take device creation off the critical path**~~ via a worker thread. **Tested and it does not
  work.** Concurrent device creation came out **11% slower** than serial (140 ms vs 126 ms), and the
  wait for the worker's device after the window existed was 58.6 ms against a serial device cost of
  60.0 ms — a saving of 1.4 ms. Window creation and D3D11 device creation **contend** rather than
  overlap, and with window creation down at ~9 ms the theoretical ceiling was only ~9 ms anyway. You
  cannot hide more than `min(window, device)`.
- **Paint something cheap immediately.** Fill the window on `WM_ERASEBKGND` and swap in the real
  renderer when ready. **This is now the only surviving direction.** First *pixel* then approaches
  window creation (~9 ms) plus a GDI fill, because nothing in the D3D path is on the critical path at
  all — a far better prospect than when this was written against a supposed 113 ms window floor. Still
  untested, and it costs a visible two-stage paint.

Switching to the specified stack is worth ~30 ms on its own: D3D11 + DXGI reaches first pixel in
126 ms against this leg's 157 ms.

Neither gets to 40 ms. **The honest reading is that G3's 40 ms criterion was set without measurement
and should be re-derived** — `PLAN.md` §3 anticipates exactly this: *"the 15 MB CI gate and SPEC §11.3's
first-paint budget are both re-derived from measurement."* The number to build the budget around is
**~117 ms of graphics initialisation** (D3D11 device + swapchain + RTV), which is 92% of the total and
is not addressable by threading. The window itself costs ~9 ms.

## Caveats — read before quoting these numbers

- ~~Linked against the OneCore CRT~~ — **resolved 2026-07-30.** The desktop C++ workload is installed,
  the `LIB` override is deleted, and the session-5 column above is a desktop-CRT measurement. Sizes
  were unaffected; timings were, though not in a way attributable to the CRT.
- **A Visual Studio installer was resident during the session-5 measurements**, and it demonstrably
  interfered with the build (a `+crt-static` link failed once with `LNK1104: libucrt.lib` and then
  succeeded on retry). Treat the absolute figures as provisional.
- **No cold-boot separation, and the 13x `CreateWindowExW` discrepancy above is unexplained.** A
  post-reboot set and a quiet-machine set are both still owed before any figure here becomes a target.
- Single machine, single GPU. `PLAN.md` §3 G5 requires a fixed reference machine before any of this
  becomes a published target.
- Timing excludes process creation, so treat every figure as a lower bound.
- The `eframe+glow` and `eframe+wgpu` subjects are still outstanding. See `docs/HANDOFF.md` for the
  argument that they are now moot: the stack decision is locked, `RESEARCH.md` §3.3 rejects egui for
  the grid, and G7 independently confirmed its scroll model breaks at Tailhawk's target row counts.
  The *comparison* G3 was designed to make has instead been made between D2D and D3D11 + DXGI, which
  is the decision that actually mattered.

## Reproducing

No override is needed any more — the desktop C++ workload is installed and `.cargo/config.toml` has
been deleted.

```powershell
cargo build --release            # or with $env:RUSTFLAGS="-C target-feature=+crt-static"
.\experiments\measure.ps1 -Exe target\release\g3-d2d.exe -OutFile g3-d2d-first-pixel.txt `
    -Columns "factory,window,target,draw,total"
```

Each process start writes one CSV line — `factory,window,target,draw,total` in milliseconds — to
`%TEMP%\g3-d2d-first-pixel.txt`. The harness aggregates medians and ranges over 7 cold starts.
