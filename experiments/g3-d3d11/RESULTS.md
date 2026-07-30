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

**⚠ This page previously concluded from those 55–70 ms figures that the residual cost was window
presentation — `ShowWindow`, DWM composition and first-paint dispatch — and therefore outside the
renderer. That is withdrawn. It was load.** See the next section.

### Consequence for the design

- **v1 renders in two stages.** Fill immediately with a solid background, bring up D3D on a worker,
  swap when ready. It halves perceived startup — and on a quiet machine does far better than halve it
  — for one `FillRect` and a thread, and the two-stage paint is invisible because both stages draw the
  same background colour.

## The ~50–60 ms "window-presentation floor" does not exist — it was load

Measured 2026-07-30 (session 6) in the minutes after a reboot, on the **same `+crt-static` binaries**
from `target-verify-static\release\` — no rebuild, so the subject is byte-identical to the runs above.
Two conditions, 11 process starts per set, leaked-subject count verified at **0** before and after
every set.

`first_pixel` is `main()` entry → `FillRect` returning inside the first `WM_PAINT`, so **`ShowWindow`
and paint dispatch are inside the measured region** — which is what makes the figure interpretable.

| | cold: uptime 3–6 min, 36% load, boot churn | quiet: uptime 6–10 min, 0–6% load | session 5, working-set load |
|---|---|---|---|
| `CreateWindowExW` | 4.17 p50 | 3.20 p50 | ~7 |
| **`earlypaint` first pixel** | **13.4 p50 / 18.0 p90** | **13.1 p50 / 14.5 p90** | 54.7 / 66.3 p50 |
| `earlypaint` D3D ready | 54.9 p50 | 45.8 p50 | 73.8 p50 |
| serial, total to first pixel | 84.3 p50 / 118.2 p90 | 68.6 p50 / 81.7 p90 | 126.4 p50 |
| D2D `HwndRenderTarget`, total | 114.3 p50 (80.8 – 1604.4) | 75.5 p50 / 87.2 p90 | 156.7 p50 |

**`ShowWindow` + DWM composition + first-paint dispatch costs ~10 ms beyond window creation, not
50–60 ms.** Nothing about window presentation is expensive on this machine.

### The two-stage paint passes the 40 ms criterion

At **13.1 ms p50, 14.5 ms p90, and 14.5 ms worst of 11 runs**, the two-stage paint clears G3's 40 ms
criterion with ~2.7x headroom. Session 5's recommendation that *"the 40 ms criterion should be
re-derived against the window-presentation floor"* is **withdrawn** — there is no floor to re-derive it
against.

**The gate now splits cleanly in two, and the split is the result:**

| | quiet-machine first pixel | vs 40 ms criterion |
|---|---|---|
| wait for the graphics device (`serial`) | 68.6 ms p50 | fails ~1.7x |
| **paint before it exists (`earlypaint`)** | **13.1 ms p50** | **passes, ~3x** |

So the criterion tests the **paint order**, not the graphics stack. `SPEC.md` §3.2 already requires the
two-stage paint for v1; this is the measurement that says the requirement is sufficient, not merely
helpful.

Unchanged caveats, and they still bar publishing this as a target: one machine, one GPU, `hardware`
driver on every run, and **not** the `PLAN.md` §3 G5 reference machine. `first_pixel` is
time-to-`FillRect`, not time-to-photon — a GDI fill still waits on DWM composition, up to one refresh
interval more.

### What load explains, and what it does not

- **It explains the whole absolute spread.** Every quiet figure lands at or below the fast end of the
  range session 5 observed (96, 112, 126, 139, 154, 297 ms). `HANDOFF.md` posed this as a two-way
  branch; **the first branch is the one that held**, so these are the defensible figures for
  `SPEC.md` §11.3 — as p50/p90 with the machine state stated, never as a mean.
- **The two-stage paint is far less load-sensitive, but it is not immune.** Its p50 moved 13.4 → 13.1
  across this session's two conditions while D2D's moved 114.3 → 75.5 with a 1604 ms outlier inside the
  same set. But session 5's 54.7–66.3 ms came from the *working-set* load (Teams, Docker, Edge
  WebView2, OneDrive, Outlook), which is not the same thing as boot churn, and it does delay paint
  dispatch. The asymmetry is the design argument, not immunity: everything that waits for the device
  inherits the machine's worst case, and the fill mostly does not.
- **Post-reboot is not the same as quiet, and uptime is the wrong thing to record.** The first four runs
  of the first cold set gave totals of 174, 225, 264 and **1604 ms** before settling to 80–114 ms —
  boot-time service churn is itself load. The quiet window on this machine opened at roughly six
  minutes' uptime. **Record CPU load, not uptime**, and re-check it after every set.

## D3D11 + DXGI is materially faster than D2D's HwndRenderTarget

Same machine, same session, same `+crt-static` desktop-CRT build, minutes apart:

| stack | total to first pixel | quiet-machine re-take | vs G3's 40 ms criterion |
|---|---|---|---|
| D2D `CreateHwndRenderTarget` | 156.65 ms (140 – 178) | 75.5 ms p50 | fails ~1.9x |
| **D3D11 + DXGI, serial** | **126.39 ms (112 – 150)** | **68.6 ms p50** | **fails ~1.7x** |

~30 ms, or 19%, for using the stack the spec already mandates — and **~7 ms, or 9%, on the quiet
re-take**, so the direction holds but the margin is smaller than the loaded figures suggested. Both
re-takes were separate 11-run sets minutes apart at 0–6% load, not paired trials, so treat the 9% as
indicative and the direction as the finding. Worth having, and it means the D2D leg
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
- **The cold and quiet sets are taken (2026-07-30, session 6) and they settle the absolutes as far as
  one machine can.** The bullets below are retained as the record of why they were needed; where they
  say a set is owed, it has since been taken — see "The ~50–60 ms window-presentation floor does not
  exist" above.
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
- ~~**A cold set is still owed.**~~ **Taken 2026-07-30.** The post-reboot and quiet sets agree with each
  other and with session 5's `CreateWindowExW` figures: 4.17 and 3.20 ms p50 here against session 5's
  8.5–11.9 ms, all in the same order of magnitude. **Session 3's 113 ms remains the lone outlier and is
  now conclusively not a floor** — four independent sets across three sessions land between 3 and 12 ms.
  Its cause is still unexplained, and no longer worth explaining.
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

The binary reports one CSV line to `%TEMP%\g3-d3d11-<mode>.txt` and then **waits to be killed** by
`measure.ps1` — the `PostQuitMessage` it originally had is gone, because self-terminating subjects leak
(see the trap above). The `earlypaint` leg needs its own column list:

```powershell
.\experiments\measure.ps1 -Exe target\release\g3-d3d11.exe -OutFile g3-d3d11-earlypaint.txt `
    -ExeArgs earlypaint -Columns "mode,window,first_pixel,d3d_ready,driver"
```

**`-OutFile` is not a free choice — it must match the filename the binary hardcodes.** Pass anything
else and `measure.ps1` polls for a file nobody writes, warns *"run N produced no output"* eleven times
and throws. It cost a whole set in session 6.
