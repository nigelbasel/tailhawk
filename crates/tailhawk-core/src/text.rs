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
