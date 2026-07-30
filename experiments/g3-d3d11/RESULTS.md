# G3, second leg — first pixel on D3D11 + DXGI

`PLAN.md` §3 G3, measured on **the stack `SPEC.md` §3 actually specifies** rather than the D2D
`HwndRenderTarget` of `experiments/g3-d2d`. It also tests the fix that leg proposed but never tried.

Measured 2026-07-30, **desktop MSVC CRT** (the shipping configuration — the OneCore workaround is
gone), `+crt-static`, 7 cold process starts per configuration via `experiments/measure.ps1`.

## Headline: the fix G3 proposed does not work

`experiments/g3-d2d/RESULTS.md` concluded that graphics device creation must come off the critical
path, and offered *"Take device creation off the critical path… a D3D11 device can be created on a
worker thread concurrently with window creation."* That is now measured, and it is **wrong**.

| phase | serial | concurrent |
|---|---|---|
| `CreateWindowExW` | 9.27 ms (7.3 – 13.5) | 13.27 ms (11.7 – 18.3) |
| `D3D11CreateDevice` / wait for it | 60.05 ms (57.2 – 70.4) | 58.63 ms (51.7 – 67.2) |
| swapchain + RTV | 56.78 ms (42.1 – 72.2) | 63.12 ms (53.2 – 121.6) |
| `Clear` + `Present` | 2.65 ms (2.2 – 3.8) | 5.81 ms (4.0 – 7.6) |
| **total** | **126.39 ms (112 – 150)** | **140.06 ms (131 – 203)** |

**Concurrent is 11% *slower* than serial.** The mechanism is visible in the numbers: in concurrent
mode the device is created on a worker thread that starts at `main()` entry, so by the time the main
thread has finished `CreateWindowExW` the device should be most of the way done. It isn't — the wait
after the window exists is **58.63 ms** against a serial device cost of **60.05 ms**. The worker saved
**1.4 ms**.

Two reasons, and the first is the interesting one:

1. **Window creation and D3D11 device creation contend rather than overlap.** Both touch graphics
   driver and DWM initialisation, so running them in parallel does not buy their sum. The theoretical
   ceiling was only ~9 ms anyway (you cannot hide more than `min(window, device)`), and even that was
   not realised.
2. **Thread overhead and contention cost more than the overlap saved** — every other phase got worse,
   including a doubled draw time and a much worse swapchain tail (121.6 ms max).

**This closes the question rather than leaving it open.** The remaining lever from that leg's list is
the other one: *paint something cheap immediately* and swap in the real renderer when ready. First
*pixel* then approaches the ~9 ms window-creation cost plus a GDI fill, because nothing in the D3D
path is on the critical path at all. Untested.

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

## Caveats — these are not yet the final G3 numbers

- **A Visual Studio installer was resident for every measurement on this page.** Two `setup.exe`
  processes were running throughout, and they demonstrably interfered: a `+crt-static` link failed
  once with `LNK1104: cannot open file 'libucrt.lib'` for two crates while succeeding for the other
  two, then succeeded for all four on an immediate retry. That is files moving underneath the linker.
- **The absolute first-pixel figures are therefore provisional.** What *is* robust is every
  comparison on this page, because each is an A/B measured minutes apart under identical conditions:
  serial vs concurrent, D2D vs D3D11, static vs dynamic. Load affects both arms equally.
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
