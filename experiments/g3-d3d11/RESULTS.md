# G3, second leg — first pixel on D3D11 + DXGI

`PLAN.md` §3 G3, measured on **the stack `SPEC.md` §3 actually specifies** rather than the D2D
`HwndRenderTarget` of `experiments/g3-d2d`. It also tests the fix that leg proposed but never tried.

Measured 2026-07-30, **desktop MSVC CRT** (the shipping configuration — the OneCore workaround is
gone), `+crt-static`, 7 cold process starts per configuration via `experiments/measure.ps1`.

## Headline: the fix G3 proposed works, but only just — and measuring it exposed a trap

`experiments/g3-d2d/RESULTS.md` concluded that graphics device creation must come off the critical
path, and offered *"a D3D11 device can be created on a worker thread concurrently with window
creation."* Measured on a **clean machine** (no installer, no leaked processes), 10 **paired,
interleaved** trials — the two arms alternate which goes first, so neither systematically benefits:

| | serial | concurrent |
|---|---|---|
| `D3D11CreateDevice` / wait for it | 74.9 ms | 66.8 ms |
| **total to first pixel** | **154.2 ms** | **145.7 ms** |
| paired wins | 3 / 10 | **7 / 10** |

**Concurrent saves ~8.5 ms, about 5.5%.** Real, reproducible, and small — exactly what the ceiling
predicts, because you cannot hide more than `min(window, device)` and window creation is only
~10–25 ms here. It is worth taking, since the cost is one thread and a channel, but it is nowhere near
enough to reach the 40 ms criterion on its own.

Under **GPU-context pressure** the picture changes sharply. With ~35–49 leaked D3D devices resident
(see the trap below), serial device creation degraded to a median of **1155 ms** while concurrent
stayed at **135 ms**, and concurrent won **8 of 8** paired trials. The plausible mechanism is that
`D3D11CreateDevice` on the thread that owns the `HWND` can stall badly when the driver wants that
thread to pump messages and it is blocked inside the call; a worker thread has no such obligation.
**Not confirmed**, and only observed in an abnormal regime — but it argues for off-thread device
creation as cheap insurance on loaded or contended machines, which is exactly where a log viewer runs.

### ⚠ Two earlier conclusions on this page were wrong, and the reason is worth recording

The first version of this experiment measured **serial then concurrent, in that order, always**. It
reported *"concurrent is 11% slower — the fix does not work."* That was an artifact: each subject that
exited on its own was never reaped, leaving a dead process still holding its D3D device, so the second
arm measured always ran against more accumulated contexts than the first. Device creation drifted from
55 ms to over 1200 ms as they piled up.

Correcting it in the other direction was also wrong. An interleaved run taken while 35–49 zombies were
resident showed concurrent winning 8/8 by ~5x, which is real for that regime but not the general case.

**Only the clean-machine paired figures above should be quoted.** Two methodological rules came out of
it, both now applied: **never compare two configurations in a fixed order** when anything can
accumulate between them, and **never let a measurement subject exit on its own** under the agent's
shell — see the trap section.

## The other direction is the real win: paint before the device exists

*Paint something cheap immediately, swap in the real renderer when ready.* Measured as two further
modes, both paired and interleaved against `serial` with the start order rotated so no arm is
systematically favoured. Two independent runs, 12 pairs each:

| | first pixel, run A | first pixel, run B |
|---|---|---|
| serial (D3D `Present` is the first pixel) | 101.9 ms p50 | 139.3 ms p50 |
| **`earlypaint`** — GDI fill in the first `WM_PAINT` | **54.7 ms p50** | **66.3 ms p50** |
| `classbrush` — class background brush, no paint handler | — | 71.5 ms p50 |
| `earlypaint` D3D renderer ready | 73.8 ms p50 | — |
| `classbrush` D3D renderer ready | — | 94.6 ms p50 |

**Painting before the device exists roughly halves time-to-first-pixel** — `earlypaint` came in at
**54%** of serial in run A and **48%** in run B, winning **12 of 12** paired trials in run A. Absolute
values drifted between runs with machine load, but the ratio held, which is the point of pairing.

**A class background brush is equivalent, not better.** `classbrush` beat `earlypaint` in only **4 of
12** pairs. So the *mechanism* does not matter — use whichever is simpler. What matters is only that
something paints without waiting for D3D.

**And that identifies the real remaining floor.** Window creation is ~7 ms, but first pixel lands at
55–70 ms even when the painting itself is a single `FillRect` and even when the system does it during
`ShowWindow` with no handler at all. **The residual ~50–60 ms is window presentation — `ShowWindow`,
DWM composition and first-paint dispatch — not graphics initialisation.** Neither approach can get
below it, because neither is what is costing the time.

That reframes the whole gate. G3 was set up to ask whether the *graphics stack* could paint fast
enough. The answer is that once you stop waiting for it, the graphics stack is no longer the
bottleneck: **the window is.** Any further first-paint work belongs outside the renderer entirely.

### Consequence for the design

- **v1 renders in two stages.** Fill immediately with a solid background, bring up D3D on a worker,
  swap when ready. It halves perceived startup for one `FillRect` and a thread, and the two-stage
  paint is invisible because both stages draw the same background colour.
- **The 40 ms criterion should be re-derived against the window-presentation floor**, not against
  graphics init. At ~55 ms p50 the two-stage approach still misses 40 ms by ~1.4x, but the miss is now
  almost entirely `ShowWindow`/DWM — outside our control and outside what the criterion was written to
  test. State the budget as a percentile above the measured floor on the G5 reference machine.

## D3D11 + DXGI is materially faster than D2D's HwndRenderTarget

Same machine, same session, same `+crt-static` desktop-CRT build, minutes apart:

| stack | total to first pixel | vs G3's 40 ms criterion |
|---|---|---|
| D2D `CreateHwndRenderTarget` | 156.65 ms (140 – 178) | fails ~4x |
| **D3D11 + DXGI, serial** | **126.39 ms (112 – 150)** | **fails ~3x** |

~30 ms, or 19%, for using the stack the spec already mandates. Worth having, and it means the D2D leg
was measuring a configuration Tailhawk was never going to ship.

**Both still fail the 40 ms criterion.** Graphics initialisation — `D3D11CreateDevice` plus swapchain
and RTV creation — is **117 ms of the 126 ms total, or 92%.** Drawing is 2.6 ms. As with the D2D leg,
the conclusion is not "this stack is slow" but "everything except drawing is initialisation", and
threading has now been eliminated as the way out of it.

## Static vs dynamic CRT

| configuration | size | total to first pixel |
|---|---|---|
| dynamic CRT | 178,688 | 166.36 ms (143 – 179) |
| `+crt-static` | 276,480 | 126.39 ms (112 – 150) |

Static costs **+97,792 bytes** and measured *faster* here, which inverts the D2D leg's finding that
the two are equivalent on startup. **Do not rely on this.** The dynamic-CRT run was much noisier
(`CreateWindowExW` spanning 13.8 – 42.4 ms against 7.3 – 13.5 ms static) and the machine had a
Visual Studio installer resident throughout — see the caveat below. The defensible statement remains
the D2D leg's: **choose `+crt-static` on deployment grounds, not performance.**

## ⚠ The trap: leaked subjects silently corrupt every later measurement

**A subject that exits on its own is never reaped under the agent's shell.** The dead process keeps its
D3D device, and they accumulate: session 5 reached 49. `tasklist` lists them, `taskkill` says *"no
running instance"*, and `cargo build` fails with `Access is denied (os error 5)` trying to replace the
exe. Meanwhile `D3D11CreateDevice` degraded **from 55 ms to over 1200 ms** as the count climbed — a 20x
corruption of the headline number, with no error and no obvious symptom.

`PowerShell`'s `$p.Dispose()` does **not** fix it; the count kept climbing with it in place. What works
is what `g3-d2d` does by accident: **never self-terminate.** Report the measurement and wait to be
killed by the harness. `g3-d2d` ran 22 times in one session and leaked zero; `g3-d3d11` self-terminated
via `PostQuitMessage` and leaked one per run. That call has been removed, and `measure.ps1` kills every
subject.

**If a measurement looks inexplicably slow, count the leaked processes before believing it.**

## Caveats — these are not yet the final G3 numbers

- **The clean-machine figures above are the only ones to quote.** No installer, no leaked processes,
  zombie count verified at 0 before and throughout. Earlier revisions of this page quoted numbers taken
  with a Visual Studio installer resident — it also demonstrably broke a build, failing a
  `+crt-static` link with `LNK1104: libucrt.lib` for two crates then succeeding on retry.
- **Absolute first-pixel values are not stable on this machine, and chasing them is a dead end.** The
  same static build has produced serial totals of 96, 112, 126, 139 and 154 ms. The cause is now
  identified and is not mysterious: **the machine carries a variable ~40% background load** from a
  normal working set — Teams, Edge WebView2, Docker, OneDrive, Outlook — which is not going away. A
  21-run set spanning that load gave a p50 of 297 ms and a range of 117–783 ms on the D2D leg.
- **So absolute first-paint figures are blocked on `PLAN.md` §3 G5's fixed reference machine, not on a
  reboot.** That was already an open item (`HANDOFF.md` open question 3) and this is simply another
  reason it has to be settled before any `SPEC.md` §11.3 number is published.
- **Use the paired comparisons for every design decision.** Interleaved A/B on the same machine within
  minutes is reliable and reproduced across runs; single absolute numbers from this machine are not. Load affects both arms equally.
- **A cold set is still owed.** `docs/HANDOFF.md` records that the same `g3-d2d` binary produced a
  113 ms `CreateWindowExW` median in session 3 and 8.5–11.9 ms in session 5, a 13x discrepancy with
  non-overlapping ranges and no established cause. Until a post-reboot set and a quiet-machine set
  agree, no absolute figure here should become a `SPEC.md` §11.3 target.
- Single machine, single GPU, `driver: hardware` on every run. WARP was never exercised.
- `Present(0, …)`, so the figure is time-to-submission. A photon costs up to one refresh interval
  more. Consistent with the D2D leg's framing, and both exclude process creation and loader time.

## Reproducing

No `.cargo/config.toml` override is needed any more — the desktop C++ workload is installed.

```powershell
cargo build --release                      # or with $env:RUSTFLAGS="-C target-feature=+crt-static"
.\experiments\measure.ps1 -Exe target\release\g3-d3d11.exe -OutFile g3-d3d11-serial.txt `
    -ExeArgs serial     -Columns "mode,window,device,swapchain,draw,total,driver"
.\experiments\measure.ps1 -Exe target\release\g3-d3d11.exe -OutFile g3-d3d11-concurrent.txt `
    -ExeArgs concurrent -Columns "mode,window,device_wait,swapchain,draw,total,driver"
```

The binary self-terminates after reporting one CSV line to `%TEMP%\g3-d3d11-<mode>.txt`.
