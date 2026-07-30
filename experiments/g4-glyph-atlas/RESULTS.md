# G4 — glyph atlas composition: colour and mono in one pass

`PLAN.md` §3 G4. **Pass criterion: "One instanced draw per viewport is preserved, or the cost of the
extra pass is measured and accepted; eviction does not stall a frame." Both met — with one condition
on eviction that the plan did not anticipate.**

Measured 2026-07-30.

## The claim under test

`PLAN.md` §3 states, as the reason G4 exists:

> a premultiplied colour atlas **cannot share the mono atlas's blend state**, so it breaks the
> one-instanced-draw rule that the whole renderer design rests on.

**That is refuted.** One instanced draw with one blend state renders monochrome ClearType text and
premultiplied colour emoji together, and the pixels prove it.

## Method

D3D11 + DXGI flip-model swapchain (the stack `SPEC.md` §3 specifies, not D2D's `HwndRenderTarget`
as in G3), with the hardware → WARP fallback chain. Glyphs rasterised by DirectWrite:

- **mono** — `IDWriteFactory2::CreateGlyphRunAnalysis` + `CreateAlphaTexture` with
  `DWRITE_TEXTURE_CLEARTYPE_3x1`, giving three subpixel coverages per pixel.
- **colour** — `IDWriteFactory2::TranslateColorGlyphRun`, enumerating the COLR layers, rasterising
  each and compositing to premultiplied BGRA.

Two 1024×1024 atlas sheets: mono as `R8G8B8A8_UNORM` (RGB = subpixel coverages, A = their average),
colour as `B8G8R8A8_UNORM` premultiplied. Fixed 20×22 slots, one glyph per slot.

Fonts resolved on this machine: **Cascadia Mono**, **Microsoft YaHei**, **Segoe UI Emoji**.
Font *fallback* is deliberately not used — one face is picked per script so glyph-run construction
stays direct. Mixed-script fallback is a separate concern.

## How one blend state serves both

The pixel shader emits **two** outputs and dual-source blending consumes the second:

```hlsl
struct PSOut {
    float4 c0 : SV_Target0;   // premultiplied colour
    float4 c1 : SV_Target1;   // per-channel coverage
};
```

with `SrcBlend = ONE`, `DestBlend = INV_SRC1_COLOR`, so the hardware computes

```
dest = c0 + dest * (1 - c1)
```

That single equation is correct for **both** cases at once:

| glyph kind | `c0` | `c1` | resulting blend |
|---|---|---|---|
| mono | `tint.rgb * cov` (cov is a 3-vector) | `cov` | per-channel — i.e. ClearType |
| colour | premultiplied texel `t.rgb` | `t.aaa` | standard premultiplied composite |

The mono path premultiplies *in the shader*, which is what makes the two sources agree on a blend
equation. A per-instance `mode` field selects between them, so both live in one instance buffer and
one `DrawInstanced` call.

**Trap:** the alpha slots must use `INV_SRC1_ALPHA`, not `INV_SRC1_COLOR`. D3D11 rejects a `*_COLOR`
blend factor in an alpha slot with a bare `E_INVALIDARG` from `CreateBlendState`.

## Correctness — the pixels, not the timings

The verification frame forces the tint **and** the background to exactly neutral grey
(0.85 / 0.10), then reads the back buffer to system memory. With a neutral tint, channel spread in a
monochrome glyph can only come from per-channel coverage surviving the blend, and saturation can only
come from the colour atlas. (The first version of this check used the real tint, 0.87/0.89/0.92 —
itself non-neutral enough to fake ~13 levels of "fringing". It proved nothing.)

```
readback 1264x761, neutral tint and background:
  109,220 ink pixels
   38,225 with per-channel spread > 8 (max 79)
   54,424 strongly saturated
```

- **ClearType subpixel coverage: PRESENT** — max spread 79 levels on a neutral tint.
- **Colour glyphs: PRESENT** — 54,424 saturated pixels from a neutral tint.
- Both in the **same single instanced draw with a single blend state.**

## Draw-call cost

2,860 instances (2,816 mono + 44 colour emoji), 120 measured frames, GPU time from D3D11 timestamp
queries with the draw repeated 50× inside the bracket. **Three independent runs are quoted** — the
third on the desktop CRT after the toolchain change. Absolute timings move by tens of percent between
runs on a loaded desktop, so no single run's figures should be read as precise.

| configuration | draws | CPU p50 | GPU p50 |
|---|---|---|---|
| **unified — 1 draw, 1 blend state** | 1 | **0.268 / 0.373 / 0.426 ms** | 0.036 / 0.029 / 0.071 ms |
| split — 2 draws, 2 blend states | 2 | 0.394 / 0.501 / 0.503 ms | 0.019 / 0.023 / 0.040 ms |

**Unified is cheaper on CPU in all three runs, by 15–32%** — the split has to sort the instance buffer
to partition it by mode, and set state twice. On GPU both sit under 0.08 ms, well under 0.5% of a
16.7 ms frame; dual-source blending is not measurably expensive at this scale.

**The split is not actually an equivalent alternative.** Its mono pass uses a single-source
straight-alpha state, and a single-source blend has exactly one alpha per pixel — it *cannot* consume
three independent per-channel coverages. Straight alpha therefore forfeits ClearType entirely. To get
subpixel coverage without dual-source blending you need two passes over the *same* geometry (darken
by `1-cov`, then add `tint*cov`), which is worse than either option measured here. So dual-source
blending is not merely the cheaper choice, it is what makes one-pass ClearType possible at all.

## Eviction — passes, but only with an O(1) policy

The CJK fixture streams a different 1,500-glyph slice of the 20,992-codepoint ideograph block every
frame, so the working set never fits and essentially every glyph evicts another. 120 frames, atlas
capacity 2,346 slots, 180,000 misses and 177,654 evictions in each run.

| eviction policy | bookkeeping per frame (run 1 / 2 / 3) |
|---|---|
| scan LRU — O(capacity) per miss | **7.58 / 4.35 / 8.51 ms** |
| list LRU — O(1) per miss | **0.37 / 0.17 / 0.31 ms** |

**A 20–28x difference in every run, and it decides the criterion.** The obvious implementation — scan
every slot for the oldest — spends 4–9 ms per frame purely on bookkeeping, up to half a 60 Hz frame
budget, before any rasterising or drawing. An intrusive doubly-linked LRU over slot indices costs
0.17–0.37 ms for the same 1,500 evictions, or well under a microsecond each.

Worse, an earlier variant of this experiment allowed **variable-width glyphs spanning adjacent
slots**, which made the victim search O(capacity × span) and had to find a free *run*. That measured
**106 ms per frame** of eviction bookkeeping. Uniform single-glyph slots are what make O(1) possible:
no repacking, no run-finding, no fragmentation.

The cost of uniform slots is atlas density — 20×22 slots give 2,346 cells where 12×20 would give
4,335, so roughly 46% of the sheet is lost to padding around narrow Latin glyphs. That is a good
trade for a monospace log grid, where glyph widths are nearly uniform anyway, and `failed 0` confirms
every Latin, CJK and emoji glyph at em 14 fitted the 20×22 box.

## The actual stall is rasterisation, and it is large

| | per frame (run 1 / 2 / 3) | quiet machine, 2026-07-30 |
|---|---|---|
| DirectWrite rasterisation (1,500 misses) | **312 / 227 / 582 ms** | **494.6 / 581.7 ms** |
| atlas upload (`UpdateSubresource` ×1,500) | 5.7 / 2.9 / 5.9 ms | 5.85 / 5.85 ms |
| eviction bookkeeping (list LRU) | 0.37 / 0.17 / 0.31 ms | 0.31 ms |
| eviction bookkeeping (scan LRU) | 4 – 8 ms | 8.51 ms |
| GPU draw | 0.22 / 0.18 / 0.19 ms | 0.19 / 0.19 ms |

**145–390 µs per glyph.** A viewport of 1,500 previously-unseen CJK glyphs needs 230–580 ms —
**14 to 35 frames at 60 Hz.** Eviction is three orders of magnitude cheaper than the rasterisation it
triggers, and rasterisation is the only term large enough to matter.

### ⚠ The spread is not machine load, and the quiet number is the slow one

This section previously said the spread tracked machine load. **Re-run on a verified quiet machine
(0–6% CPU, minutes after a reboot, same `+crt-static` binary), rasterisation came in at 494.6 and
581.7 ms per frame — at the *top* of the 227–582 ms range, not the bottom.** That is **330–388 µs per
glyph**, so:

- **A cold run does not bound this cost favourably.** The pessimistic end of the earlier range is the
  honest figure to design against, and the "it's just load" reading is withdrawn.
- **There is a within-process ordering effect of ~18%.** The two overflow phases rasterise *identical*
  work — 1,500 misses, 180,000 total, same glyphs — yet the phase that runs second measured 581.7 ms
  against the first's 494.6 ms, in a single quiet process. So **the fixed-order rule from
  `experiments/g3-d3d11/RESULTS.md` applies inside this binary too**: the two LRU phases run in a fixed
  order and their raster columns are not comparable to each other. Their *eviction* columns are the
  measurement that matters and the gap there is 27x, far outside the effect.
- **The O(1) eviction requirement is reconfirmed on quiet numbers:** 8.51 ms scanning versus 0.31 ms
  with the intrusive list.

The likely cause of the rasterisation cost is granularity: this code calls `CreateGlyphRunAnalysis`
once **per glyph**, which allocates a COM object and does three interface calls per glyph. Batching a
whole run into a single analysis should be far cheaper, and is **not tested here** — worth its own
experiment before the number is treated as a floor. **The quiet re-take raises the value of that
experiment**: the dominant renderer expense is confirmed at the pessimistic end, and it is not going to
be explained away by machine state.

## Consequences for the design

1. **`SPEC.md` §11.2's one-instanced-draw rule survives colour emoji.** No re-costing of V2 is
   needed on this account. The mechanism must be recorded, though: dual-source blending with a
   premultiplying mono path, not two atlases in two passes.
2. **Glyph rasterisation must be off the paint path.** At 145–210 µs/glyph a cold viewport cannot be
   rasterised within a frame. The grid needs to draw *something* for a glyph that is not yet resident
   — a placeholder box, or the previous frame's content — and fill in over subsequent frames. This is
   a v1 requirement, not an optimisation, and `SPEC.md` currently does not say it.
3. **The atlas is a fixed-slot LRU with uniform slots and an O(1) victim list.** Not a shelf packer,
   not variable-width spans. This is a design constraint with a measured 20x–290x justification.
4. **Cache the absence of ink.** A glyph with no raster (space, or a codepoint absent from the face)
   must be cached as a blank. Without it, every space is re-rasterised every frame — on the first run
   of this experiment a 44-row fixture with ten spaces per row produced exactly 440 spurious misses
   per frame.
5. **Colour glyphs cannot carry subpixel AA.** A coloured layer's three coverages cannot survive an
   alpha composite against a different colour, so colour layers are averaged to greyscale coverage
   before compositing. Harmless — they are pictorial — but it means the mono and colour paths differ
   in AA quality by construction, not by oversight.

## Traps found, worth not rediscovering

- **`GetData` returns `S_FALSE` when a query result is not ready, and `S_FALSE` is a *success*
  HRESULT.** `windows`'s `Result<()>` is `Ok` in that case, so `is_err()` is useless as a readiness
  test. The first version of this experiment span on `is_err()`, exited immediately, read zeroed
  memory, and reported **0.000 ms GPU time for every frame**. Detect readiness with a sentinel value
  the driver must overwrite.
- **`DWRITE_GLYPH_RUN::fontFace` is `ManuallyDrop<Option<IDWriteFontFace>>`.** Writing a
  `transmute_copy` *into* it is safe (nothing is released). Copying it *out* into an owned
  `IDWriteFontFace` is not — that releases a reference which was never added, and the refcount
  underflow surfaces later as a use-after-free. Borrow the layer face out of a
  `DWRITE_COLOR_GLYPH_RUN`; never take it by value.
- **Present must be outside the timed region.** With the flip model it blocks on the back-buffer
  queue, which pinned every measured frame to exactly 16.669 ms — one refresh interval — and
  completely hid the real CPU cost.
- **A single pass of a few thousand small quads is below the noise floor** of a swapchain-coupled
  measurement. Repeat the draw inside the timestamp bracket and divide.

## Caveats — read before quoting these numbers

- ~~Still linked against the OneCore CRT~~ — **resolved 2026-07-30.** Re-run on the **desktop CRT** with
  `+crt-static` after the workload was installed and the `LIB` override deleted. Every conclusion
  reproduced: unified still cheaper on CPU, eviction ratio still 20–28x, correctness readback
  bit-identical. Run 3 in the tables above is that run. A Visual Studio installer was resident for it,
  which is the likely reason its absolute rasterisation figure is the highest of the three.
- Single machine, single GPU, `driver: hardware`. WARP was not exercised, and it is the rung where a
  dual-source-blend assumption is most likely to differ.
- **Shaders are compiled at runtime via `D3DCompile`.** `SPEC.md` §3 requires offline compilation
  with embedded bytecode to avoid a `d3dcompiler_47.dll` dependency; that is a packaging concern and
  does not affect these measurements.
- Rasterisation cost is measured at one-glyph-per-analysis granularity. Batched runs are untested.
- The mono atlas's alpha channel carries the greyscale average, so one sheet serves both subpixel and
  greyscale rendering at no extra memory. Greyscale mode (`MODE_MONO_GREY`) exists in the shader but
  was not separately benchmarked.
- No font fallback, no DPI scaling, no italic/bold, one em size.

## Reproducing

The `LIB` override in the git-ignored `.cargo/config.toml` is still required on this machine.

```
cargo run --release -p g4-glyph-atlas
```

Runs the verification frame, then four measured phases, and writes the report to
`%TEMP%\g4-glyph-atlas.txt`. Failures and panics go to `%TEMP%\g4-glyph-atlas-errors.txt`. The window
stays open showing a representative frame; close it to exit. Takes about 100 seconds, almost all of
it in the two CJK thrashing phases.
