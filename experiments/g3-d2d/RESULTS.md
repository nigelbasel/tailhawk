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

## Binary size

| Build | Bytes | vs 2 MB criterion |
|---|---|---|
| Dynamic CRT | 146,432 (0.14 MB) | **passes, ~14x** |
| `+crt-static` (as G3 specifies) | 243,712 (0.23 MB) | **passes, ~8x** |

Static CRT costs **+97,280 bytes (+66%)** and is the configuration G3 asks for, since a
copy-and-run single exe should not depend on a redistributable being present.

Worth carrying forward: with a 0.23 MB floor, the 15 MB CI gate has far more headroom than assumed.
This independently reinforces `docs/LOKI.md` §5 — the claim that an HTTP stack threatened the gate
was already refuted at +1.68 MB measured, and against this floor it is not close.

## First pixel

Seven cold starts per configuration. Medians, with range:

| Phase | Dynamic CRT | `+crt-static` |
|---|---|---|
| `D2D1CreateFactory` | 0.20 ms (0.14 – 0.38) | 0.18 ms (0.13 – 0.65) |
| `CreateWindowExW` | 113 ms (21 – 144) | 73 ms (25 – 84) |
| `CreateHwndRenderTarget` | 176 ms (152 – 210) | 171 ms (156 – 216) |
| `Clear` + `EndDraw` | 6.0 ms (5.0 – 6.9) | 6.0 ms (5.5 – 6.2) |
| **Total** | **273 ms (218 – 327)** | **249 ms (194 – 303)** |

**Against G3's 40 ms criterion: FAIL, by ~6x.** An empty window also exceeds `SPEC.md` §11.3's
150 ms budget for the whole app painting real content, by ~1.7x.

Static CRT is nominally 24 ms faster, but the entire difference sits in `CreateWindowExW` — the
noisiest phase, with ranges that overlap heavily. **Treat the two configurations as equal on startup
and choose `+crt-static` on deployment grounds, not performance.**

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
aggressive first-paint budget on this hardware. Two directions, neither yet tested:

- **Take device creation off the critical path.** `SPEC.md` already specifies D3D11 + DXGI rather
  than D2D's `HwndRenderTarget`; a D3D11 device can be created on a worker thread concurrently with
  window creation and a swapchain attached once the `HWND` exists. That potentially removes most of
  the 176 ms from the serial path, bounded below by the 113 ms window creation.
- **Paint something cheap immediately.** Fill the window on `WM_ERASEBKGND` and swap in the real
  renderer when ready. First *pixel* then approaches the ~113 ms window-creation floor, at the cost
  of a visible two-stage paint.

Neither gets to 40 ms. **The honest reading is that G3's 40 ms criterion was set without
measurement and should be re-derived** — `PLAN.md` §3 anticipates exactly this: *"the 15 MB CI gate
and SPEC §11.3's first-paint budget are both re-derived from measurement."* A floor of ~113 ms for
an empty window on a current machine is the number to build the budget around.

## Caveats — read before quoting these numbers

- **Linked against the OneCore CRT, not the desktop CRT.** This machine's MSVC toolset has
  `lib\onecore\x64` but no `lib\x64`, so `LIB` was pointed at the OneCore variant to link at all.
  **The desktop C++ workload is due to be installed; re-take every figure here once it is.**
- Single machine, single GPU, no cold-boot separation. `PLAN.md` §3 G5 requires a fixed reference
  machine before any of this becomes a published target.
- Timing excludes process creation, so treat every figure as a lower bound.
- The `eframe+glow` and `eframe+wgpu` subjects are still outstanding, so the *comparison* G3 was
  designed to make has not been made — only the windows-rs + D2D leg.

## Reproducing

The `LIB` override is required on this machine and is set in a git-ignored `.cargo/config.toml`.
Without it, linking fails with `LNK1104: cannot open file 'msvcrt.lib'`.

```
cargo build --release
target\release\g3-d2d.exe        # writes %TEMP%\g3-d2d-first-pixel.txt
```

Output is one CSV line: `factory,window,target,draw,total` in milliseconds.
