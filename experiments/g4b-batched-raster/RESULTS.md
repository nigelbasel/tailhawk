# G4b — batched glyph rasterisation

Follow-up to the open caveat in `../g4-glyph-atlas/RESULTS.md`: *"Rasterisation cost is measured at
one-glyph-per-analysis granularity. Batched runs are untested."* G4 measured DirectWrite at
145–390 µs per glyph and named batching a whole run into a single `CreateGlyphRunAnalysis` as the
likely fix, without testing it. `docs/HANDOFF.md` ranked it the highest-value unblocked experiment.

Taken 2026-07-30, session 7, on the desktop CRT. Machine quiet throughout: **2–5% CPU**, zero leaked
subjects before and after. No window, no D3D device, no swapchain — deliberately, so the
leaked-subject trap that cost session 5 two conclusions cannot apply here.

## The answer in one line

**Batching works, is bit-identical, and is worth ~1.8x — but the premise was wrong.** Per-analysis
overhead was never the dominant cost. The dominant cost is a **cross-process, capacity-limited
system font cache**: the first process on a machine to rasterise a glyph pays ~86–108 µs, and every
process afterwards pays ~3 µs for the same glyph. G4's 330–388 µs/glyph is a *cache-thrash* figure
produced by a fixture that cycles 20,992 distinct glyphs on purpose.

## 1. Batching does not perturb the raster

The gating question, because a speedup is worthless if the output cannot feed an atlas: grid fitting
and the ClearType filter are both position-dependent, so a glyph rasterised amid neighbours could
legitimately differ from the same glyph rasterised alone.

It does not. Both arms produce the same artefact — one uniform atlas cell per glyph, so the batched
arm pays to slice its wide bitmap back into cells — and the cells are **bit-identical**:

| inter-cell pad | overflow | worst cell mismatch, batch 4 → 1500 |
|---|---|---|
| 0 px | 0 | **0 of 1500** |
| 2 px | 0 | 0 of 1500 |
| 4 px | 0 | 0 of 1500 |
| 8 px | 0 | 0 of 1500 |

Identical at **pad 0**, so no inter-glyph gap is required: DirectWrite rasterises each glyph
independently into the strip and the ClearType filter does not bleed across the boundary. Cell
geometry was *derived* from measured bounds (16×15 at (−1, −12) for Microsoft YaHei at em 14), not
guessed — the first version of this experiment guessed 20×26 and clipped 1,086 of 1,500 glyphs, which
would have made cells compare equal for the wrong reason.

## 2. Batching is worth ~1.8x, and only on cold glyphs

Every arm gets a **disjoint slice** of the ideograph block, so no arm can benefit from another's
misses, and the sweep is repeated at a second em in reverse order to catch a warm-up trend.

| glyphs per analysis | em 17, forward | em 18, reverse | vs batch 1 |
|---|---|---|---|
| 1 | 86.0 µs/glyph | 86.7 | 1.00x |
| 4 | 51.1 | 51.8 | **1.68x** |
| 16 | 53.8 | 49.1 | 1.68x |
| 64 | 45.3 | 48.3 | **1.84x** |
| 256 | 50.0 | 52.2 | 1.69x |
| 1000 | 50.6 | 50.6 | 1.71x |
| 1000 (second slice) | 42.1 | 47.8 | 1.92x |

**The win saturates at batch 4.** Everything from 4 to 1000 sits in 1.68–1.92x with no trend, and the
two independent 1000-glyph replicates agree at 1.71x and 1.92x. So a renderer should batch, but there
is no reason to batch a whole viewport — a handful of glyphs per analysis collects the entire benefit.

**Replicated.** A second run of the whole sweep gives batch 1 at 82.0 / 84.2 µs/glyph and ratios of
1.59–2.01x — the same picture. That the second run is still *cold* is itself consistent with the
capacity finding below: the sweep's 14,000 glyph-size pairs sit at the top of the cache's range, so
they do not survive to a second run.

**On warm glyphs batching wins nothing.** The in-process sweep over already-cached glyphs:

| batch | p50 | vs b1 | create | bounds | texture | copy |
|---|---|---|---|---|---|---|
| 1 | 3.69 ms | 1.00x | 0.46 | 0.09 | 2.53 | 0.60 |
| 16 | 3.38 ms | 1.09x | 0.08 | 0.01 | 2.72 | 0.57 |
| 1024 | 4.01 ms | 0.92x | 0.06 | 0.00 | 3.30 | 0.61 |
| 1500 | 4.02 ms | 0.92x | 0.06 | 0.00 | 3.35 | 0.61 |

`create` collapses 8x, from 0.46 ms to 0.06 ms — batching does remove per-analysis overhead exactly as
predicted. It just does not matter: `CreateAlphaTexture` is ~70% of the total and is flat, and past
batch 256 the larger intermediate bitmap makes the total **worse**. **Large batches are a pessimisation.**

## 3. The real mechanism: a cross-process font cache

The measurement that reframes everything. Running the same 1,500-glyph pass as the first DirectWrite
work in a fresh process, repeatedly:

| process | total | µs/glyph | of which `CreateGlyphRunAnalysis` |
|---|---|---|---|
| **1st ever** | **162.5 ms** | **108.3** | **157.4 ms — 97% of it** |
| 2nd | 4.4 ms | 3.0 | 0.76 ms |
| 3rd … 6th | 4.3 – 5.1 ms | 2.9 – 3.4 | 0.38 – 0.53 ms |

**36x, and it survives process exit.** So the cache is not per-process state — it is the shared system
font cache. `Windows Font Cache Service` (`FontCache`) is running on this machine and is the obvious
candidate; that identification is **inference from the cross-process persistence, not verified**. The
confirming test — stop the service and re-measure — is a system mutation and was not run.

The cost lives in `CreateGlyphRunAnalysis`, not `CreateAlphaTexture`: the analysis object is where
rasterisation actually happens, and `CreateAlphaTexture` merely copies the result out.

### Capacity: between 8,000 and 16,000 distinct glyphs

Each size gets its own em, because the cache key includes size — that makes every row cold regardless
of what earlier runs touched, without needing a reboot. Per-glyph cost rises with em, so the signal is
the scale-free **cold/warm ratio**.

| distinct glyphs | em | cold | warm | warm again | cold/warm |
|---|---|---|---|---|---|
| 500 | 20 | 108.8 µs | 8.8 | 8.3 | 12.4x |
| 1,000 | 21 | 123.9 | 10.2 | 9.7 | 12.1x |
| 2,000 | 22 | 115.0 | 11.0 | 10.3 | 10.4x |
| 4,000 | 23 | 124.4 | 11.3 | 11.0 | 11.0x |
| 8,000 | 24 | 144.3 | 14.3 | 13.6 | 10.1x |
| **16,000** | 25 | 258.9 | **245.4** | 226.7 | **1.1x** |

Up to 8,000 the set stays resident and re-use is 10–12x cheaper. At 16,000 the ratio collapses to 1.1x
— the second pass is still paying full price, so the set no longer fits. The boundary is **bracketed,
not pinned**, and since the budget is probably in bytes rather than glyphs it will move with em size.

## 4. Reconciling G4

G4's number is reproducible, and it is a thrash figure.

| measurement | µs/glyph |
|---|---|
| G4 as reported, session 6 (post-reboot, quiet) | 330 – 388 |
| **G4 binary re-run today**, same machine, quiet | **92 – 97** |
| This experiment's replica of `raster_mono`, over G4's full 20,992-glyph block | 94 – 129 |
| …replicating G4's 120-frame cycling loop exactly | 173 |
| This experiment's replica, over 1,500 glyphs, warm | 4.0 |
| This experiment's own arm, 1,500 glyphs, first-ever process | 108 |

G4's fixture draws each frame from a different slice of the 20,992-codepoint ideograph block so that
the working set never fits **the atlas** — that was the point, it was built to force eviction. It also,
unintentionally, never fits the **font cache**, which is 2.5x smaller than the fixture. So G4 measured
the sustained-miss cost, correctly, but of a workload no log viewport reaches.

**This also explains session 6's anomaly.** Session 6 recorded that the quiet post-reboot re-take came
in at the *top* of the range and concluded "a cold run does not bound this cost favourably — the
spread is not load". The observation was right and the mechanism was wrong: a reboot **empties the
font cache service**, so the post-reboot run was the most cache-cold run and therefore the slowest. It
was never about CPU load in either direction. Today's re-run, with the cache partly warm, gives 92–97
against session 6's 330–388 on the same binary and the same quiet machine.

## Consequences for the design

1. **Batch 4–64 glyphs per analysis.** Bit-identical, ~1.8x on exactly the cold path that hurts, cheap
   to implement. Do **not** batch a whole viewport — past 256 it is a pessimisation.
2. **`SPEC.md` §3.2's "rasterisation off the paint path" requirement survives, but its justification
   changes and should be re-scoped.** The honest figure is ~86–108 µs/glyph **the first time a
   (glyph, size, rendering mode) is seen on that machine**, persisting across restarts — not per
   viewport. A genuinely cold 1,500-glyph viewport measured **162.5 ms**, still 8–10 frames, so
   placeholders and fill-in-over-later-frames remain a v1 requirement. But steady state is 1,500
   glyphs in **4.4 ms**, comfortably inside one frame. This is a first-run-experience requirement, not
   a permanent tax, and the spec should say which.
3. **The atlas should assume it is competing with a cache it does not control.** Tailhawk's own atlas
   and the system font cache hold the same glyphs. For a Latin log viewport — a few hundred distinct
   glyphs — both fit trivially and rasterisation is not a cost worth designing around. The 8k–16k
   ceiling only threatens a CJK-heavy viewport with a very large distinct-glyph working set.
4. **No re-costing is owed on batching.** It was ranked the highest-value remaining experiment on the
   strength of a possible order of magnitude. The order of magnitude is not there; 1.8x is.

## Traps found, worth not rediscovering

- **⚠ The font cache is cross-process, so "run it again" is not a repeat measurement — it is a warm
  one.** This experiment's first version reported 2.5 µs/glyph and concluded G4 was wrong by 150x. It
  was measuring cache hits left behind by its own previous run. Any DirectWrite rasterisation figure
  is meaningless unless the process is the first to touch those (glyph, size) pairs since the cache
  last lost them.
- **An in-process cache probe cannot see it.** Five identical passes gave pass0/pass4 = 0.99x, which
  reads as "no cache" and is worthless — the section before it had already warmed every glyph, and the
  cache outlives the process anyway. The probe is retained in the binary with that warning attached,
  because misreading it is what produced the wrong answer the first time.
- **A reboot is not a neutral cold start for anything that draws text.** It empties the font cache
  service, which makes post-reboot text rendering several times slower than a quiet warm machine. Any
  "cold set" that involves glyphs is measuring cache state, not machine load.
- **Derive the atlas cell from measured bounds, never from em size.** Guessing 20×26 for em 14 clipped
  1,086 of 1,500 glyphs. The clipped cells then compared *equal* between arms, which would have been
  read as a correctness pass. Assert containment (`overflow == 0`) before believing any comparison.
- **Vary em to get a cold measurement without a reboot.** The cache key includes size, so em 20, 21,
  22 … are independent cold populations of the same glyphs. This is what makes the capacity sweep
  possible in one process on a machine whose cache history is unknown.

## Caveats — read before quoting these numbers

- Single machine, single font (Microsoft YaHei), CJK only. Latin faces have far smaller distinct-glyph
  counts and were not measured; the mono path only.
- The colour path (`TranslateColorGlyphRun`) was not measured at all. G4 established colour glyphs are
  rare and pictorial; whether they share the same cache is unknown.
- Main figures at em 14; the cold batch sweep at em 17/18 and the capacity sweep at em 20–25. Per-glyph
  cost scales with em, so the µs figures are not comparable across those sections — only the ratios.
- The 8,000–16,000 capacity is a bracket from six points, not a measured boundary, and is very likely a
  byte budget rather than a glyph count.
- `FontCache` as the mechanism is inferred from cross-process persistence, not confirmed by stopping
  the service.
- `GetAlphaTextureBounds` timings are near zero for large batches simply because there is one call per
  batch; that column is not evidence about the cost of bounds queries.
