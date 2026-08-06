# Handoff — resume here

## ✅ The shaping bridge is built — `shape.rs`, 2026-08-06, session 13

`crates/tailhawk-core/src/shape.rs`. **199 tests, all passing** (198 core + the shell's one), fmt and
clippy clean, CI green on x64 and ARM64. **The V2/V3 gap is closed.** `glyphs.rs`'s test-only
`glyphs_for` — one code point, one glyph — now has a real replacement: `Shaper::shape` takes a line
and a face and returns, per grapheme cluster, which glyph ids draw it. **V3 may be built on this.**

**It holds no device and no window**, like `raster.rs`, so every test runs in-process. The call
sequence is `AnalyzeScript` + `AnalyzeBidi` → `GetGlyphs` → `GetGlyphPlacements`, and the module
authors both COM callbacks (`IDWriteTextAnalysisSource` / `IDWriteTextAnalysisSink`) as one object.

### ⚠ `GetGlyphs` returns glyphs in *logical* order, even with `isRightToLeft = true`

**This was written from memory the other way round and two tests failed.** The obvious guess is that
a right-to-left run comes back already reversed. It does not — the array is in logical order and
`clusterMap` still rises. **Visual reordering is the drawing code's job, and V3 owes it.**

The discriminator is worth keeping: **alef joins to its right but not its left**, so `ابب` is
unambiguously `[alef, beh-initial, beh-final]` logically and the reverse visually, and alef's glyph
is identifiable from the `.cmap`. It came back at index 0. `directwrite_returns_glyphs_in_logical_order`
pins it; `an_arabic_run_resolves_to_a_right_to_left_bidi_level` proves the run really was marked RTL,
so the finding is about DirectWrite rather than about a run that was never flagged.

`glyph_range` deliberately does **not** depend on the answer: it reads the map's direction from the
map. That is why the fix was a doc correction rather than a rewrite.

### 🔍 The adversarial review found two real defects, and both were live bugs

The owner's standing condition, and it paid again. Neither was caught by the 25 tests that existed.

| Defect | Effect |
|---|---|
| **`glyph_range` asked only whether the cluster's *first* position was shared** | The two clusterings do not nest. A grapheme cluster whose first position was swallowed by a preceding ligature but whose *later* positions open a DirectWrite cluster of their own was declared absorbed, and **its glyph was claimed by nobody and never drawn**. `\u{200B}\u{FE0F}\u{E0067}` is a real line that does it — and the dropped glyph was a **tag character**, the hidden-text vector §13.4 names and §5.6 forbids discarding silently. Fixed: ownership is decided by where a DirectWrite cluster *opens*. |
| **`snap_to_cluster` moved a straddling boundary *backwards*, which deleted it** | A cluster's start is usually already a cut, so the moved boundary collapsed onto it and vanished — and the run then swallowed everything to the next surviving cut. `بि` is one cluster (`U+093F` is `Mc`), so `بिक्षि tail` shaped the **whole Devanagari word as right-to-left Arabic**: conjunct ligature lost, matra not reordered, cluster **77% wider** — column drift against §3.3's own acceptance test. It reached past the script too: `GET /a بिक्षि 200` shaped the trailing `200` as Arabic. **Six attacker-supplied bytes changing the rest of the line is exactly what §13.4 calls a real defect.** Fixed: boundaries snap *forward* to the cluster's end, a position no earlier cut can occupy. |

Two more findings were about honesty and are corrected: the doc for `glyph_range`/`ClusterGlyphs`
stated the buggy rule as if it were the right one, and **`every_glyph_is_claimed_by_exactly_one_cluster`
did not test the property it claimed to** — it drove only one-code-unit clusters, and multi-unit
clusters are the entire reason shaping exists. It now sweeps every monotonic map of up to four
positions, rising *and* falling, crossed with **every partition of those positions into clusters**;
that cross product is what surfaced the dropped glyph. A `u32` truncation guard on absurd line
lengths and one genuinely dead filter went with them.

**Both fixes were verified by reverting them**: restoring the old absorbed rule fails 3 tests
including the real-text sweep; restoring backward snapping fails 3 including the Devanagari one.

### Seven negative controls on the original, each applied, observed and reverted

| Mutation | Result |
|---|---|
| ligature "absorbed" guard removed | 2 fail — both clusters claim one glyph |
| neighbour search made forward-only (the left-to-right assumption) | 2 fail |
| forward scan past equal `clusterMap` entries dropped | 2 fail |
| UTF-16 length replaced by byte length | **8 fail** |
| `E_NOTIMPL` returned from `GetLocaleName` | **9 fail** — every DirectWrite test |
| wrong HRESULT compared in the retry loop | 1 fail |
| forward scan started at `start+1` rather than `start+len` | **did not fire** |

**The one that did not fire changed the code, again.** It was a no-op on every fixture, but it is not
redundant: a grapheme cluster DirectWrite splits into two of its own gives a rising map *inside* one
cluster, and the two forms differ there. `a_cluster_directwrite_splits_still_takes_all_of_its_glyphs`
is that case, and the control fires with it present. Same shape as `raster.rs`'s `tighten` — **a
control that does not fire is a statement about the fixtures.** It is now three for three.

### Two claims that were measured rather than asserted

- **Which source callbacks actually run.** Instrumenting all five showed `AnalyzeScript`/`AnalyzeBidi`
  calling `GetTextAtPosition`, **`GetLocaleName` and `GetParagraphReadingDirection`** — and *not*
  `GetTextBeforePosition` or `GetNumberSubstitution`. `E_NOTIMPL` from the latter two is harmless;
  from `GetLocaleName` it fails the whole analysis. The comment now says only that.
- **The retry loop is exercised, not assumed.** `shape_from` takes the first buffer guess, so
  starving it to one glyph drives the real `ERROR_INSUFFICIENT_BUFFER` path — the same trick
  `rasterise_in_batches` uses for batch size. A bounded-growth test covers the give-up path too.

### What shaping still owes

- **Visual reordering for right-to-left runs.** Established above as the caller's job; nothing does
  it yet. V3.
- **Font fallback.** `shape` takes one face and a codepoint it lacks shapes to `.notdef`, exactly as
  `Face::glyph_indices` behaves. Noticing that and trying the next face is the caller's, and
  `GlyphKey` is keyed by face precisely so a fallback glyph cannot collide.
- **The locale is fixed at `en-us`.** It selects language-specific forms (`locl`); a log file carries
  no language tag, so a stable wrong answer beats a guessed one — but it is a limitation, not a
  decision.
- **Nothing consumes `Shaped` yet**, so `advances` and `offsets` are produced and unused. They are
  what places marks *within* a cluster; §3.3 still gives the cell grid the final say on where a
  cluster starts.

**⚠ Static `tailhawk.exe` is 506,368 bytes — byte-for-byte the previous figure, and it means
nothing.** Nothing in `crates/tailhawk` references shaping, so LTO strips the whole module. Session 8
recorded this trap and it still applies: **the real size cost lands at V3**, when the grid references
the atlas and the shaper together.

**CLEANROOM needs nothing.** Session 12 filed the §5 entry (`04cd386`) *before* the code, which is
what §1.5 asks and what had slipped three times. No source outside the repo was consulted this
session at all — the ordering finding came from a probe against the running DirectWrite, not a page.

### ▶ Resume here: V3, the grid

Everything V3 needed from V2 now exists. The two things `SPEC.md` §3.3 still names, unchanged from
session 11's note below: the **u64 scroll model, hit-test and selection**, and the
**`max_byte_len` / `all_ascii` extent fields captured during the index scan** — §5.3 says they are
free there and recovering them later means a second pass over 10 GB. `index.rs` has neither field.
Do it *with* V3's scrollbar; do not let V3 ship without it.

---

**Paused:** 2026-08-06, session 13. **M1 and M2 are complete** — E3, E4 and E8 all done, CI green on
both architectures. Only M2's two large-fixture done-criteria remain unrun. **M3 is under way: V4
(the cell model), V1 (the device), V2's monochrome path and the shaping bridge are all done; V3, the
grid, is next and nothing blocks it.** `PLAN.md` marks M3 the highest-risk milestone, with a
stop-and-reconsider gate at >50% overrun. **Everything below is on disk and pushed. Nothing is
held in a chat session.**

---

## ✅ V2's mono path is joined up end to end — 2026-08-06, session 11

`crates/tailhawk-core/src/glyphs.rs`. **171 tests, all passing**, fmt and clippy clean. Glyph id in,
quad out: `GlyphCache` owns the rasteriser, the atlas, the sheet and the face, and
`real_text_reaches_real_pixels` drives "Hi" all the way from a code point to dark pixels on light
paper in an offscreen target. **The monochrome half of V2 works.**

**The rule it exists to enforce is `SPEC.md` §3.2's *a frame must never block on rasterisation*, and
it is enforced structurally rather than by discipline.** `quad` cannot rasterise — it has no way to.
A glyph that is not resident returns a **placeholder** quad and joins a queue; `flush_misses` does
the rasterising, and the caller runs it **after presenting**. That ordering is the whole requirement:
a genuinely cold 1,500-glyph viewport is 162 ms, 8–10 frames, because DirectWrite's cross-process
font cache is empty for those glyphs the first time a machine ever draws them.

**The placeholder is a hollow box, and it lives outside every slot the atlas can hand out.** The
sheet is 1024×1024 but the atlas is told it is one cell-row shorter, so the reserved cell cannot be
evicted no matter how hard the atlas thrashes — which is what stops a full sheet from having nothing
to draw a miss with. A hollow box rather than a solid block because a screen of solid blocks reads as
corruption while a screen of outlines reads as loading, and it is what a terminal already shows for a
glyph it cannot render.

**Three negative controls, each applied, observed and reverted:**

| Mutation | Result |
|---|---|
| the atlas is given the placeholder's row | **all 6 fail** — and loudly, because `Sheet::upload` refuses the out-of-sheet write rather than corrupting a slot. The reserved row is load-bearing |
| a no-ink glyph is not recorded as blank | `a_space_is_recorded_as_blank_and_stops_being_a_miss` fails — G4's 440-misses-per-frame bug |
| *(the placeholder path, tested by the two assertions in `asking_for_a_glyph_never_rasterises_it`: the placeholder occupies exactly the cell the real glyph will, and the second frame samples a different slot)* | |

**⚠ Shaping is not solved and nothing here pretends otherwise.** ~~The one-code-point-one-glyph
mapping used by the tests is `glyphs_for`, confined to the test module **on purpose**, with a comment
saying it is wrong for Devanagari, Arabic and anything else §3.3 names. **V3 must not be built on
it.**~~ — **solved in session 13 by `shape.rs`; see the top of this file.** `glyphs_for` is still
test-only and still wrong, and still must not be used; `Shaper::shape` is what V3 builds on. What
`GlyphCache` takes is a glyph id, which was already the right currency — deciding *which* glyph ids
draw a cluster is what the shaper now answers.

**Still owed by V2:** the colour path through `TranslateColorGlyphRun` into a second sheet; §3.2's
per-DPI rebuild (`Atlas::clear` exists for it, nothing calls it, and `GlyphCache` has no way to change
`px_per_em` yet); re-priming the placeholder after a device rebuild (`prime` is separate from `new`
precisely so it can be, but nothing wires it to the recovery path); and moving rasterisation to a
worker rather than merely after the present.

---

## 🚧 V2's glyph pass draws, and the pixels were read back — 2026-08-06, session 11

`crates/tailhawk-core/src/text.rs`, `sheet.rs`, `shaders/glyphs.hlsl`, plus an offscreen test
harness in `gpu.rs`. **166 tests, all passing**, fmt and clippy clean. `SPEC.md` §3.2's single
instanced draw now exists: one `DrawInstanced` over a structured buffer of glyph quads, with
per-instance foreground colour, so per-token colouring is free.

**The dual-source blend state is real and verified against actual pixels.** `SrcBlend = ONE`,
`DestBlend = INV_SRC1_COLOR`, the pixel shader emitting premultiplied colour in `SV_Target0` and
per-channel coverage in `SV_Target1`, so the hardware computes `dest = c0 + dest * (1 - c1)`. The
trap G4 found is in the code with a comment on it: **the alpha slots take `INV_SRC1_ALPHA`; a
`*_COLOR` factor in an alpha slot fails `CreateBlendState` with a bare `E_INVALIDARG`.**

**Everything is tested by drawing into an offscreen target and reading the pixels back** — no window
and no swapchain, via `gpu::offscreen`. That is the only way to know: every call can succeed while
producing the wrong composite, and a flip-model back buffer is undefined after `Present`.

### ⚠ The ClearType test passed for the wrong reason, and the negative control is what caught it

The first version drew a **light tint over a dark backdrop** and asserted channel spread. It passed —
and it **also passed with the blend rewired to a single alpha**, which is exactly the state the test
exists to reject. The spread was coming from `c0.rgb = tint.rgb * cov`, the *source* term, which is
per-channel no matter what the destination factor does.

The arrangement that discriminates is **black ink on light neutral paper**: with `tint.rgb = 0` the
source contributes nothing, so every channel of the result comes from `dest * (1 - c1)` alone.
Rewired to a single alpha it now returns `[109, 109, 109]` — perfectly neutral, the collapse made
visible. It also asserts the *ordering*, not just the spread: the channel given the most coverage is
attenuated most, so it comes back darkest.

This is the second time this session that **a control which did not fire changed the code rather than
being written off** (the first was the rasteriser's cell extent). It is the most useful habit in this
project: a test that passes tells you nothing until you have watched it fail.

| Mutation | Result |
|---|---|
| `DestBlendAlpha` set to `INV_SRC1_COLOR` | **all 7 fail** — `CreateBlendState` rejects it outright, which is G4's documented trap |
| `DestBlend` set to `INV_SRC1_ALPHA` (one alpha, not three coverages) | the ClearType test fails — **only after it was rewritten**; the original passed |
| `Instance` fields reordered, same size | 2 fail. Same-size layout drift is the real hazard: a structured buffer is read as raw bytes, so it does not fail to compile, it draws nonsense |

### Four decisions worth not re-litigating

- **The device now demands feature level 11_0** and nothing lower. The glyph pass reads its quads
  from a `StructuredBuffer` in the *vertex* shader, which is shader model 5; on a feature-level 10
  device the shaders simply fail to create, and there is no text. Better to fall to WARP, which is
  always 11_0. **11_1 is deliberately not requested** — a runtime that does not know it rejects the
  whole array with `E_INVALIDARG` rather than skipping the entry.
- **The sheet is `D3D11_USAGE_DEFAULT` with `UpdateSubresource`, not `DYNAMIC` with `Map`.** A
  dynamic texture must be mapped `WRITE_DISCARD`, which throws away every glyph already resident.
  The whole point of an atlas is that they stay.
- **Point sampling, clamped.** A cell is addressed in whole texels at the scale it was rasterised
  for, so filtering can only blur it — and bleed the neighbouring slot's ink in at the edges.
- **The colour sheet is not created yet, and `t2` is left unbound on purpose.** An unbound SRV
  samples as zero, so a stray `MODE_COLOUR` instance draws nothing rather than garbage — which a
  test asserts. Creating an empty 4 MB texture nothing writes to would be worse.

**Static `tailhawk.exe` is 506,368 bytes** — still 3.2% of §11.2's 15 MB gate, and still not a
measurement of the atlas, because nothing in the shell references it yet. The build runs and comes up
on the hardware rung with feature level 11_0 enforced.

---

## 🚧 V2 has started — the atlas allocator and the rasteriser, 2026-08-05, session 11

`crates/tailhawk-core/src/atlas.rs` and `crates/tailhawk-core/src/raster.rs`. **159 tests, all
passing** (158 core + the shell's one), fmt and clippy clean. Between them: which glyph lives in
which cell, which cell is given up when the sheet is full, which glyphs have no ink, and what the
pixels of a cell actually are. **Neither is wired to a device yet** — the sheets, the upload and the
instanced draw are the next thing to build.

**Nothing here was designed this session.** Every rule is a conclusion of `g4-glyph-atlas` or
`g4b-batched-raster`, and the module cites the measurement beside each one:

| Rule | The measurement behind it |
|---|---|
| Uniform slots, one glyph each | A variant allowing a glyph to span adjacent slots cost **106 ms/frame** — the victim search became O(capacity × span) and had to find a free *run* |
| O(1) intrusive victim list | **4–9 ms/frame** scanning for the oldest against **0.17–0.37 ms** for the list. 20–28x in every run |
| A slot touched this frame is never evicted | Otherwise the frame overwrites a cell it is about to draw from |
| Cache the absence of ink | G4's first run: exactly **440 spurious misses per frame** from ten spaces per row |
| The slot size comes from measured bounds, never from em | G4b guessed 20×26 for em 14 and clipped **1,086 of 1,500** glyphs — and the clipped cells then compared *equal*, which reads as a correctness pass |

**Vacant slots start at the head of the LRU list**, so allocation and eviction are the same operation
and neither needs a search or a separate free list. The head is either an empty cell or the oldest
resident; if the head was used this frame then every slot was, and `insert` reports
`SheetFullThisFrame` rather than corrupting the frame.

**Three decisions the experiments did not settle.**

- **The key carries the face and a synthetic-style flag**, not just §3.2's "(glyph id, style, dpi
  scale)". Glyph ids are face-local, and font fallback (§3.3) means one viewport draws from several
  faces at once — glyph 42 of the CJK fallback is not glyph 42 of the primary. Bold and italic are
  normally separate faces and so are already covered; what is not is a *synthetic* bold or oblique
  applied to the same face, which is the same glyph id at the same size with a different raster.
- **The size in the key is whole device pixels**, not points and not a scale factor, because §3.2
  requires integer device-pixel advances and a float key could hold two rasters that round to one
  cell.
- **The blank set is bounded and dropped wholesale when it overflows.** A blank costs no slot, so
  nothing else bounds it, and a viewer left open for days on logs full of unusual codepoints would
  grow the map for ever. Rebuilding a blank costs one miss, not a rasterisation, so it does not earn
  an LRU of its own.

**Three negative controls, each applied, observed and reverted:**

| Mutation | Result |
|---|---|
| the never-evict-this-frame guard removed | `a_slot_used_in_this_frame_is_never_evicted` fails |
| `lookup` no longer touches — i.e. FIFO, not LRU | `the_glyph_nobody_asked_for_is_the_one_evicted` fails |
| eviction forgets to drop the victim's map entry | **3 tests fail**, including the thrashing one, where two keys end up pointing at one slot |

### The rasteriser — `raster.rs`, mono only

`Rasteriser` holds an `IDWriteFactory2` and needs **no device and no window**, which is why all of
this is testable in-process. `resolve` walks a candidate family list; `measure_cell` derives the
uniform cell from measured bounds; `rasterise` fills one cell per glyph as RGBA — **R, G, B are the
three ClearType coverages and A is their average**, so one sheet serves subpixel and greyscale at no
extra memory (G4).

**`BATCH = 16`**, inside G4b's measured 4–64 window and well clear of the 256 mark where the larger
intermediate bitmap makes batching *slower* than not batching.

**G4b's headline finding is now a regression test in the product**:
`batched_rasterisation_is_bit_identical_to_one_glyph_at_a_time` compares batch 4, 16 and 64 against
one-glyph-at-a-time over printable ASCII. Grid fitting and the ClearType filter are both
position-dependent, so this could legitimately have been false; if a future DirectWrite makes it
false, batching has to go, and that test is what will say so.

**A glyph with no ink is `None`, decided by inspecting the cell rather than the strip.** G4b detected
blanks per *chunk*; within a non-empty strip an individual space is an all-zero cell, and missing it
is G4's 440-spurious-misses-per-frame bug.

**Two negative controls fired; one did not, and that one changed the code.**

| Mutation | Result |
|---|---|
| the run advance no longer matches the slicing step | the bit-identical test fails |
| empty rectangles are allowed into the measured extent | 2 tests fail — **but only after the extent fold was extracted** |

The extent fold is now the free function `tighten`, tested on synthetic rectangles, because **on the
faces this machine has, the mutation was a no-op**: an empty `RECT{0,0,0,0}` already sits inside the
ink extent of printable ASCII, so including it changes nothing and no test over real glyphs can tell
the two behaviours apart. It matters on a face whose ink box does not contain the origin, and since
the cell origin *is* every glyph's placement offset, getting it wrong shifts the whole grid. Worth
remembering as a shape: **a control that does not fire is a statement about the fixture, not about
the code.**

**⚠ No size figure is quoted for the DirectWrite dependency, deliberately.** Nothing in
`crates/tailhawk` references the atlas or the rasteriser, so LTO strips both and the exe would
measure unchanged. Session 8 recorded exactly this trap when `encoding_rs` first went in. **The real
cost lands when the grid references them**, which is V3.

**⚠ What V2 still owed at the time of this section** — the sheet, the blend state and the draw
have since been built; see the section above. Left after that: the colour path and the join. ~~the
two sheets and the upload; the dual-source blend state and the instanced draw
(`SrcBlend = ONE`, `DestBlend = INV_SRC1_COLOR`, and **the alpha slots take `INV_SRC1_ALPHA` or
`CreateBlendState` fails with a bare `E_INVALIDARG`**); the colour path through
`TranslateColorGlyphRun`; and the placeholder-and-fill-in-later behaviour that keeps rasterisation
off the paint path.~~ **Read both `RESULTS.md` files before writing any of it** — G4's absolute
µs/glyph figures are cache-thrash figures and are superseded by G4b.

---

## ✅ V1 is done — device-removed recovery, 2026-08-05, session 11

`crates/tailhawk-core/src/gpu.rs`. **136 tests, all passing** (135 core + the shell's one), fmt and
clippy clean. The device, swapchain and WARP chain were already M0's; what V1 owed was the recovery,
and `SPEC.md` §3.2 states it as a prohibition — "**Never panic on device-removed** — that is the
exact bug that crashes wgpu apps on driver auto-update."

**A lost device is now rebuilt inside `Renderer::paint`, and the shell is never told.** A driver
auto-update, a TDR, a GPU reset, unplugging an external monitor, a laptop switching between its
integrated and discrete GPUs — all of them arrive as the same four `DXGI_ERROR_DEVICE_*` codes, none
is a program error, and each is answered by building a new device and redrawing the frame.

**The policy is separated from the D3D calls so it can be tested without a GPU**, and it has one
subtlety worth not re-deriving:

| Rule | Why |
|---|---|
| First loss retries **the same rung** | A driver update leaves the hardware perfectly able to host the next device. Dropping to WARP over one blip trades the GPU away for nothing. |
| Second loss with no clean frame in between **pins WARP** | A device that dies again immediately is not a blip, and WARP cannot be removed by a driver. |
| **Only a frame that needed no recovery clears the streak** | Otherwise a device that dies and is rebuilt on *every* frame resets the count each time and rebuilds for ever. |
| Three rebuilds in one unbroken streak, then **give up** | Rebuilding a D3D11 device on every `WM_PAINT` is worse for the user than a static window. Giving up returns `Err`; the shell drops to the flat background it painted before a device existed, which is still not a panic. |

**The DXGI factory is rebuilt with the swapchain rather than cached, and that is the actual trap.** A
factory created before a device was removed is stale; a renderer that reuses it comes back
"recovered" while presenting to nothing. `present()` also *fails* rather than silently succeeding
when a window is attached but no swapchain exists — which is precisely the state a rebuild that
forgot to re-attach leaves behind.

### How something D3D11 gives you no way to trigger got tested anyway

D3D11 has no `RemoveDevice` — D3D12 does — and the real causes are a driver update or a TDR, neither
of which a test can arrange. So a `test-hooks` feature injects the `DXGI_ERROR_DEVICE_REMOVED`, and
**everything after that point is the real path**: the same classification, the same policy, a real
device rebuild, a real `Present`. Under resolver 2 a feature turned on by a dev-dependency does not
reach `cargo build`, and `cargo tree -e normal` confirms the shipped binary does not carry the hook.

**The swapchain half of recovery is tested from the *shell* crate, not the core.** The core may not
own a window (§3.1), so its own device tests rebuild with nothing attached — and a rebuild that drops
the swapchain passes all of them. `crates/tailhawk/src/main.rs` has the one test that attaches a real
hidden window, loses the device, and then resizes and presents again.

**Three negative controls, each applied, observed and reverted:**

| Mutation | Result |
|---|---|
| `rebuild` does not re-create the swapchain | **Only the shell test fails.** All 135 core tests pass. That failure is the entire justification for testing this from the shell. |
| escalation removed (always retry the current rung) | 2 tests fail, including the one that checks a *real* rebuilt device landed on WARP |
| the clean-frame reset removed | `losses_separated_by_clean_frames_are_recovered_from_indefinitely` fails — the realistic case, a machine that loses its device occasionally over a long session |

**One from-memory constant was written and deleted before commit.** The first draft carried
`D3DDDIERR_DEVICEREMOVED = 0x88760870`, recalled rather than looked up. Grepping both installed
Windows SDKs found no such symbol, so it went. The four DXGI codes are each taken from the `windows`
crate's generated bindings, and `was_lost` asks `GetDeviceRemovedReason()` as well — which covers a
call that fails with some other code while the device is being removed underneath it, and is why the
exotic constant bought nothing anyway. Same failure mode as E8's severity tables; caught before
commit this time rather than by review.

**Two smaller things came with it.** `Renderer::driver()` is now live rather than fixed at startup —
recovery can move the device to WARP and the title bar is the only place this build says which rung
it is on, so the shell re-reads it after each paint. And `#![windows_subsystem = "windows"]` is now
`cfg_attr(not(test), …)`: a GUI-subsystem test harness has nowhere to print.

**Static `tailhawk.exe` is 505,856 bytes**, against 502,784 at M1 and V4. Still 3.2% of §11.2's
15 MB gate. Smoke-tested: the static build opens, comes up on the hardware rung, and titles itself
`Tailhawk — hardware`.

---

## 🚧 M3 has started — V4, the cell model, 2026-08-05, session 10

`crates/tailhawk-core/src/cell.rs`. **125 tests, all passing**, fmt and clippy clean.

**M3 is `PLAN.md`'s highest-risk milestone** (11 weeks, with a stop-and-reconsider gate at >50%
overrun), so it starts with the piece that is portable, headless and needs no device: §3.3's cell
model. V1's device work is largely M0's already; V2 (the atlas) and V3 (the grid) both sit on top of
this arithmetic, and §3.3's acceptance test — *columns line up* — is decided here rather than in
DirectWrite.

**The rule: delegate to `unicode-width`'s cluster-aware `UnicodeWidthStr::width`, one cluster at a
time.** It already handles ZWJ sequences, regional-indicator pairs, keycaps, presentation selectors,
conjoining jamo and spacing marks. Exactly two things are layered on top: a control character takes a
visible cell (§5.6 forbids dropping it), and an emoji-modifier sequence is two cells rather than the
2 + 2 `unicode-width` gives it.

**⚠ The first version of this file hand-rolled that algorithm and was wrong four ways** — see the
review section below. The reason is worth keeping: the premise was that a cluster's width is its
*base's* width, on the theory that anything cluster-aware must be summing code points and would call
`👨‍👩‍👧‍👦` eight cells. `UnicodeWidthStr::width` in `unicode-width` 0.2 does not sum — it segments — and
that premise cost four defects, three deleted special cases and a test that asserted the wrong
answer for a script §3.3's own acceptance test names.

**Two dependencies were added**, `unicode-segmentation` and `unicode-width` — pure Rust, no C
toolchain, Apache-2.0 OR MIT, so `deny.toml` admits them without a new entry. The alternative is
shipping and then ageing our own copy of the Unicode character database, which is not a maintainable
position for a project that expects to see CJK, Devanagari, RTL and emoji in real logs.

**§13.4's "reveal invisibles" toggle is a width question, not only a painting one** — a revealed
zero-width space occupies a cell — so `CellModel` carries the flag. That is also what makes a Trojan
Source `U+202E` override visible rather than merely isolated.

**⚠ But §13.4 cannot yet be claimed, and the gap is asserted rather than hidden.** The toggle reveals
an invisible that forms its *own* cluster — `U+200B`, `U+202E` — and **not** one absorbed into a
preceding cluster: `a` + ZWJ, `a` + a tag character (`U+E0067`, the hidden-text vector), `a` + a
variation selector. Closing it needs `General_Category` data to tell a `Cf` character from a
legitimate combining mark, which must *not* be revealed, and probably needs reveal mode to segment
differently rather than only measure differently.
`revealing_does_not_yet_reach_an_invisible_inside_a_cluster` pins the current behaviour so the gap
stays visible.

### V4's review found six defects, and the two hit-test ones were the dangerous pair

| Defect | Effect |
|---|---|
| **A stray `U+FE0F` was treated as "make this two cells"** on *any* base | VS16 means something only on an `Emoji=Yes` base. **928,985** `<base> U+FE0F` clusters were over-counted. Appending three bytes anywhere in a line shifted every column to its right — attacker-controllable, and §13.4 calls a viewer that can be made to lie a real defect. |
| **`U+FE0E` likewise, under-counting** | A wide ideograph followed by a stray VS15 got one cell and painted over its neighbour. 182,716 under-counts. |
| **Multi-jamo Hangul** | `ᄀᄀ가` is one cluster of four cells; base-width said two. |
| **Devanagari and Thai spacing marks** | `कि` is two cells, not one. §3.3's "combining marks occupy 0 additional cells" is true of `Mn`/`Me`, **not** of `Mc` — and a *test of ours asserted the wrong answer*, for a script §3.3's acceptance test names by name. |
| **A zero-width cluster stole the next cluster's column** in `byte_at_cell` (`width.max(1)`) | 57 of 104 visible clusters hit-tested to the wrong byte. `"\u{202E}abc"` — §13.4's own Trojan Source example — returned the override's offset for a click on `a`. A caret placed from a click did not sit where the user clicked. |
| **Reveal-invisibles misses invisibles inside a cluster** | Documented above rather than fixed. |

**The reviewer verified all of it empirically** — a probe crate outside the repo, on the same locked
crate versions, sweeping every scalar value and fuzzing 500,000 strings — rather than reasoning about
Unicode from memory. That is the difference between this review and a plausible-sounding one, and it
is worth insisting on for anything touching Unicode.

It also confirmed **no panic, no non-char-boundary result, and `cell_count <= line.len()` across
every scalar value and both fuzz corpora** — which is the property §3.3's `max_byte_len` upper bound
for the scrollbar depends on.

**⚠ The binary did not grow, and that figure means nothing yet.** The static `tailhawk.exe` is
**502,784 bytes — byte-for-byte the M1 figure**, with two Unicode-table crates newly linked. That is
LTO stripping code the *shell* never references: nothing in `crates/tailhawk` calls the cell model,
the record model or the indexer. Session 8 recorded exactly this trap when `encoding_rs` first went
in, and it applies to everything M2 and M3 have added so far. **The real size cost of the Unicode
tables lands the moment the grid references them**, and that is the measurement to take at V3 — not
this one.

**CI is green on all three of session 11's commits** — `6d38a0e` (V1), `51a1bcf` (the allocator) and
`d66d551` (the rasteriser), runs `31011208042`, `31012052003` and `31013496467`, x64 and **ARM64**
both. DirectWrite cross-compiles to ARM64 without complaint, and the runner ran the same 158 + 1
tests this machine does.

### ✅ The device and font tests really do run on the CI runner — checked, not assumed

**The `Test` step runs with `-- --nocapture`, and that is load-bearing.** Several tests skip loudly
rather than fail when there is no D3D11 device, no window station or no usable font — but `cargo
test` captures the output of *passing* tests, so without `--nocapture` a skip and a real run are
indistinguishable in the log and a skipped device test reads as a green one. **A green device test is
not evidence the device path ran** unless the skip message is absent.

Run `31013808780` (`06005e0`) is the first with the messages visible: **no `SKIPPED` line anywhere in
the log**, against 8 `test result: ok` lines from the same fetch, so the log was genuinely read. So
the GitHub `windows-latest` runner has a D3D11 device, a window station and one of the candidate
faces, and it exercised device-removed recovery with a real swapchain and rasterised real glyphs.
That was an open question — the tests were written to degrade rather than fail precisely because a
headless runner was a real possibility.

**This paragraph replaces a claim I made without checking.** Session 11 first asserted the runner had
not skipped them, on the strength of the test *counts* — which cannot show it either way, because a
skipped test still counts as passed. The counts were right and the inference was not.

### ▶ Resume here: V3, and the shaping gap in front of it

~~V1 still owes device-removed recovery~~; ~~V2 owes the sheets and the draw~~; ~~nothing connects the
four pieces~~ — **V1 is done and V2's mono path is joined up end to end**, all session 11. See the
sections at the top of this file.

**V3 is the grid: the u64 scroll model, hit-test and selection, plus §3.3's `max_byte_len` /
`all_ascii` extent fields.** Two things to settle before writing much of it:

1. ~~**Cluster → glyph ids.** `GlyphCache` speaks glyph ids and `cell.rs` speaks grapheme clusters,
   and nothing bridges them.~~ — **done, session 13.** `shape.rs` bridges them via
   `IDWriteTextAnalyzer`. What V3 inherits from it: **glyphs come back in logical order even for
   right-to-left runs, so V3 owes the visual reordering**, and font fallback is the caller's job.
2. **The extent fields must be captured during the index scan** — §3.3 says they are free there, and
   the reason is that recovering them later means a second pass over 10 GB. `index.rs` has neither
   field. Do it *with* V3's scrollbar, but do not let V3 ship without it.

**And the size question finally becomes answerable at V3.** Every figure quoted since M2 has been
LTO stripping code the shell never references. The moment the grid references the atlas, the real cost
of `encoding_rs`, the two Unicode-table crates and DirectWrite all land at once.

- **V3 (grid)** owes §3.3's `max_byte_len` / `all_ascii` extent fields. **These must be captured
  during the index scan** — §3.3 says they are free there, and the reason is that recovering them
  later means a second pass over 10 GB. `index.rs` has neither field yet. Do this *with* V3's
  scrollbar, not before, but do not let V3 ship without it.
- **V2 (atlas)** is the higher-risk half and has the most prior art already measured:
  `experiments/g4-glyph-atlas/RESULTS.md` and `experiments/g4b-batched-raster/RESULTS.md` settle the
  eviction policy, the batching win (~1.8x, saturating at 4) and the cross-process font-cache
  behaviour. **Read both before writing any rasterisation code** — G4's absolute µs/glyph figures are
  cache-thrash figures and are superseded by G4b.

**Out of scope for this file, deliberately:** font fallback and the colour-glyph atlas are V2;
§3.3's `max_byte_len` / `all_ascii` horizontal-extent rule belongs with V3's scrollbar, because it is
the scrollbar that needs a conservative upper bound before layout has run. `cell_count_never_exceeds_byte_length`
is the test that pins the property that rule depends on.

---

## 🚧 E8 is done — the record model, 2026-08-05, session 10

`crates/tailhawk-core/src/record.rs`. **106 tests, all passing**, fmt and clippy clean.

The OTel-shaped record of §6.1, the severity banding of §6.2, and the tables every format detector
(E9/E10) will resolve through. Nothing here parses anything — this is the shape the parsers fill.

**The severity tables were transcribed from the OpenTelemetry spec, not written from knowledge, and
that decision paid for itself immediately.** The first draft was written from memory and had **syslog
Emergency at 23 with Alert and Critical in the FATAL band** — the appendix says **21, 19 and 18** —
and **`java.util.logging` FINER at 3** where the appendix says **5**, a whole band out. Every one of
those was plausible, confident and wrong, in the one table whose entire purpose is that a WARN means
the same number in every format. `CLEANROOM.md` §5 carries the entry.

| Table | Source |
|---|---|
| syslog (RFC 5424), log4j, Zap, Windows Event Log, `java.util.logging` | **OTel Appendix B, transcribed** |
| HTTP-status aliases (§6.2) | **Ours** — Appendix A describes the Apache access log but assigns it no severity. Opt-in, and labelled as ours in the code. |
| the generic level-word table | Ours, a union of the above plus the abbreviations real writers emit |

**Three decisions worth not re-litigating.**

- **Absent severity is `None`, and zero is not representable.** The OTel spec allows both a zero and
  omission; carrying both would be two spellings of one state. `UI-DESIGN.md` §11.2 renders it blank,
  never INFO.
- **`resource` is deliberately not on `Record`** — §6.1's own field table says it "belongs to the
  pane, not the row", which is what makes §8.3's merged-view column problem answerable.
- **`raw` is decoded text where §6.1 says "the original bytes".** Every consumer works in decoded
  text. But it *is* a loss — U+FFFD does not invert — so `Record::span` carries the byte offset and
  length, and **that** is what keeps §10.2's "copy preserving original bytes" and §10.3's "search and
  copy operate on the untruncated bytes" reachable. `raw` is the convenient form, not the
  authoritative one.

**`verbose` and `critical` are genuinely ambiguous across frameworks** — Serilog's `Verbose` is its
lowest level, Windows Event Log's is DEBUG (5); `Critical` is FATAL (21) in .NET and ERROR2 (18) in
Windows Event Log. Both are resolved against the *normative* table, since Appendix B has a Windows
table and no .NET one.

---

## 🔍 Both components were reviewed adversarially before commit — and the review found real defects

Two independent review agents were run over E4 and E8. **This is the practice the owner asked for on
2026-08-05, and it is the condition attached to working without checkpoints — keep doing it.** It was
not ceremonial; four real defects came out of it, none of which the test suites had caught.

| Found in | Defect | Status |
|---|---|---|
| `indexer.rs` | **A file that shrank after being sized invented a line.** `end` is sampled before the scan, so a copy-truncating writer (§5.5, one of the three rotation modes) leaves the file shorter. A range that read *nothing* still reported one line at `start`, and the trailing-terminator test compared against `end` rather than the bytes actually read, so `a\nb\n` sized at 40 reported **3 lines where there are 2**. | **Fixed**, with `a_file_that_shrank_after_it_was_sized_reports_only_what_is_there`. Reverting the fix reproduces the 3-vs-2 failure. |
| `indexer.rs` | `from + chunk_bytes` overflowed on an absurd caller-supplied chunk size — panic in debug, and in release it wraps to `to < from` and **silently drops the chunk's lines**. | **Fixed** with `saturating_add`, plus a test. |
| `record.rs` | **`java.util.logging` FINER and FINE were a band out** (see above). | **Fixed** against the spec, with the mapping test. |
| `record.rs` | **`level >= Critical` excluded the rows whose level reads `Critical`** — the band name parsed as FATAL (21) while a `Critical` row resolved to 18. §6.2's whole purpose is that this cannot happen. | **Fixed**, and `a_name_means_the_same_thing_however_it_is_resolved` now asserts the invariant for *every* name both resolvers accept. |

Two further findings were about honesty rather than behaviour, and both are corrected: the
`CLEANROOM.md` §5 entry for E8 **had not been filed** while the code claimed it had (§1.5 requires it
*before* the code — this is the second time that rule has slipped, and the entry says so), and
`LineScanner`'s comment about a failed partial match **misdescribed its own mechanism**.

**The reviewer also confirmed, by building an out-of-tree differential fuzzer, that the parallel and
serial indexes agree** — exhaustively over short UTF-8 and UTF-16 strings at every chunk size, thread
count and stride, plus randomised UTF-32. That is the strongest evidence yet for M2's headline
criterion, short of the 4 GB fixture.

---

## 🚧 E4 is done — the parallel indexer, 2026-08-05, session 10

`crates/tailhawk-core/src/indexer.rs`, plus a segment directory added to `index.rs`. **85 tests, all
passing**, fmt and clippy clean.

**Session 9's machine crashed mid-session.** Nothing was lost — the tree was clean and `6b76992` was
already pushed. Worth knowing only because "commit often; the history is the artefact" is what made
the crash a non-event.

| M2 criterion (`PLAN.md` §4) | State |
|---|---|
| **E3** — line index | **Done**, session 9. |
| **E4** — background + parallel indexer, code-unit alignment invariant | **Parallel: done.** `build_index` splits the file into code-unit-aligned chunks, one `LineScanner` per chunk on `std::thread::scope`, merged by prefix sum. **"Background": not done** — it is a synchronous call, see below. |
| **E8** — record model + OTel mapping + severity tables | **Done**, session 10 — see the E8 section above. |
| **Done:** index a 10 GB fixture with bounded memory | **Not run** — needs 10 GB of scratch disk. Peak indexing memory is now *asserted by construction* rather than hoped for: `read_bytes × threads` (2 MB at the defaults) plus the anchors, with **no per-chunk buffer of line starts**. |
| **Done:** 4 GB UTF-16LE on 8 threads is byte-identical to serial | **Run in miniature, not at 4 GB.** `a_real_file_indexes_the_same_on_many_threads_as_on_one` drives a real `LogFile` on all cores over a 20,000-line UTF-16LE fixture with a BOM and misaligned-`0x0A` decoys, and compares serial against parallel line by line *and* against the decoder. The property is tested; the scale is not. |

**CI is green on `87d1cae`** — run `31000029072`, 2m15s, all three jobs: `deny`, x64 and **ARM64**,
with both unsigned artefacts produced. `std::thread::scope` and the indexer cross-compile to ARM64
without complaint.

### `gh` is installed now — stop reading the Actions page in a browser

`winget install --id GitHub.cli`, done in session 10. Authenticated as `nigelbasel`, scopes
`gist`/`read:org`/`repo`.

**A session that has not been restarted since the install will not find it on `PATH`** — use the full
path, which needs the quotes:

```bash
"/c/Program Files/GitHub CLI/gh.exe" run list --limit 5
"/c/Program Files/GitHub CLI/gh.exe" run view <run-id>
```

Two notes for whoever sets this up again. `gh auth login` **exceeds a 120 s tool timeout and lands in
the background** — that is normal, not a hang: it prints a one-time code and then polls until the code
is entered at `github.com/login/device`. Nothing is written to `~/.config/gh` until that happens, so
`gh auth status` reports "not logged into any hosts" for the whole waiting period even though the
flow is working. Logging in to github.com in a browser is *not* the same step as submitting the code.

### The one design decision, and why it went the way it did

**A worker cannot know the global line number its chunk starts at** — that is settled only once every
earlier chunk has finished counting. So a worker anchors from *its own* first line, and the merge
records the base. `LineIndex` therefore grew a **segment directory**: one small entry per chunk,
binary-searched on lookup, and a serially built or followed index is a single segment where lookup
degenerates to the old `n / S`.

**The alternative was rejected on memory.** Workers could instead buffer every line start so the merge
picks the globally aligned ones, which would make the parallel index bit-identical to the serial one.
That costs 8 bytes per line of transient buffer — an 8 MB chunk of *empty lines* is 64 MB per worker,
8 workers is half a gigabyte, against §11.2's 120 MB whole-process claim. Empty lines are not exotic
input. `a_file_of_nothing_but_terminators_indexes_correctly` is the test that stands where that design
would have failed.

`SPEC.md` §5.3 already specified this in "Construction" ("emits anchors at its own local stride. A
prefix sum over the per-chunk line counts converts local anchor numbering to global") — the segment
directory is that sentence made concrete. §5.3's "The structure" has been extended to describe it, and
`CLEANROOM.md` §5 carries the entry §1.5 asks for.

### What "background" still means, and it is genuinely not done

`build_index` is **synchronous**: it blocks until the whole range is indexed. §5.3's **R5** — anchors
published as they are produced, the UI never blocking, the line count a lower bound until the scan
completes — **is not implemented**. Nothing consumes an index yet, so a progressive-publication API
built now would be built blind against a caller that arrives at M3. The pieces are shaped for it: the
merge is already a fold over independent per-chunk results in file order.

### Three tests worth knowing about

- `a_terminator_is_never_split_by_an_aligned_boundary` — the whole parallel scan rests on
  `line_terminator().len() == code_unit()` for every charset, which is what makes a code-unit-aligned
  boundary unable to fall *inside* a terminator. If an encoding ever arrives where that is false,
  chunked scanning needs a carry between workers and this is the test that says so.
- `a_misaligned_0a_survives_every_chunk_boundary` — E3's silent-corruption test, now driven at every
  legal chunk size rather than one.
- `a_real_file_indexes_the_same_on_many_threads_as_on_one` — the only test that exercises `LogFile` as
  a `ChunkReader`. Concurrent positional reads through one handle are a property of `LogFile`, not of
  the indexer, and no in-memory reader can test it.

**Three negative controls were run rather than assumed** — each mutation was applied, the failure
observed, and the mutation reverted:

| Mutation | Result |
|---|---|
| alignment guard disabled, odd chunk size in UTF-16LE | **2 lines found where there are 3** — silent corruption, the exact class `PLAN.md` marks E4 High risk. The guard is load-bearing. |
| first chunk stops owning the file's opening line | 7 of 11 fail |
| the trailing-terminator `pop_line` removed | 7 of 11 fail |

**One thing the negative controls exposed:** `many_threads_and_one_thread_agree_anchor_for_anchor`
**passed under two of the three mutations**. It is a *consistency* test, so it cannot catch a bug both
paths share. The correctness comes from `a_parallel_index_agrees_with_the_decoder_on_the_line_count`
and from resolving every line against an independent scan. Worth remembering before trusting a
serial-vs-parallel comparison to prove anything on its own.

### Deliberate omissions

- **R2 (byte offset → line number) is still not implemented.** `offset_of_line` covers R1; the inverse
  has no caller until "go to offset" and §5.5's re-anchoring after truncation.
- **`memchr` is still absent**, as in E3. The scan is a scalar loop.
- **Truncation and rotation remain M4.** A chunk read that returns zero stops rather than spins, and
  that is all the indexer owes until E7.

---

## 🚧 M2 started — E3 (the index structure) is done, 2026-08-04, session 9

`crates/tailhawk-core/src/index.rs`. **73 tests, all passing**, fmt and clippy clean.

| M2 criterion (`PLAN.md` §4) | State |
|---|---|
| **E3** — line index | **Done.** `LineIndex` (sparse anchors, blocked allocation) + `LineScanner` (terminator matching with cross-chunk carry), per the re-derived `SPEC.md` §5.3. |
| **E4** — background + parallel indexer, code-unit alignment invariant | **Not started.** The scanner is already the right shape for it: it holds no index and no I/O, so one per chunk shares nothing. |
| **E8** — record model + OTel mapping + severity tables | **Done**, session 10 — see the E8 section above. |
| **Done:** index a 10 GB fixture with bounded memory | **Not run** — needs 10 GB of scratch disk. The per-line cost *is* asserted (`the_index_costs_what_section_11_2_says_it_does`, < 0.14 B/line over 1 M lines), so the budget is tested even though the fixture is not. |
| **Done:** 4 GB UTF-16LE indexed on 8 threads is byte-identical to serial | **Not run** — needs E4. |

**Two deliberate omissions.** Truncation/rotation handling is **not** in `LineIndex` — that is M4
(§5.5, E7), and building it now would be speculative. `memchr` is **not** used: the scan is a scalar
loop, which is correct and simple; if the 10 GB criterion or M4's 50 MB/s needs it, optimise then,
with a measurement.

**Three tests worth knowing about:**

- `a_misaligned_0a_is_not_a_terminator_in_utf16` — the silent-corruption class `PLAN.md` marks E4
  **High** risk. `U+4A00` encodes LE as `00 4A`, so a scanner ignoring alignment finds a `0x0A` and
  splits the file in the wrong place. This is the test that will fail if someone "simplifies" the
  scanner to a `memchr` over raw bytes.
- `the_index_line_count_agrees_with_the_decoder` — the index scans *bytes*, the decoder decodes
  *characters*, and they must agree on how many lines exist. Different routes to the same number, so
  it is a real cross-check. Covers the trailing-terminator case where a line start at EOF is not a
  line.
- `line_starts_are_identical_however_the_bytes_are_chunked` — the E4 precondition. If a scan depends
  on where reads landed, a parallel indexer and a serial one disagree.

**`PLAN.md`'s E3 row was stale and is corrected** — it still described "block-sparse, group-varint,
u64 overflow fallback", which the re-derivation rejected.

---

## ✅ M2 is unblocked — the index was re-derived clean, 2026-08-04, session 9

**`SPEC.md` §5.3 has been replaced by a clean-room re-derivation, and the index is cleared to
implement.** `CLEANROOM.md` §4 now reads **RE-DERIVED CLEAN / Yes**, with the §5 entry and the §6
attestation both filed. **M2 can start.**

**The owner's decision was "re-derive clean, replace §5.3"** — not "log the risk and proceed", and
not the strictest quarantine option. Recorded because the cheaper option was genuinely available and
was declined.

**Why it was not the owner's to answer as originally framed, which was my error.** The four questions
below were put to the owner as provenance questions. He rightly refused them: he was never the
reader — agent sessions wrote `RESEARCH.md` and `SPEC.md`, so he had no basis to attest to anything.
Three of the four then turned out not to be decisions at all. `CLEANROOM.md` §2 already records the
provenance as "unknown and unreconstructable", which *is* the finding; unreconstructable provenance
means fail closed, and §2 already specified the remedy. Only the risk-appetite question was ever his.
**Lesson: do not escalate a question whose answer is already written down, and check who could
possibly know before asking anyone.**

### What the re-derivation actually changed — it did not reproduce the old design

| | Superseded | Re-derived |
|---|---|---|
| Structure | block-sparse, delta-encoded | **sparse anchors + forward scan**; blocking kept for *allocation*, not compression |
| Stride | 128 lines/block | **64 lines/anchor**, chosen against cold-read latency |
| Index size, 10 GB | ~56 MB | **~6.3 MB** |
| Delta encoding | yes | **rejected** — ~9x worse than sparse anchoring, and long lines (§10.3) escape in every delta scheme |

**The finding that drove it: memory is not the binding constraint.** Every sparse variant is
negligible against §11.2's 120 MB claim, so the stride is chosen on the *cold* forward-scan read —
6.3 KB at S=64 against 100 KB at S=1024 — not on size. Derived from the two corpora's measured mean
line lengths (116.8 and 84.2 bytes), which §3 permits and prefers.

**Neither §5.3 was read.** The replacement was spliced by line range (394–436, boundaries asserted
against the §5.3/§5.4 headings) so the superseded text never entered context. Two anchors *were*
disclosed rather than hidden — `block-sparse, 128/block` from §11.2, read before the problem was
recognised, and the word `delta-encoded` from a diff header — and the derivation argues against both
explicitly. Had it landed on 128 with delta encoding, that coincidence would have needed disclosing
instead.

### The original blocker, retained because the containment failure is the reusable lesson

`CLEANROOM.md` §2 named `docs/RESEARCH.md` §5.3 as
*the* contaminated section and requires the index to be re-derived from §3 sources before any code.
Session 2 logged that §5.3's technical content was **deliberately not read**, specifically to keep
this agent eligible to write the index under §1.4's "whoever reads, doesn't write".

**That containment is understated, and the design has propagated into `SPEC.md`:**

| Where | What |
|---|---|
| `SPEC.md` §5.3 "Line index" | Same section number *and* topic as the contaminated `RESEARCH.md` §5.3. **Not read this session.** Its provenance is unestablished. |
| `SPEC.md` §11.2, memory budget table | States **"Line index (block-sparse, 128/block)"** — a specific design constant, in a section carrying no contamination warning. **This was read**, while deliberately avoiding §5.3. |

§2 explicitly forbids quoting §5.3 "into a design document that then feeds implementation", which is
what appears to have happened. Whether `128` was derived independently or traces to GPL source is
exactly the thing nobody recorded — the original sin §2 describes, now one document further on.

**Why this cannot be resolved by just pressing on.** Rule §1.4 is irreversible: reading contaminated
content permanently disqualifies the reader from writing that component. Guessing wrong costs either
this agent's eligibility for M2 or puts unestablished provenance into the codebase, and neither is
recoverable after the fact.

**How the four questions resolved:**

1. ~~Is `SPEC.md` §5.3 a restatement, or independently derived?~~ — **moot.** Replaced rather than
   adjudicated. Unknowable either way, which was itself the answer.
2. ~~Does `128/block` have establishable provenance?~~ — **no, and it is gone.** The re-derivation
   landed on 64 for reasons it states.
3. ~~Widen §2's affected-section list?~~ — **done.** §2 now carries a propagation table naming both
   `SPEC.md` sections, and §4's register is updated.
4. ~~Is this agent still eligible?~~ — **yes, with disclosure.** Neither §5.3 was read; the two
   anchors that were seen are recorded in §5 and §6 and argued against in the derivation.

**The reusable lesson, now in `CLEANROOM.md` §2:** *a contamination notice attached to one section
does not travel with the content.* When a design is quoted onward, the warning stays behind — which
is exactly how `SPEC.md` came to carry the design with no warning on it. **Naming the component in
§4 catches this; naming the section does not.** Session 8 read and edited `SPEC.md` §5.3 (`571eb2e`)
without filing a §5 entry, and nothing flagged it at the time.

**⚠ What the attestation deliberately does not cover.** It speaks only for the 2026-08-04
derivation. If the superseded `SPEC.md` §5.3 *was* contaminated, session 8's exposure was real and
unlogged, and nothing here retroactively clears it. The component is clean going forward because the
design was **rebuilt**, not because the earlier exposure was ruled harmless.

---

## ✅ Dogfooded against both real corpora — 2026-08-04, session 9. The encoding gate passes.

Both corpora were opened with the M1 build and their reported figures checked against independently
computed ground truth. **Both are exactly right.** Paths stay in project memory, not here.

| | Tailhawk reported | Independently computed | |
|---|---|---|---|
| Corpus A | `UTF-8, 85,763 lines, 10,015,454 bytes` | 85,762 `LF` + an unterminated trailing line; CRLF throughout; zero non-ASCII | ✅ |
| Corpus B | `UTF-8, 109,855 lines, 9,246,865 bytes` | 109,855 `LF`, ends with `LF`, no trailing partial; strict UTF-8 decode succeeds | ✅ |

**The encoding regression this was built to catch did not fire.** Corpus B is now **9.2 MB of
BOM-less UTF-8** carrying **47,101 U+2014 em dashes and 643 U+2713 check marks** (143,232 non-ASCII
bytes, exactly 3 bytes each). It was reported as UTF-8. A CP1252 default would have mangled every
one of them, which is the failure `Get-Content` still exhibits on this file.

**The `+1` on corpus A is correct, not an off-by-one.** `Decoder::finish` deliberately flushes a
trailing unterminated line at end-of-stream, while `pump` holds it during streaming. Corpus B, which
ends with `LF`, shows no such adjustment — the two together are the evidence the rule is right.

**E1's writer-safety guarantee paid off against a real writer.** `[IO.File]::ReadAllBytes` **could
not open corpus A at all** — sharing violation, because the live writer holds it — while Tailhawk
read it without trouble. Until now that guarantee was only exercised by its own unit test and its
negative control.

### Both corpora have outgrown their description, and corpus A rotates hard

The figures in the Dogfooding section below were sampled 2026-07-29 and are **stale**:

| | as sampled | 2026-08-04 |
|---|---|---|
| Corpus A | ~450 KB, *idle* | **grew 2.7 → 10 MB in ~3 minutes, then rotated back to 1.2 MB while being watched** |
| Corpus B | ~3.3 MB | **9.2 MB** |

Corpus A is no longer an idle-file test — it is a **fast-growing, actively-rotating** one, and it
rotated under observation without prompting. That makes it a far better **M4** subject than the
handoff previously assumed, and it is the only rotation specimen available that is real rather than
synthetic. M1 does not follow, so Tailhawk simply read what existed at open time.

### Owner direction, session 9: keep the milestone order — no UI work brought forward

M1 has no visible surface and was never scoped to have one: `paint()` draws `BACKGROUND` and nothing
else, and the title bar is the only place the file result surfaces. Confirmed this session by
screenshot — every pixel of the client area is **RGB(18, 20, 23)**, exactly `background_rgb8()`, on
the hardware driver. A correctly-painted M1 window and a dead one are indistinguishable to the eye,
because the v1 palette is near-black.

Three options were put to the owner: keep the order, build a throwaway visible spike (~1 day,
DirectWrite straight to the window, no index), or genuinely reorder M3 ahead of M2. **The owner chose
to keep the order**, and added that **there is no need to dogfood again until real logs actually
render** — i.e. at M3. This is consistent with the session-7 decision to prefer robustness over
reaching a usable UI sooner, now taken a second time with the visible consequence in front of them.

**So: no further dogfooding is owed until M3.** What was owed — "do the corpora decode correctly
before the index and grid are built on top of them" — is discharged above.

---

## 🚀 M1 is most of the way done — 2026-07-31, session 8. It reads real files.

```
cargo run --release -p tailhawk -- C:\path\to\some.log
```

opens a window whose title reports what the core made of the file — encoding, line count, bytes. That
is M1's demo. It is headless work, so the title bar is the only surface it has until the grid arrives
at M3.

Verified end to end on the static build:

| File | Title reported |
|---|---|
| BOM-less UTF-8 with em dashes (Corpus B's shape) | `UTF-8, 5 lines, 171 bytes` |
| UTF-8 with BOM | `UTF-8, 2 lines, 107 bytes` |
| UTF-16LE with BOM | `UTF-16LE, 3 lines, 228 bytes` |

**56 tests, all passing.** `cargo test -p tailhawk-core`.

| M1 criterion (`PLAN.md` §4) | State |
|---|---|
| **E5** — encoding detection (BOM, NUL-parity, UTF-8 validation, chardetng) | **Done** — `crates/tailhawk-core/src/encoding.rs` |
| **E6** — incremental streaming decode, carry across boundaries | **Done** — `crates/tailhawk-core/src/lines.rs` |
| **E1** — source abstraction, open/share modes, writer-safety guarantee | **Done** — `crates/tailhawk-core/src/file.rs`, with the rotation-loop test *and* a negative control |
| **E2** — overlapped `ReadFile` layer **on IOCP** | **Half done.** Overlapped reads with explicit `OVERLAPPED.Offset` work and are tested. **The IOCP with 4–8 outstanding requests is not built** — reads are issued one at a time. That is a throughput property with no consumer until M4's 50 MB/s criterion, so it was left rather than built blind. |
| Encoding fixture matrix decodes correctly | **Done** — BOM'd ×5, BOM-less UTF-8/16LE/16BE/32LE/32BE, head-vs-tail mixed, truncated-mid-sequence, binary-embedded, DBCS |
| Rotation loop shows no sharing violation on the writer side | **Done** — and paired with a negative control that opens `FILE_SHARE_READ` only and asserts the writer *is* locked out |
| Fuzz targets clean | **Done, session 9** — `arbitrary_bytes_decode_the_same_however_they_are_chunked` in `crates/tailhawk-core/src/lines.rs`. |

### ✅ M1 is complete — the fuzz criterion closed, session 9

**Option 1 was taken: an ordinary `#[test]`, not `cargo-fuzz`.** The deciding argument was not the
MSVC awkwardness the previous session flagged but **ARM64** — libFuzzer is effectively absent on
Windows ARM64 and CI builds both architectures, so `cargo-fuzz` would have covered one leg of the
matrix at best. The test runs in the existing job on both, with no new tooling.

**It needs no dependency at all.** A ten-line xorshift64* in the test module replaced `arbitrary`,
which keeps the `deny.toml` allow-list out of the decision entirely. A fuzzer needs a *reproducible
seed* far more than a good distribution, and a fixed iteration count keeps CI deterministic where a
wall-clock budget would not. `TAILHAWK_FUZZ_ITERS` raises it for a local soak.

**Most bytes are drawn from an interesting alphabet, not uniformly.** Uniform random bytes almost
never produce a `CRLF` pair, a valid multi-byte sequence or a BOM — so a uniform fuzzer explores none
of the states that actually carry across chunk boundaries, which is the whole point.

Three properties, none of which may depend on framing: no input panics; no emitted line contains
`\r` or `\n`; and the lines are identical however the same bytes are chunked, **including invalid
ones** — replacement characters are output too, and must land in the same places.

**Soaked at 40,000 iterations × 11 charsets × 7 framings — about 3.1 million decode runs, clean**, in
17 s release. That includes the two cases most likely to break the property: `ISO-2022-JP`, which is
stateful, and UTF-32, where a truncated unit at EOF was one of the three original bugs. CI runs 400
iterations (~1.9 s).

**59 tests, all passing.** `fmt` and `clippy` clean on the product crates.

The pre-existing boundary-invariance tests feed *fixed* inputs at every possible split; this feeds
*arbitrary content* at several splits. They are complementary, and the older ones remain the source
of the three real bugs found so far.

### Choices worth not re-litigating

- **UTF-32 is hand-written, and it is not gold-plating.** The WHATWG Encoding Standard excludes UTF-16
  and UTF-32 *detection* and excludes UTF-32 entirely, so `encoding_rs` cannot be delegated to for the
  parts §5.6 cares most about on Windows. `Charset` is therefore an enum wrapping `&'static Encoding`
  plus two UTF-32 variants, not a bare `&'static Encoding`.
- **`Charset::code_unit()` and `Charset::allows_parallel_indexing()` exist now, in M1.** §5.3 requires
  encoding to be resolved before *chunk assignment*, not merely before indexing. Putting the types M2
  needs into M1 is what makes "decode before index" structural rather than a note in a plan.
- **`detect` takes the codepage fallback as a parameter.** The core may not name a Win32 function
  (§3.1), so `system_codepage()` lives behind `cfg(windows)` and is passed in.
- **Line emission is a callback**, not an iterator. A line that spans chunks lives in the decoder's
  `pending` buffer and one that does not is borrowed from the scratch buffer; a callback serves both
  without allocating per line.
- **`FileSource::pump` holds a trailing partial line rather than emitting it.** A writer that has
  flushed half a line is the normal state of a live log.
- **Following, polling and rotation *handling* are absent on purpose.** They are M4. What M1 owes is
  that the bytes arrive correctly and the writer is never impeded.

### ✅ `SPEC.md` §5.3's DBCS exception is withdrawn — owner-approved, session 8

The spec disabled parallel indexing for codepages 932/936/950/949 on the grounds that `0x0A` is a
legal trail byte in them. **It is not**, and the property turned out to be far more general than the
DBCS carve-out it was raised against — so the exception was **deleted rather than narrowed**.

Two exhaustive tests in `crates/tailhawk-core/src/encoding.rs`, over eight multi-byte encodings and
ten single-byte representatives:

| | What it drives | Result |
|---|---|---|
| `a_0a_byte_is_always_a_terminator_in_every_byte_oriented_encoding` | every code point through every encoder | no character but U+000A produces a `0x0A`; U+000A always does |
| `a_0a_byte_is_never_consumed_as_a_trail_byte_by_any_decoder` | all 65,536 two-byte prefixes into every decoder, followed by `0x0A` | the newline survives every time |

**The second one is the test that matters, and the first version of this work did not have it.** A
scan reads arbitrary bytes off disk, not bytes some encoder produced, so the question is whether a
`0x0A` can be *consumed* as a trail byte — not whether one can be *written* there. Decoders accept
sequences encoders never emit. Measuring only the encoder side would have been a plausible,
confident, and insufficient basis for a normative change.

**What replaced it.** `allows_parallel_indexing()` is gone: it could only return `true`, and
`code_unit()` now carries the whole rule. In its place is `is_random_access_decodable()`, which is
false for **`ISO-2022-JP` alone** — escape-driven, so a line start is a *character* boundary but not
a *decoder* boundary. That constrains **viewport decode only, never the scan**, and it is the last
of what §5.3 used to claim.

### Size: the binary doubled, and it is still 3.2% of the gate

| | static `tailhawk.exe` |
|---|---|
| M0, session 7 | 249,344 |
| M1, with `encoding_rs` + `chardetng` linked | **502,784** |
| CI gate (`SPEC.md` §11.2) | 15,728,640 |

Worth recording rather than acting on. Note that the figure only moved once the **shell** referenced
the core's new code — before that LTO stripped both crates entirely and the exe was byte-identical to
M0's, which is a misleading way to measure a dependency.

---

## ✅ CI is green, and the ARM64 unknown is closed — 2026-07-31, session 8

Session 7 left this as the first thing to check. It has run twice and both runs succeeded.
**The ARM64 leg linked** — run #2 (`dbe45b6`) produced `tailhawk-arm64-unsigned` alongside the x64
artefact. The ⚠ is deleted from the M0 table below. ~~There is still no `gh` CLI on this machine; the
Actions page was read through the browser.~~ — **`gh` is installed as of session 10**, see below.

**Run #3 carried all of M1's code and passed on both architectures** — clippy, `fmt`, 56 tests, the
size gate, and both binary-content assertions (no `d3dcompiler`, no CRT redistributable).
`encoding_rs` and `chardetng` cross-compile to ARM64 without complaint.

**`cargo-deny` now runs, and it caught its own config on the first try.** The job was added in session
8 and failed immediately — not on a dependency, but because the config was named `cargo-deny.toml`,
which `cargo deny` does not look for. Renamed to `deny.toml`; the job is green and the allow-list is
now genuinely enforced rather than described. Both halves of that are in the traps table.

The only warning on any run is the Node 20 deprecation notice on `actions/checkout`, `actions/cache`
and `actions/upload-artifact`. Cosmetic, and it will resolve itself when those actions bump.

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
| CI producing x64 and ARM64 under the size gate | **Done and verified, session 8.** Both legs succeeded on the runner; the ARM64 artefact exists. This machine's VS install has no ARM64 linker, so CI is the only place that leg is provable — and it proved. |
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

~~**Start the next session by checking CI and then starting M1**~~ — **both done, session 8.** CI is
green including ARM64; M1 is most of the way through. See the two sections above.

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

## Directory rename — done, but as `TailHawk`. Settle the capital H or leave it alone.

The rename happened. The working directory is **`C:\dev\git\TailHawk`** — **capital H**, where the
plan said lowercase. Everything works: the project key is `C--dev-git-TailHawk`, memory lives there
and loads, and session 8 ran entirely from it.

**It is only cosmetic, and changing it is not free.** The project key is the literal path string, so
another rename orphans the memory again and needs the same copy step. The remote
(`github.com/nigelbasel/tailhawk`) and every document already say lowercase, so the directory is the
only place the capital H appears — and nothing reads the directory name.

**Recommendation: leave it.** If it is ever changed anyway, the procedure is unchanged from before —
it cannot be done from inside a session, because the harness resets the shell CWD into the repo after
every tool call and Windows will not rename a directory a process has as its CWD:

```powershell
cd C:\dev\git
Rename-Item TailHawk Tailhawk
Copy-Item -Recurse -Force "$env:USERPROFILE\.claude\projects\C--dev-git-TailHawk\*" `
                          "$env:USERPROFILE\.claude\projects\C--dev-git-Tailhawk"
```

**Decide once.** Every change of spelling — even just capitalisation — orphans the memory again.

---

## Where things stand

The project is **Tailhawk** (command `tailhawk`) — a Windows desktop log tailer/viewer.
Research, specification, UI design and development plan are complete and adversarially reviewed.
**Phase 0 is effectively closed, M0 is done, and M1 is most of the way through — the application
opens real log files and decodes them correctly, now verified against both real corpora (session 9)
rather than fixtures alone.** Nothing left in Phase 0 is both runnable and
blocking: G2 is informational, G3's `eframe` legs are moot, and G1/G5/G6 are owner-gated. Five
experiments were built and written up (G3 ×2, G4, G4b, G7).

Repo: **`github.com/nigelbasel/tailhawk`, private. CI is green on both x64 and ARM64.** Working
directly on `master` — no branches, no PRs (tried once, not worth it solo). Commit often; the history
is the artefact.

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

`LICENSE-MIT`, `LICENSE-APACHE` (dual, copyright asserted personally), `deny.toml`
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
2. ~~**Add `LICENSE-MIT`, `LICENSE-APACHE`, `deny.toml`**~~ — **done 2026-07-29.** All three
   are at the repo root. `deny.toml` is an allow-list, so GPL/AGPL/LGPL fail without being
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

**✅ Done, session 9 — the encoding gate passed on both corpora. See the section at the top for the
measured result, and note the size/behaviour figures in this section are the 2026-07-29 sample and
are now stale.** Nothing further is owed here until **M3**, by owner decision: no need to dogfood
again until real logs actually render.

~~**Now actionable, as of session 8.**~~ `cargo run --release -p tailhawk -- <path>` opens a file,
detects its encoding and streams every line, reporting the result in the title bar. That is enough to
run against both corpora and see whether the encoding and line count are right — it is not enough to
*read* them, which needs M3.

~~**Worth doing early rather than late:**~~ **— and it was.** The value of the two corpora is that the
correct answer is already known, and a wrong answer this session is far cheaper than a wrong answer
after the index and grid are built on top of it. Corpus B in particular should report **UTF-8** and
its em dashes must survive; if the title says `windows-1252` the detector is wrong. **It reported
UTF-8 and all 47,101 em dashes survived.**

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
| **Test a streaming decoder at *every* split, not a plausible one.** Feeding each fixture at every chunk size from 1 byte upwards found three bugs a single-gulp read cannot: UTF-32 at 1 byte/read decoded to nothing at all; a pending CR resolved against a chunk that decoded to *no characters*, giving every CRLF a phantom empty line; and a UTF-32 unit left at EOF was dropped instead of becoming U+FFFD. All three pass at chunk sizes that happen to land on boundaries. | `crates/tailhawk-core/src/lines.rs` |
| **A decoded chunk can be empty.** UTF-16LE fed one byte at a time produces text on only every second read. Any per-chunk state machine that assumes "a chunk has content" — the pending-CR carry is one — resolves against a character that has not arrived yet. | `crates/tailhawk-core/src/lines.rs` |
| **`0x0A` is *not* a legal DBCS trail byte**, contrary to what `SPEC.md` §5.3 said until session 8. It is always a line terminator, in every byte-oriented encoding — so parallel indexing needs no encoding-specific exception at all, only code-unit alignment. | `crates/tailhawk-core/src/encoding.rs` |
| **Measure the decoder, not the encoder, when the question is "what can appear in a file".** The encoder-side test — every code point through every encoder — was persuasive and would have been an insufficient basis for a normative spec change: decoders accept sequences encoders never emit. The test that actually settles it drives all 65,536 two-byte prefixes *into* each decoder. Both pass; only one of them is evidence. | `crates/tailhawk-core/src/encoding.rs` |
| **A predicate that can only return one value is a fact, not an API.** `allows_parallel_indexing()` became unconditionally `true` once the exception was withdrawn, so it was deleted and `code_unit()` carries the rule. What replaced it — `is_random_access_decodable()`, false for `ISO-2022-JP` alone — is non-vacuous and about a different question. | `crates/tailhawk-core/src/encoding.rs` |
| **A tail encoding sample is meaningless without its absolute offset.** NUL-position parity is relative to the start of the *file*; probing a sample that begins at an odd offset reports UTF-16**BE** for a UTF-16**LE** file — a confident, exactly-wrong answer. | `crates/tailhawk-core/src/encoding.rs` |
| **A share-mode test that only checks the happy path proves nothing.** `writer_safety_…` passes whether or not `FILE_SHARE_DELETE` is set, unless it is paired with a negative control that opens read-shared only and asserts the writer *is* blocked. A share mode is exactly the kind of constant that gets tidied by someone who does not know what it costs. | `crates/tailhawk-core/src/file.rs` |
| **`cargo deny` looks for `deny.toml`, not `cargo-deny.toml`.** The config was misnamed from session 2 to session 8. The first time anything ran it, it logged one `[WARN] unable to find a config path` and then rejected all 26 crates including MIT and Apache-2.0, because the default allow-list is empty. A misnamed config silently stops being the policy, and only running the tool reveals it. | `deny.toml`, `CLEANROOM.md` §7 |
| **A config file nothing executes is documentation.** `deny.toml` and `CLEANROOM.md` §7 described a licence gate for two sessions while no CI job invoked it. Whenever a policy file is added, add the thing that runs it in the same commit. | `.github/workflows/ci.yml` |
| **LTO makes an unreferenced dependency free, which is a misleading way to measure one.** Adding `encoding_rs` + `chardetng` left the exe byte-identical at 249,344 while nothing called them; it went to 502,784 the moment the shell did. Measure a dependency's size *after* wiring it in, never before. | `crates/tailhawk/src/main.rs` |
| **Never check a live file's reported size against a separately-stat'd one.** Corpus A read 5,587,678 bytes against a stat of 2,773,726 taken a minute earlier and looked like a 2.01x bug — a suspiciously exact ratio, which is what made it convincing. The file had simply grown. **To verify counts, snapshot the file first and run against the frozen copy;** use the live file only to exercise sharing and rotation. | session 9 |
| **The obvious verification tools cannot open a live log at all.** `[IO.File]::ReadAllBytes` and anything else opening share-`Read` fails with a sharing violation against a real writer — the very case Tailhawk exists to handle. Take ground truth through `[IO.File]::Open($p,'Open','Read','ReadWrite,Delete')`. Reaching for the naive reader first reads as "the file is locked", not "my reader is wrong". | session 9 |
| **A correctly-painted window and a dead one are indistinguishable at this palette.** `BACKGROUND` is RGB(18, 20, 23), so an M1 window that is working perfectly looks like an empty frame — the owner reasonably reported seeing nothing. **Do not debug this by looking.** Screenshot the client area and assert the pixels equal `background_rgb8()`; that distinguishes "painting correctly" from "not painting" in one step. | `crates/tailhawk-core/src/lib.rs` |

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
6. ~~**`SPEC.md` §5.3's DBCS rationale is factually wrong**~~ — **done, owner-approved, session 8.**
   The exception was deleted rather than narrowed, because the decoder-side measurement showed the
   property is universal across every byte-oriented encoding. §5.3 now states code-unit alignment as
   the *only* constraint on chunking, records both measurements, and keeps one much narrower rule:
   `ISO-2022-JP` cannot be decoded from an arbitrary line, because it is stateful. **M2 may index in
   parallel for every encoding.** See the section at the top.
7. ~~**How to fuzz, for M1's last criterion.**~~ — **done, session 9.** An ordinary `#[test]` with a
   deterministic seed and no dependency, not `cargo-fuzz`. The deciding argument was ARM64, not MSVC.
   **M1 is complete.** See the M1 section at the top.
8. ~~**The provenance questions above.**~~ — **resolved, session 9.** The index was re-derived clean
   and `SPEC.md` §5.3 replaced; `CLEANROOM.md` §4 reads RE-DERIVED CLEAN. **M2 is unblocked.** Three
   of the four were not decisions at all — see the section at the top.

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
