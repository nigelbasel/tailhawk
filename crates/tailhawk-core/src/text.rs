//! The glyph pass — `SPEC.md` §3.2's single instanced draw.
//!
//! One `DrawInstanced` renders a whole viewport of glyphs with per-instance foreground colour and
//! style, so per-token colouring is free and nothing builds an `IDWriteTextLayout` per line per
//! frame. Monochrome ClearType text and premultiplied colour emoji share **one blend state**, which
//! `experiments/g4-glyph-atlas` measured as both possible and cheaper than splitting them (15–32%
//! less CPU, because a split has to sort the instance buffer by mode and set state twice).
//!
//! The mechanism is dual-source blending: the pixel shader emits premultiplied colour in
//! `SV_Target0` and per-channel coverage in `SV_Target1`, and `SrcBlend = ONE`,
//! `DestBlend = INV_SRC1_COLOR` makes the hardware compute `dest = c0 + dest * (1 - c1)`.
//! Single-source straight alpha cannot do this at all — one alpha per pixel cannot carry three
//! independent coverages — so ClearType without dual-source blending costs a second pass over the
//! same geometry.

use windows::Win32::Graphics::Direct3D::{
    D3D11_SRV_DIMENSION_BUFFER, D3D_PRIMITIVE_TOPOLOGY_TRIANGLELIST,
};
use windows::Win32::Graphics::Direct3D11::{
    ID3D11BlendState, ID3D11Buffer, ID3D11Device, ID3D11DeviceContext, ID3D11PixelShader,
    ID3D11SamplerState, ID3D11ShaderResourceView, ID3D11VertexShader, D3D11_BIND_CONSTANT_BUFFER,
    D3D11_BIND_SHADER_RESOURCE, D3D11_BLEND_DESC, D3D11_BLEND_INV_SRC1_ALPHA,
    D3D11_BLEND_INV_SRC1_COLOR, D3D11_BLEND_ONE, D3D11_BLEND_OP_ADD, D3D11_BUFFER_DESC,
    D3D11_BUFFER_SRV, D3D11_BUFFER_SRV_0, D3D11_BUFFER_SRV_1, D3D11_COLOR_WRITE_ENABLE_ALL,
    D3D11_CPU_ACCESS_WRITE, D3D11_FILTER_MIN_MAG_MIP_POINT, D3D11_MAP_WRITE_DISCARD,
    D3D11_RENDER_TARGET_BLEND_DESC, D3D11_RESOURCE_MISC_BUFFER_STRUCTURED, D3D11_SAMPLER_DESC,
    D3D11_SHADER_RESOURCE_VIEW_DESC, D3D11_SHADER_RESOURCE_VIEW_DESC_0,
    D3D11_TEXTURE_ADDRESS_CLAMP, D3D11_USAGE_DYNAMIC,
};
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_UNKNOWN;

use crate::sheet::Sheet;
use crate::Result;

const GLYPHS_VS: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/glyphs_vs.cso"));
const GLYPHS_PS: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/glyphs_ps.cso"));

/// Three ClearType coverages from the mono sheet's RGB.
pub const MODE_MONO_SUBPIXEL: u32 = 0;
/// A premultiplied bitmap from the colour sheet. Unbound until the colour path exists, and an
/// unbound SRV samples as zero, so a stray instance in this mode draws nothing.
pub const MODE_COLOUR: u32 = 1;
/// The greyscale average, which the rasteriser wrote into the mono sheet's alpha channel.
pub const MODE_MONO_GREY: u32 = 2;

/// **A solid fill — no atlas, no coverage, just `tint`.** `SPEC.md` §7.1's highlight backgrounds and
/// §11.1's selection.
///
/// It needed no new pipeline, no new blend state and no new field on [`Instance`], which is worth
/// saying because the plan had this costed as the risk most likely to double M5. The dual-source
/// equation the glyph pass already binds — `dest = c0 + dest * (1 - c1)` — *is* an alpha composite
/// when the coverage is the same in every channel, and an opaque replace at `a = 1`.
///
/// **Ordering is by position in the instance buffer**, which is how these end up underneath: a
/// background instance is emitted before the glyphs that sit on it.
pub const MODE_SOLID: u32 = 3;

/// How many glyph quads one buffer holds. A 4K viewport of 8-pixel cells is about 48,000 cells, so
/// this covers a full screen in one draw; anything larger is drawn in several, never truncated.
const CAPACITY: u32 = 65_536;

/// One glyph quad. **The layout must match `Instance` in `shaders/glyphs.hlsl` exactly** — a
/// structured buffer is read as raw bytes, so a mismatch does not fail to compile, it draws
/// nonsense.
#[repr(C)]
#[derive(Copy, Clone, Debug, Default, PartialEq)]
pub struct Instance {
    /// Top-left of the quad in device pixels.
    pub pos: [f32; 2],
    pub size: [f32; 2],
    /// Top-left and bottom-right in sheet texels, not normalised — the shader divides by the sheet
    /// size, so an instance never has to know how big the sheet is.
    pub uv0: [f32; 2],
    pub uv1: [f32; 2],
    /// `rgb` is the foreground colour; `a` multiplies coverage.
    pub tint: [f32; 4],
    pub mode: u32,
    pub pad: [u32; 3],
}

/// Cuts an instance at `right`, reporting whether any of it is left to draw.
///
/// **A pane's rows are bounded, but not to the pixel.** `View::slice_anchored` rounds the visible
/// slice outward to whole clusters, so the glyph straddling a pane's right edge is laid out in
/// full: `paint.rs` asserts the slack it allows (`right <= viewport_px + cell_width`). Stacked, that
/// overrun ran off the window and nobody saw it. Side by side it lands on the neighbouring pane —
/// a ragged column of half-characters down the seam that changes as the log scrolls.
///
/// The quad and its texture window are cut together, so what survives is the left part of the same
/// glyph rather than a squeezed whole one. A zero-width remainder is dropped.
pub fn clip_right(inst: &mut Instance, right: f32) -> bool {
    let width = inst.size[0];
    if width <= 0.0 || inst.pos[0] >= right {
        return false;
    }
    let kept = right - inst.pos[0];
    if kept >= width {
        return true;
    }
    let share = kept / width;
    inst.uv1[0] = inst.uv0[0] + (inst.uv1[0] - inst.uv0[0]) * share;
    inst.size[0] = kept;
    true
}

/// Cuts everything in `buffer` from index `start` onward at `right`, dropping what is wholly past.
///
/// **A free function rather than a `Painter` method**, so the index arithmetic can be exercised
/// with three hand-built instances and no device. The `Painter` half needs an `HWND` and a live
/// D3D device to reach, and this project's scar list is entirely made of decisions that were
/// correct and untestable until they were neither.
///
/// The prefix before `start` belongs to panes already placed and is left exactly as it is.
pub fn cut_from(buffer: &mut Vec<Instance>, start: usize, right: f32) {
    let start = start.min(buffer.len());
    let mut at = 0usize;
    buffer.retain_mut(|inst| {
        let keep = at < start || clip_right(inst, right);
        at += 1;
        keep
    });
}

#[repr(C)]
#[derive(Copy, Clone, Default)]
struct Consts {
    viewport: [f32; 2],
    atlas: [f32; 2],
    pad: [f32; 4],
}

pub struct TextPipeline {
    vs: ID3D11VertexShader,
    ps: ID3D11PixelShader,
    blend: ID3D11BlendState,
    sampler: ID3D11SamplerState,
    instances: ID3D11Buffer,
    instances_srv: ID3D11ShaderResourceView,
    consts: ID3D11Buffer,
}

impl TextPipeline {
    pub fn new(device: &ID3D11Device) -> Result<Self> {
        let mut vs = None;
        let mut ps = None;
        unsafe {
            device.CreateVertexShader(GLYPHS_VS, None, Some(&mut vs))?;
            device.CreatePixelShader(GLYPHS_PS, None, Some(&mut ps))?;
        }

        // dest = c0 + dest * (1 - c1).
        //
        // **The alpha slots must take `INV_SRC1_ALPHA`, not `INV_SRC1_COLOR`.** A `*_COLOR` factor
        // in an alpha slot fails `CreateBlendState` with a bare `E_INVALIDARG` and no further
        // explanation (G4).
        let mut blend_desc = D3D11_BLEND_DESC::default();
        blend_desc.RenderTarget[0] = D3D11_RENDER_TARGET_BLEND_DESC {
            BlendEnable: true.into(),
            SrcBlend: D3D11_BLEND_ONE,
            DestBlend: D3D11_BLEND_INV_SRC1_COLOR,
            BlendOp: D3D11_BLEND_OP_ADD,
            SrcBlendAlpha: D3D11_BLEND_ONE,
            DestBlendAlpha: D3D11_BLEND_INV_SRC1_ALPHA,
            BlendOpAlpha: D3D11_BLEND_OP_ADD,
            RenderTargetWriteMask: D3D11_COLOR_WRITE_ENABLE_ALL.0 as u8,
        };
        let mut blend = None;
        unsafe { device.CreateBlendState(&blend_desc, Some(&mut blend))? };

        // Point sampling, clamped. A glyph cell is addressed in whole texels at a scale the atlas
        // was rasterised for, so any filtering can only blur it — and bleed a neighbouring slot's
        // ink in at the edges.
        let sampler_desc = D3D11_SAMPLER_DESC {
            Filter: D3D11_FILTER_MIN_MAG_MIP_POINT,
            AddressU: D3D11_TEXTURE_ADDRESS_CLAMP,
            AddressV: D3D11_TEXTURE_ADDRESS_CLAMP,
            AddressW: D3D11_TEXTURE_ADDRESS_CLAMP,
            MaxLOD: f32::MAX,
            ..Default::default()
        };
        let mut sampler = None;
        unsafe { device.CreateSamplerState(&sampler_desc, Some(&mut sampler))? };

        let stride = std::mem::size_of::<Instance>() as u32;
        let buffer_desc = D3D11_BUFFER_DESC {
            ByteWidth: stride * CAPACITY,
            Usage: D3D11_USAGE_DYNAMIC,
            BindFlags: D3D11_BIND_SHADER_RESOURCE.0 as u32,
            CPUAccessFlags: D3D11_CPU_ACCESS_WRITE.0 as u32,
            MiscFlags: D3D11_RESOURCE_MISC_BUFFER_STRUCTURED.0 as u32,
            StructureByteStride: stride,
        };
        let mut instances = None;
        unsafe { device.CreateBuffer(&buffer_desc, None, Some(&mut instances))? };
        let instances = instances.expect("buffer out param is set on success");

        // A buffer SRV has to be described explicitly. Passing `None` works for textures, where the
        // dimensions are inferable from the resource, and silently is not an option here.
        let srv_desc = D3D11_SHADER_RESOURCE_VIEW_DESC {
            Format: DXGI_FORMAT_UNKNOWN,
            ViewDimension: D3D11_SRV_DIMENSION_BUFFER,
            Anonymous: D3D11_SHADER_RESOURCE_VIEW_DESC_0 {
                Buffer: D3D11_BUFFER_SRV {
                    Anonymous1: D3D11_BUFFER_SRV_0 { FirstElement: 0 },
                    Anonymous2: D3D11_BUFFER_SRV_1 {
                        NumElements: CAPACITY,
                    },
                },
            },
        };
        let mut instances_srv = None;
        unsafe {
            device.CreateShaderResourceView(
                &instances,
                Some(&srv_desc),
                Some(&mut instances_srv),
            )?
        };

        let consts_desc = D3D11_BUFFER_DESC {
            ByteWidth: std::mem::size_of::<Consts>() as u32,
            Usage: D3D11_USAGE_DYNAMIC,
            BindFlags: D3D11_BIND_CONSTANT_BUFFER.0 as u32,
            CPUAccessFlags: D3D11_CPU_ACCESS_WRITE.0 as u32,
            ..Default::default()
        };
        let mut consts = None;
        unsafe { device.CreateBuffer(&consts_desc, None, Some(&mut consts))? };

        Ok(Self {
            vs: vs.expect("vertex shader out param is set on success"),
            ps: ps.expect("pixel shader out param is set on success"),
            blend: blend.expect("blend state out param is set on success"),
            sampler: sampler.expect("sampler out param is set on success"),
            instances,
            instances_srv: instances_srv.expect("srv out param is set on success"),
            consts: consts.expect("constant buffer out param is set on success"),
        })
    }

    /// Draws every instance. **The render target and viewport must already be set** — one frame
    /// sets them once and may run several passes through them.
    ///
    /// More instances than one buffer holds are drawn in several calls rather than truncated. That
    /// is not the one-draw rule being abandoned: it is the rule holding for every viewport that
    /// fits in `CAPACITY`, which is every viewport a monitor has.
    pub fn draw(
        &self,
        context: &ID3D11DeviceContext,
        mono: &Sheet,
        viewport: (u32, u32),
        instances: &[Instance],
    ) -> Result<()> {
        if instances.is_empty() {
            return Ok(());
        }
        let (sheet_w, sheet_h) = mono.size();
        self.write_consts(context, viewport, (sheet_w, sheet_h))?;

        unsafe {
            context.IASetPrimitiveTopology(D3D_PRIMITIVE_TOPOLOGY_TRIANGLELIST);
            context.VSSetShader(&self.vs, None);
            context.PSSetShader(&self.ps, None);
            context.VSSetConstantBuffers(0, Some(&[Some(self.consts.clone())]));
            context.VSSetShaderResources(0, Some(&[Some(self.instances_srv.clone())]));
            context.PSSetShaderResources(1, Some(&[Some(mono.srv().clone())]));
            context.PSSetSamplers(0, Some(&[Some(self.sampler.clone())]));
            context.OMSetBlendState(&self.blend, Some(&[0.0; 4]), 0xFFFF_FFFF);
        }

        for chunk in instances.chunks(CAPACITY as usize) {
            self.write_instances(context, chunk)?;
            unsafe { context.DrawInstanced(6, chunk.len() as u32, 0, 0) };
        }
        Ok(())
    }

    fn write_consts(
        &self,
        context: &ID3D11DeviceContext,
        viewport: (u32, u32),
        sheet: (u16, u16),
    ) -> Result<()> {
        let consts = Consts {
            viewport: [viewport.0.max(1) as f32, viewport.1.max(1) as f32],
            atlas: [sheet.0.max(1) as f32, sheet.1.max(1) as f32],
            pad: [0.0; 4],
        };
        let mut mapped = Default::default();
        unsafe {
            context.Map(
                &self.consts,
                0,
                D3D11_MAP_WRITE_DISCARD,
                0,
                Some(&mut mapped),
            )?;
            std::ptr::copy_nonoverlapping(
                std::ptr::addr_of!(consts).cast::<u8>(),
                mapped.pData.cast::<u8>(),
                std::mem::size_of::<Consts>(),
            );
            context.Unmap(&self.consts, 0);
        }
        Ok(())
    }

    fn write_instances(&self, context: &ID3D11DeviceContext, chunk: &[Instance]) -> Result<()> {
        let mut mapped = Default::default();
        unsafe {
            context.Map(
                &self.instances,
                0,
                D3D11_MAP_WRITE_DISCARD,
                0,
                Some(&mut mapped),
            )?;
            std::ptr::copy_nonoverlapping(
                chunk.as_ptr().cast::<u8>(),
                mapped.pData.cast::<u8>(),
                std::mem::size_of_val(chunk),
            );
            context.Unmap(&self.instances, 0);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gpu::offscreen::Offscreen;

    const SHEET: u16 = 32;
    const TARGET: u32 = 24;

    fn quad(x: f32, w: f32) -> Instance {
        Instance {
            pos: [x, 10.0],
            size: [w, 20.0],
            uv0: [100.0, 200.0],
            uv1: [110.0, 220.0],
            tint: [1.0; 4],
            mode: 0,
            pad: [0; 3],
        }
    }

    /// **The quad and its texture window are cut together.** Narrowing the quad alone would squeeze
    /// the whole glyph into the space that is left — a visibly thinner letter at every seam —
    /// rather than showing the left part of it and hiding the rest.
    #[test]
    fn a_glyph_straddling_the_edge_is_cut_not_squeezed() {
        let mut half = quad(90.0, 10.0);
        assert!(clip_right(&mut half, 95.0), "half of it is still on screen");
        assert_eq!(half.size[0], 5.0, "the quad is cut to the edge");
        assert_eq!(half.pos[0], 90.0, "and does not move");
        assert_eq!(
            half.uv1[0], 105.0,
            "the texture window is cut by the same share"
        );
        assert_eq!(half.uv0, [100.0, 200.0], "the left edge is untouched");
        assert_eq!(half.uv1[1], 220.0, "and so is the vertical extent");
    }

    /// **The panes already placed are never touched.** Each pane clips from its own mark, and that
    /// mark is the buffer's length at the moment its layout began — so everything before it belongs
    /// to a pane that has already been positioned and cut to its own edge. Re-cutting the prefix at
    /// this pane's edge would shred the pane to its left.
    #[test]
    fn only_this_pane_is_cut_and_a_mark_past_the_end_is_a_no_op() {
        let mut buffer = vec![quad(10.0, 10.0), quad(500.0, 10.0), quad(95.0, 10.0)];
        cut_from(&mut buffer, 1, 100.0);
        assert_eq!(buffer.len(), 2, "the one wholly past the edge goes");
        assert_eq!(buffer[0].pos[0], 10.0, "the earlier pane is untouched…");
        assert_eq!(buffer[0].size[0], 10.0, "…including its width");
        assert_eq!(buffer[1].pos[0], 95.0);
        assert_eq!(buffer[1].size[0], 5.0, "and this pane's straddler is cut");

        let mut nothing = vec![quad(500.0, 10.0)];
        cut_from(&mut nothing, 9, 100.0);
        assert_eq!(nothing.len(), 1, "a mark past the end cuts nothing");
        assert_eq!(nothing[0].size[0], 10.0);

        let mut all = vec![quad(500.0, 10.0), quad(600.0, 10.0)];
        cut_from(&mut all, 0, 100.0);
        assert!(
            all.is_empty(),
            "a pane with nothing inside it draws nothing"
        );
    }

    /// A glyph wholly inside is untouched, and one wholly past the edge is dropped rather than
    /// drawn at zero width.
    #[test]
    fn what_fits_is_left_alone_and_what_is_past_the_edge_goes() {
        let mut inside = quad(10.0, 10.0);
        let before = inside;
        assert!(clip_right(&mut inside, 400.0));
        assert_eq!(inside.size, before.size);
        assert_eq!(inside.uv1, before.uv1);

        assert!(
            !clip_right(&mut quad(400.0, 10.0), 400.0),
            "starts at the edge"
        );
        assert!(!clip_right(&mut quad(500.0, 10.0), 400.0), "starts past it");
        assert!(!clip_right(&mut quad(10.0, 0.0), 400.0), "nothing to draw");
        let mut exact = quad(390.0, 10.0);
        assert!(clip_right(&mut exact, 400.0), "ends exactly on the edge");
        assert_eq!(exact.size[0], 10.0, "and is not cut");
    }

    /// A dark backdrop, for the tests that only ask whether ink landed.
    const BACKDROP: [f32; 4] = [0.1, 0.1, 0.1, 1.0];

    /// Black ink on light neutral paper — the arrangement that isolates the blend equation. `INK`
    /// has `rgb = 0`, so the source term contributes nothing and every channel of the result comes
    /// from the destination factor; `PAPER` is **exactly neutral**, so any channel difference in the
    /// result was produced by the blend rather than being there beforehand. G4's equivalent check
    /// first used the real tint (0.87/0.89/0.92), which is non-neutral enough on its own to fake
    /// about 13 levels of "fringing" and proved nothing.
    const INK: [f32; 4] = [0.0, 0.0, 0.0, 1.0];
    const PAPER: [f32; 4] = [0.85, 0.85, 0.85, 1.0];

    fn offscreen_or_skip(what: &str) -> Option<Offscreen> {
        match Offscreen::new(TARGET, TARGET) {
            Ok(o) => Some(o),
            Err(e) => {
                eprintln!("SKIPPED {what}: no D3D11 device ({e})");
                None
            }
        }
    }

    /// Fills an 8×8 patch of the sheet with one RGBA value, at the sheet's origin.
    fn patch(rgba: [u8; 4]) -> Vec<u8> {
        rgba.iter().cycle().take(8 * 8 * 4).copied().collect()
    }

    fn one_quad(mode: u32, tint: [f32; 4]) -> Instance {
        Instance {
            pos: [4.0, 4.0],
            size: [8.0, 8.0],
            uv0: [0.0, 0.0],
            uv1: [8.0, 8.0],
            tint,
            mode,
            pad: [0; 3],
        }
    }

    /// `CreateBlendState` is where the trap lives: a `*_COLOR` factor in an alpha slot fails with a
    /// bare `E_INVALIDARG` and no explanation. If the pipeline builds at all, the dual-source state
    /// the whole design rests on was accepted.
    #[test]
    fn the_dual_source_blend_state_is_accepted() {
        let Some(off) = offscreen_or_skip("the_dual_source_blend_state_is_accepted") else {
            return;
        };
        TextPipeline::new(off.device()).expect("the dual-source blend state must be creatable");
    }

    #[test]
    fn a_glyph_quad_puts_ink_where_it_was_asked_to_and_nowhere_else() {
        let Some(off) =
            offscreen_or_skip("a_glyph_quad_puts_ink_where_it_was_asked_to_and_nowhere_else")
        else {
            return;
        };
        let pipeline = TextPipeline::new(off.device()).expect("pipeline");
        let sheet = Sheet::mono(off.device(), SHEET, SHEET).expect("sheet");
        assert!(
            sheet.upload(off.context(), 0, 0, 8, 8, &patch([255, 255, 255, 255])),
            "a patch inside the sheet must upload"
        );

        off.clear(BACKDROP);
        pipeline
            .draw(
                off.context(),
                &sheet,
                (TARGET, TARGET),
                &[one_quad(MODE_MONO_SUBPIXEL, [1.0, 1.0, 1.0, 1.0])],
            )
            .expect("draw");
        let pixels = off.read_back().expect("read back");

        let inside = pixels.at(8, 8);
        let outside = pixels.at(1, 1);
        assert!(
            inside.iter().take(3).all(|&c| c > 200),
            "full white coverage over a dark backdrop should be near-white, got {inside:?}"
        );
        assert!(
            outside.iter().take(3).all(|&c| c < 60),
            "outside the quad must be untouched backdrop, got {outside:?}"
        );
        // The quad is at (4,4)-(12,12), so one pixel outside each edge must still be backdrop.
        for (x, y) in [(3, 8), (12, 8), (8, 3), (8, 12)] {
            let edge = pixels.at(x, y);
            assert!(
                edge.iter().take(3).all(|&c| c < 60),
                "the quad bled outside its bounds at ({x},{y}): {edge:?}"
            );
        }
    }

    /// §7.1's highlight background, **in pixels**, and the two things about it that matter.
    ///
    /// It samples no atlas — so a solid quad must paint its colour whatever the sheet holds, which
    /// this proves by leaving the sheet empty. And instances draw in **buffer order**, so a solid
    /// emitted before a glyph ends up underneath it; that ordering is the entire mechanism by which
    /// a background is a background, and nothing else enforces it.
    #[test]
    fn a_solid_quad_fills_its_rectangle_and_glyphs_land_on_top_of_it() {
        let Some(off) = offscreen_or_skip("a_solid_quad_fills_its_rectangle") else {
            return;
        };
        let pipeline = TextPipeline::new(off.device()).expect("pipeline");
        let sheet = Sheet::mono(off.device(), SHEET, SHEET).expect("sheet");
        assert!(
            sheet.upload(off.context(), 0, 0, 8, 8, &patch([255, 255, 255, 255])),
            "a patch inside the sheet must upload"
        );

        // A red background, then black "ink" over it. Red is chosen because neither the backdrop nor
        // the ink is red, so a red pixel can only have come from the solid quad.
        //
        // **⚠ The read-back is BGRA**, because the render target is `DXGI_FORMAT_B8G8R8A8_UNORM` —
        // so red arrives at index **2**. Every other pixel test in this file happens to be
        // channel-agnostic (`all(|c| c > 200)`), so this is the first one that can get it wrong, and
        // the first version did: it asserted `[0] > 200` and read `[0, 0, 255, 255]`.
        const RED: [f32; 4] = [1.0, 0.0, 0.0, 1.0];
        const R: usize = 2;
        const G: usize = 1;
        const B: usize = 0;
        off.clear(BACKDROP);
        pipeline
            .draw(
                off.context(),
                &sheet,
                (TARGET, TARGET),
                &[one_quad(MODE_SOLID, RED), one_quad(MODE_MONO_SUBPIXEL, INK)],
            )
            .expect("draw");
        let pixels = off.read_back().expect("read back");

        // Under the glyph the ink won, and the background did not show through it.
        let inked = pixels.at(8, 8);
        assert!(
            inked[0] < 60 && inked[1] < 60 && inked[2] < 60,
            "the glyph must sit on top of the background, got {inked:?}"
        );

        // Outside the quads, untouched backdrop — the solid did not bleed.
        let outside = pixels.at(1, 1);
        assert!(
            outside[R] < 60,
            "the solid quad must not paint outside its rectangle, got {outside:?}"
        );

        // And a solid on its own, with no glyph over it, is the colour it was given.
        off.clear(BACKDROP);
        pipeline
            .draw(
                off.context(),
                &sheet,
                (TARGET, TARGET),
                &[one_quad(MODE_SOLID, RED)],
            )
            .expect("draw");
        let pixels = off.read_back().expect("read back");
        let filled = pixels.at(8, 8);
        assert!(
            filled[R] > 200 && filled[G] < 60 && filled[B] < 60,
            "a solid quad must paint its own colour without sampling the sheet, got {filled:?}"
        );
    }

    /// **The claim in `SPEC.md` §3.2 that this pipeline exists to make good**, and it takes care to
    /// test something that is actually about the blend.
    ///
    /// The arrangement is **black text on a light neutral backdrop**, which is where ClearType
    /// fringing is visible in real life. With `tint.rgb = 0` the source term `c0.rgb` is zero, so
    /// every channel of the result comes from `dest * (1 - c1)` and nothing else: per-channel
    /// coverage attenuates the three channels differently and a neutral backdrop comes back
    /// non-neutral, while a single alpha attenuates them equally and it stays neutral.
    ///
    /// **The obvious version of this test does not discriminate**, which is worth knowing before
    /// rewriting it. With a *light* tint over a dark backdrop the spread comes from
    /// `c0.rgb = tint.rgb * cov` no matter what the destination factor does — so the test passes
    /// even with the blend wired to a single alpha, which was verified by mutation. G4's own first
    /// attempt made the same class of mistake with a non-neutral tint.
    #[test]
    fn per_channel_coverage_survives_the_blend_which_is_what_makes_it_cleartype() {
        let Some(off) = offscreen_or_skip(
            "per_channel_coverage_survives_the_blend_which_is_what_makes_it_cleartype",
        ) else {
            return;
        };
        let pipeline = TextPipeline::new(off.device()).expect("pipeline");
        let sheet = Sheet::mono(off.device(), SHEET, SHEET).expect("sheet");
        // Three deliberately different coverages, with alpha carrying their average as the
        // rasteriser writes it.
        let coverage = [220u8, 128, 32];
        let average = ((220 + 128 + 32) / 3) as u8;
        assert!(sheet.upload(
            off.context(),
            0,
            0,
            8,
            8,
            &patch([coverage[0], coverage[1], coverage[2], average])
        ));

        off.clear(PAPER);
        pipeline
            .draw(
                off.context(),
                &sheet,
                (TARGET, TARGET),
                &[one_quad(MODE_MONO_SUBPIXEL, INK)],
            )
            .expect("draw");
        let subpixel = off.read_back().expect("read back").at(8, 8);

        let spread = subpixel[..3].iter().max().unwrap() - subpixel[..3].iter().min().unwrap();
        assert!(
            spread > 8,
            "a neutral backdrop came back neutral ({subpixel:?}) — the blend collapsed three \
             coverages into one alpha, so this is not ClearType"
        );
        // Ordering, not just spread. The channel given the *most* coverage is attenuated most, so it
        // ends up darkest, and the readback is BGRA — index 2 is red, whose coverage was 220.
        assert!(
            subpixel[2] < subpixel[1] && subpixel[1] < subpixel[0],
            "R had the most coverage so it should be darkest and B lightest, in BGRA: {subpixel:?}"
        );
    }

    /// The same sheet, the same instance, one field different — the greyscale mode has to read the
    /// average out of alpha rather than the three coverages, or the second half of "one sheet serves
    /// both" is not true.
    #[test]
    fn greyscale_mode_reads_the_average_and_has_no_spread() {
        let Some(off) = offscreen_or_skip("greyscale_mode_reads_the_average_and_has_no_spread")
        else {
            return;
        };
        let pipeline = TextPipeline::new(off.device()).expect("pipeline");
        let sheet = Sheet::mono(off.device(), SHEET, SHEET).expect("sheet");
        assert!(sheet.upload(off.context(), 0, 0, 8, 8, &patch([220, 128, 32, 126])));

        // The same arrangement as the subpixel test, so the two are a matched pair: identical sheet,
        // identical backdrop, identical tint, one field of the instance different.
        off.clear(PAPER);
        pipeline
            .draw(
                off.context(),
                &sheet,
                (TARGET, TARGET),
                &[one_quad(MODE_MONO_GREY, INK)],
            )
            .expect("draw");
        let grey = off.read_back().expect("read back").at(8, 8);

        let spread = grey[..3].iter().max().unwrap() - grey[..3].iter().min().unwrap();
        assert!(
            spread <= 1,
            "greyscale mode must apply one coverage to all three channels, got {grey:?}"
        );
    }

    /// A stray colour instance before the colour sheet exists must be invisible, not garbage. An
    /// unbound SRV samples as zero, which makes the premultiplied composite a no-op.
    #[test]
    fn a_colour_instance_draws_nothing_while_the_colour_sheet_is_unbound() {
        let Some(off) =
            offscreen_or_skip("a_colour_instance_draws_nothing_while_the_colour_sheet_is_unbound")
        else {
            return;
        };
        let pipeline = TextPipeline::new(off.device()).expect("pipeline");
        let sheet = Sheet::mono(off.device(), SHEET, SHEET).expect("sheet");
        assert!(sheet.upload(off.context(), 0, 0, 8, 8, &patch([255, 255, 255, 255])));

        off.clear(BACKDROP);
        pipeline
            .draw(
                off.context(),
                &sheet,
                (TARGET, TARGET),
                &[one_quad(MODE_COLOUR, [1.0, 1.0, 1.0, 1.0])],
            )
            .expect("draw");
        let pixels = off.read_back().expect("read back");
        assert!(
            pixels.at(8, 8).iter().take(3).all(|&c| c < 60),
            "an unbound colour sheet must sample as zero: {:?}",
            pixels.at(8, 8)
        );
    }

    /// Nothing to draw is not an error, and must not leave the target changed.
    #[test]
    fn an_empty_instance_list_draws_nothing() {
        let Some(off) = offscreen_or_skip("an_empty_instance_list_draws_nothing") else {
            return;
        };
        let pipeline = TextPipeline::new(off.device()).expect("pipeline");
        let sheet = Sheet::mono(off.device(), SHEET, SHEET).expect("sheet");
        off.clear(BACKDROP);
        pipeline
            .draw(off.context(), &sheet, (TARGET, TARGET), &[])
            .expect("an empty draw is not a failure");
        assert!(off.read_back().expect("read back").at(8, 8)[..3]
            .iter()
            .all(|&c| c < 60));
    }

    /// The sheet's own guard: a slot rectangle that leaves the sheet, or a buffer that is not
    /// exactly one cell, is refused. An out-of-range `D3D11_BOX` is undefined rather than an error,
    /// so this cannot be left to the driver.
    #[test]
    fn a_slot_outside_the_sheet_is_refused_rather_than_written() {
        let Some(off) =
            offscreen_or_skip("a_slot_outside_the_sheet_is_refused_rather_than_written")
        else {
            return;
        };
        let sheet = Sheet::mono(off.device(), SHEET, SHEET).expect("sheet");
        let ctx = off.context();
        assert!(
            !sheet.upload(ctx, SHEET - 4, 0, 8, 8, &patch([1, 2, 3, 4])),
            "runs off the right"
        );
        assert!(
            !sheet.upload(ctx, 0, SHEET - 4, 8, 8, &patch([1, 2, 3, 4])),
            "runs off the bottom"
        );
        assert!(
            !sheet.upload(ctx, 0, 0, 0, 8, &[]),
            "an empty rectangle is not a slot"
        );
        assert!(
            !sheet.upload(ctx, 0, 0, 8, 8, &patch([1, 2, 3, 4])[..64]),
            "a short buffer would read past its end"
        );
        assert!(
            sheet.upload(ctx, SHEET - 8, SHEET - 8, 8, 8, &patch([1, 2, 3, 4])),
            "exactly fits"
        );
    }
}
