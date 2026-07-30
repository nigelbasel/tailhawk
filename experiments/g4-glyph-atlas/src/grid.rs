//! The viewport renderer, and the G4 measurements.
//!
//! The design being tested: **one instanced draw, one blend state, both glyph kinds.** The pixel
//! shader emits two outputs — premultiplied colour in `SV_Target0` and per-channel coverage in
//! `SV_Target1` — and dual-source blending (`ONE`, `INV_SRC1_COLOR`) resolves
//! `dest = src + dest * (1 - coverage)`. That expression is simultaneously correct for
//! per-channel ClearType coverage and for a premultiplied colour bitmap, so the two do not need
//! separate blend states.

use std::ffi::c_void;

use windows::core::{Result, PCSTR};
use windows::Win32::Graphics::Direct3D::Fxc::{D3DCompile, D3DCOMPILE_OPTIMIZATION_LEVEL3};
use windows::Win32::Graphics::Direct3D::{
    ID3DBlob, D3D_PRIMITIVE_TOPOLOGY_TRIANGLELIST, D3D_SRV_DIMENSION_BUFFER,
};
use windows::Win32::Graphics::Direct3D11::{
    ID3D11Buffer, ID3D11BlendState, ID3D11Device, ID3D11DeviceContext, ID3D11PixelShader,
    ID3D11Query, ID3D11SamplerState, ID3D11ShaderResourceView, ID3D11VertexShader,
    D3D11_BIND_CONSTANT_BUFFER, D3D11_BIND_SHADER_RESOURCE, D3D11_BLEND_DESC,
    D3D11_BLEND_INV_SRC1_ALPHA, D3D11_BLEND_INV_SRC1_COLOR, D3D11_BLEND_INV_SRC_ALPHA,
    D3D11_BLEND_ONE, D3D11_BLEND_OP_ADD, D3D11_BLEND_SRC_ALPHA, D3D11_BUFFER_DESC,
    D3D11_BUFFER_SRV, D3D11_BUFFER_SRV_0, D3D11_BUFFER_SRV_1, D3D11_COLOR_WRITE_ENABLE_ALL,
    D3D11_CPU_ACCESS_WRITE, D3D11_FILTER_MIN_MAG_MIP_POINT, D3D11_MAP_WRITE_DISCARD,
    D3D11_QUERY_DATA_TIMESTAMP_DISJOINT, D3D11_QUERY_DESC, D3D11_QUERY_TIMESTAMP,
    D3D11_QUERY_TIMESTAMP_DISJOINT, D3D11_RENDER_TARGET_BLEND_DESC,
    D3D11_RESOURCE_MISC_BUFFER_STRUCTURED, D3D11_SAMPLER_DESC, D3D11_SHADER_RESOURCE_VIEW_DESC,
    D3D11_SHADER_RESOURCE_VIEW_DESC_0, D3D11_TEXTURE_ADDRESS_CLAMP, D3D11_USAGE_DYNAMIC,
    D3D11_VIEWPORT,
};
use windows::Win32::Graphics::Dxgi::Common::{
    DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_FORMAT_R8G8B8A8_UNORM,
};

use crate::atlas::{AtlasTexture, GlyphCache, GlyphEntry, GlyphKey, Policy};
use crate::gpu::{Driver, Gpu};
use crate::text::{self, Fonts, FACE_CJK, FACE_EMOJI, FACE_MONO};

const ATLAS_W: u32 = 1024;
const ATLAS_H: u32 = 1024;
const EM: f32 = 14.0;
/// Slot box. Must cover the widest glyph accepted — a ClearType raster of a full-width CJK
/// ideograph or an emoji at `EM`, rounded up. Uniform slots are what make eviction O(1); the cost
/// is atlas density on narrow Latin glyphs, which is small because a log grid is monospace anyway.
const SLOT_W: u32 = 20;
const SLOT_H: u32 = 22;

const MODE_MONO_SUBPIXEL: u32 = 0;
const MODE_COLOUR: u32 = 1;
const MODE_MONO_GREY: u32 = 2;

#[repr(C)]
#[derive(Copy, Clone, Default)]
struct Instance {
    pos: [f32; 2],
    size: [f32; 2],
    uv0: [f32; 2],
    uv1: [f32; 2],
    tint: [f32; 4],
    mode: u32,
    _pad: [u32; 3],
}

#[repr(C)]
#[derive(Copy, Clone, Default)]
struct Consts {
    viewport: [f32; 2],
    atlas: [f32; 2],
    _pad: [f32; 4],
}

const SHADER: &str = r#"
cbuffer Consts : register(b0) {
    float2 viewport;
    float2 atlas;
    float4 pad_;
};

struct Instance {
    float2 pos;
    float2 size;
    float2 uv0;
    float2 uv1;
    float4 tint;
    uint mode;
    uint3 pad_;
};

StructuredBuffer<Instance> instances : register(t0);
Texture2D<float4> mono_atlas       : register(t1);
Texture2D<float4> colour_atlas     : register(t2);
SamplerState samp                  : register(s0);

struct VSOut {
    float4 pos  : SV_Position;
    float2 uv   : TEXCOORD0;
    float4 tint : TEXCOORD1;
    nointerpolation uint mode : TEXCOORD2;
};

static const float2 CORNERS[6] = {
    float2(0,0), float2(1,0), float2(0,1),
    float2(0,1), float2(1,0), float2(1,1)
};

VSOut vs_main(uint vid : SV_VertexID, uint iid : SV_InstanceID) {
    Instance it = instances[iid];
    float2 c = CORNERS[vid];
    float2 px = it.pos + c * it.size;
    VSOut o;
    o.pos  = float4(px.x / viewport.x * 2.0 - 1.0, 1.0 - px.y / viewport.y * 2.0, 0.0, 1.0);
    o.uv   = lerp(it.uv0, it.uv1, c) / atlas;
    o.tint = it.tint;
    o.mode = it.mode;
    return o;
}

struct PSOut {
    float4 c0 : SV_Target0;   // premultiplied colour
    float4 c1 : SV_Target1;   // per-channel coverage, consumed via INV_SRC1_COLOR
};

PSOut ps_main(VSOut i) {
    PSOut o;
    if (i.mode == 1) {
        // Colour atlas is already premultiplied. dest = t + dest*(1-a).
        float4 t = colour_atlas.Sample(samp, i.uv);
        o.c0 = float4(t.rgb, t.a);
        o.c1 = float4(t.a, t.a, t.a, t.a);
    } else {
        float4 s = mono_atlas.Sample(samp, i.uv);
        // mode 0 takes the three subpixel coverages; mode 2 takes the greyscale average from .a.
        float3 cov = (i.mode == 0) ? s.rgb : float3(s.a, s.a, s.a);
        cov *= i.tint.a;
        o.c0 = float4(i.tint.rgb * cov, dot(cov, 1.0/3.0));
        o.c1 = float4(cov, dot(cov, 1.0/3.0));
    }
    return o;
}
"#;

fn compile(entry: &str, target: &str) -> Result<ID3DBlob> {
    let mut code = None;
    let mut errors: Option<ID3DBlob> = None;
    let entry_z = format!("{entry}\0");
    let target_z = format!("{target}\0");
    let hr = unsafe {
        D3DCompile(
            SHADER.as_ptr() as *const c_void,
            SHADER.len(),
            PCSTR(b"g4.hlsl\0".as_ptr()),
            None,
            None,
            PCSTR(entry_z.as_ptr()),
            PCSTR(target_z.as_ptr()),
            D3DCOMPILE_OPTIMIZATION_LEVEL3,
            0,
            &mut code,
            Some(&mut errors),
        )
    };
    if let Err(e) = hr {
        if let Some(err) = errors {
            let msg = unsafe {
                std::slice::from_raw_parts(
                    err.GetBufferPointer() as *const u8,
                    err.GetBufferSize(),
                )
            };
            crate::emit_err(&String::from_utf8_lossy(msg));
        }
        return Err(e);
    }
    Ok(code.expect("blob on success"))
}

pub struct Phase {
    pub name: String,
    pub draws: u32,
    pub instances: u32,
    pub cpu_ms: Vec<f64>,
    pub gpu_ms: Vec<f64>,
    pub note: String,
}

pub struct Grid {
    vs: ID3D11VertexShader,
    ps: ID3D11PixelShader,
    /// One blend state for both glyph kinds, via dual-source blending.
    blend_unified: ID3D11BlendState,
    /// The conventional straight-alpha state, for the two-pass comparison.
    blend_straight: ID3D11BlendState,
    sampler: ID3D11SamplerState,
    instance_buf: ID3D11Buffer,
    instance_srv: ID3D11ShaderResourceView,
    consts: ID3D11Buffer,
    instance_capacity: u32,

    mono: AtlasTexture,
    colour: AtlasTexture,
    cache: GlyphCache,
    fonts: Fonts,
    frame: u64,

    q_disjoint: ID3D11Query,
    q_start: ID3D11Query,
    q_end: ID3D11Query,

    pub phases: Vec<Phase>,
    done: bool,
    instances: Vec<Instance>,
    verdict: String,
    /// Text colour and clear colour. Held as state so the verification pass can force both to be
    /// exactly neutral grey — see `verify`.
    tint: [f32; 4],
    clear: [f32; 4],
}

impl Grid {
    pub fn new(gpu: &Gpu) -> Result<Self> {
        let dev = &gpu.device;

        crate::step("compile shaders");
        let vsb = compile("vs_main", "vs_5_0")?;
        let psb = compile("ps_main", "ps_5_0")?;
        let vs_bytes = unsafe {
            std::slice::from_raw_parts(vsb.GetBufferPointer() as *const u8, vsb.GetBufferSize())
        };
        let ps_bytes = unsafe {
            std::slice::from_raw_parts(psb.GetBufferPointer() as *const u8, psb.GetBufferSize())
        };
        let mut vs = None;
        let mut ps = None;
        unsafe {
            dev.CreateVertexShader(vs_bytes, None, Some(&mut vs))?;
            dev.CreatePixelShader(ps_bytes, None, Some(&mut ps))?;
        }

        // dest = src + dest * (1 - src1). Correct for per-channel coverage AND premultiplied colour.
        //
        // The alpha slots must use INV_SRC1_ALPHA, not INV_SRC1_COLOR: D3D11 rejects a *_COLOR
        // blend factor in an alpha slot with E_INVALIDARG.
        let mut rt = D3D11_RENDER_TARGET_BLEND_DESC {
            BlendEnable: true.into(),
            SrcBlend: D3D11_BLEND_ONE,
            DestBlend: D3D11_BLEND_INV_SRC1_COLOR,
            BlendOp: D3D11_BLEND_OP_ADD,
            SrcBlendAlpha: D3D11_BLEND_ONE,
            DestBlendAlpha: D3D11_BLEND_INV_SRC1_ALPHA,
            BlendOpAlpha: D3D11_BLEND_OP_ADD,
            RenderTargetWriteMask: D3D11_COLOR_WRITE_ENABLE_ALL.0 as u8,
        };
        let mut desc = D3D11_BLEND_DESC::default();
        desc.RenderTarget[0] = rt;
        let mut blend_unified = None;
        crate::step("CreateBlendState unified (dual-source)");
        unsafe { dev.CreateBlendState(&desc, Some(&mut blend_unified))? };

        rt.SrcBlend = D3D11_BLEND_SRC_ALPHA;
        rt.DestBlend = D3D11_BLEND_INV_SRC_ALPHA;
        rt.SrcBlendAlpha = D3D11_BLEND_SRC_ALPHA;
        rt.DestBlendAlpha = D3D11_BLEND_INV_SRC_ALPHA;
        let mut desc2 = D3D11_BLEND_DESC::default();
        desc2.RenderTarget[0] = rt;
        let mut blend_straight = None;
        unsafe { dev.CreateBlendState(&desc2, Some(&mut blend_straight))? };
        crate::step("CreateBlendState straight");

        let samp_desc = D3D11_SAMPLER_DESC {
            Filter: D3D11_FILTER_MIN_MAG_MIP_POINT,
            AddressU: D3D11_TEXTURE_ADDRESS_CLAMP,
            AddressV: D3D11_TEXTURE_ADDRESS_CLAMP,
            AddressW: D3D11_TEXTURE_ADDRESS_CLAMP,
            MaxLOD: f32::MAX,
            ..Default::default()
        };
        let mut sampler = None;
        unsafe { dev.CreateSamplerState(&samp_desc, Some(&mut sampler))? };

        crate::step("CreateSamplerState");
        let capacity = 65536u32;
        let (instance_buf, instance_srv) = make_structured(dev, capacity)?;

        crate::step("constant buffer");
        let cb_desc = D3D11_BUFFER_DESC {
            ByteWidth: std::mem::size_of::<Consts>() as u32,
            Usage: D3D11_USAGE_DYNAMIC,
            BindFlags: D3D11_BIND_CONSTANT_BUFFER.0 as u32,
            CPUAccessFlags: D3D11_CPU_ACCESS_WRITE.0 as u32,
            ..Default::default()
        };
        let mut consts = None;
        unsafe { dev.CreateBuffer(&cb_desc, None, Some(&mut consts))? };

        let mono = AtlasTexture::new(dev, ATLAS_W, ATLAS_H, DXGI_FORMAT_R8G8B8A8_UNORM, 4)?;
        let colour = AtlasTexture::new(dev, ATLAS_W, ATLAS_H, DXGI_FORMAT_B8G8R8A8_UNORM, 4)?;
        crate::step("atlas textures");

        crate::step("queries + fonts");
        let mk_query = |kind| -> Result<ID3D11Query> {
            let d = D3D11_QUERY_DESC {
                Query: kind,
                MiscFlags: 0,
            };
            let mut q = None;
            unsafe { dev.CreateQuery(&d, Some(&mut q))? };
            Ok(q.expect("query"))
        };

        Ok(Self {
            vs: vs.expect("vs"),
            ps: ps.expect("ps"),
            blend_unified: blend_unified.expect("blend"),
            blend_straight: blend_straight.expect("blend"),
            sampler: sampler.expect("sampler"),
            instance_buf,
            instance_srv,
            consts: consts.expect("cb"),
            instance_capacity: capacity,
            mono,
            colour,
            cache: GlyphCache::new(ATLAS_W, ATLAS_H, SLOT_W, SLOT_H, Policy::ListLru),
            fonts: Fonts::new()?,
            frame: 1,
            q_disjoint: mk_query(D3D11_QUERY_TIMESTAMP_DISJOINT)?,
            q_start: mk_query(D3D11_QUERY_TIMESTAMP)?,
            q_end: mk_query(D3D11_QUERY_TIMESTAMP)?,
            phases: Vec::new(),
            done: false,
            verdict: String::new(),
            tint: [0.87, 0.89, 0.92, 1.0],
            clear: [0.09, 0.10, 0.12, 1.0],
            instances: Vec::new(),
        })
    }

    /// Ensure a glyph is in the atlas and return its entry.
    fn ensure(&mut self, face: u16, glyph: u16, colour_wanted: bool) -> Option<GlyphEntry> {
        let key = GlyphKey {
            face,
            glyph,
            size_q: (EM * 4.0) as u16,
        };
        if let Some(e) = self.cache.map.get_mut(&key) {
            self.cache.stats.hits += 1;
            let e = *e;
            // A blank occupies no slot, so there is nothing to touch.
            if e.width == 0 {
                return Some(e);
            }
            let ta = crate::now();
            let slot = self
                .cache
                .slots
                .slot_of(e.atlas_x / SLOT_W, e.atlas_y / SLOT_H);
            self.cache.slots.touch(slot, self.frame);
            self.cache.stats.alloc_ns += ns(ta, crate::now());
            return Some(e);
        }
        self.cache.stats.misses += 1;

        let t0 = crate::now();
        let raster = if colour_wanted {
            match text::raster_colour(&self.fonts, face, glyph, EM, [1.0, 1.0, 1.0, 1.0]) {
                Ok(Some(r)) => Some(r),
                Ok(None) => text::raster_mono(&self.fonts, face, glyph, EM).ok().flatten(),
                Err(_) => None,
            }
        } else {
            text::raster_mono(&self.fonts, face, glyph, EM).ok().flatten()
        };
        let t1 = crate::now();
        self.cache.stats.raster_ns += ns(t0, t1);

        // Cache the *absence* of ink too. Without this, every space in every row is re-rasterised
        // every frame — which is how a 44-row fixture with ten spaces per row produced 440 misses
        // per frame on the first run of this experiment.
        let blank = GlyphEntry {
            atlas_x: 0,
            atlas_y: 0,
            width: 0,
            height: 0,
            left: 0,
            top: 0,
            colour: false,
        };
        let Some(raster) = raster else {
            self.cache.stats.blanks += 1;
            self.cache.map.insert(key, blank);
            return Some(blank);
        };
        if raster.is_empty() {
            self.cache.stats.blanks += 1;
            self.cache.map.insert(key, blank);
            return Some(blank);
        }
        let (w, h) = (raster.width(), raster.height());
        if h > SLOT_H || w > SLOT_W {
            // One glyph per slot is what makes eviction O(1); an oversized glyph is refused and
            // counted rather than clipped or allowed to straddle slots.
            self.cache.stats.failed += 1;
            return None;
        }
        let ta = crate::now();
        let alloc = self.cache.slots.alloc(self.frame);
        self.cache.stats.alloc_ns += ns(ta, crate::now());
        let alloc = match alloc {
            Some(a) => a,
            None => {
                self.cache.stats.failed += 1;
                return None;
            }
        };
        if let Some(old) = alloc.evicted {
            self.cache.map.remove(&old);
            self.cache.stats.evictions += 1;
        }
        let (x, y) = (alloc.col * SLOT_W, alloc.row * SLOT_H);

        let t2 = crate::now();
        let tex = if raster.colour { &self.colour } else { &self.mono };
        tex.upload(&self.ctx_of(), x, y, w, h, &raster.pixels);
        let t3 = crate::now();
        self.cache.stats.upload_ns += ns(t2, t3);

        self.cache.slots.place(alloc.slot, key, self.frame);
        let entry = GlyphEntry {
            atlas_x: x,
            atlas_y: y,
            width: w,
            height: h,
            left: raster.bounds.left,
            top: raster.bounds.top,
            colour: raster.colour,
        };
        self.cache.map.insert(key, entry);
        Some(entry)
    }

    fn ctx_of(&self) -> ID3D11DeviceContext {
        CONTEXT.with(|c| c.borrow().clone().expect("context set before render"))
    }

    fn build_instances(&mut self, rows: &[Vec<(u16, u32)>], w: u32, subpixel: bool) {
        self.instances.clear();
        let line_h = SLOT_H as f32;
        let mut y = 4.0f32;
        for row in rows {
            if y > w as f32 * 4.0 {
                break;
            }
            let mut x = 6.0f32;
            for &(face, cp) in row {
                let glyph = self.fonts.glyph_index(face, cp);
                let want_colour = face == FACE_EMOJI;
                if let Some(e) = self.ensure(face, glyph, want_colour) {
                    if e.width == 0 {
                        x += if face == FACE_MONO { 8.0 } else { 16.0 };
                        continue;
                    }
                    let mode = if e.colour {
                        MODE_COLOUR
                    } else if subpixel {
                        MODE_MONO_SUBPIXEL
                    } else {
                        MODE_MONO_GREY
                    };
                    self.instances.push(Instance {
                        pos: [x + e.left as f32, y + line_h * 0.75 + e.top as f32],
                        size: [e.width as f32, e.height as f32],
                        uv0: [e.atlas_x as f32, e.atlas_y as f32],
                        uv1: [(e.atlas_x + e.width) as f32, (e.atlas_y + e.height) as f32],
                        tint: self.tint,
                        mode,
                        _pad: [0; 3],
                    });
                }
                x += if face == FACE_MONO { 8.0 } else { 16.0 };
            }
            y += line_h;
        }
    }

    fn upload_instances(&self, ctx: &ID3D11DeviceContext) {
        let n = self.instances.len().min(self.instance_capacity as usize);
        if n == 0 {
            return;
        }
        unsafe {
            let mut mapped = Default::default();
            if ctx
                .Map(
                    &self.instance_buf,
                    0,
                    D3D11_MAP_WRITE_DISCARD,
                    0,
                    Some(&mut mapped),
                )
                .is_ok()
            {
                std::ptr::copy_nonoverlapping(
                    self.instances.as_ptr(),
                    mapped.pData as *mut Instance,
                    n,
                );
                ctx.Unmap(&self.instance_buf, 0);
            }
        }
    }

    fn set_state(&self, ctx: &ID3D11DeviceContext, w: u32, h: u32, unified: bool) {
        let c = Consts {
            viewport: [w as f32, h as f32],
            atlas: [ATLAS_W as f32, ATLAS_H as f32],
            _pad: [0.0; 4],
        };
        unsafe {
            let mut mapped = Default::default();
            if ctx
                .Map(&self.consts, 0, D3D11_MAP_WRITE_DISCARD, 0, Some(&mut mapped))
                .is_ok()
            {
                std::ptr::copy_nonoverlapping(&c, mapped.pData as *mut Consts, 1);
                ctx.Unmap(&self.consts, 0);
            }

            let vp = D3D11_VIEWPORT {
                TopLeftX: 0.0,
                TopLeftY: 0.0,
                Width: w as f32,
                Height: h as f32,
                MinDepth: 0.0,
                MaxDepth: 1.0,
            };
            ctx.RSSetViewports(Some(&[vp]));
            ctx.IASetInputLayout(None);
            ctx.IASetPrimitiveTopology(D3D_PRIMITIVE_TOPOLOGY_TRIANGLELIST);
            ctx.VSSetShader(&self.vs, None);
            ctx.PSSetShader(&self.ps, None);
            ctx.VSSetConstantBuffers(0, Some(&[Some(self.consts.clone())]));
            ctx.PSSetConstantBuffers(0, Some(&[Some(self.consts.clone())]));
            ctx.VSSetShaderResources(0, Some(&[Some(self.instance_srv.clone())]));
            ctx.PSSetShaderResources(
                1,
                Some(&[
                    Some(self.mono.srv.clone()),
                    Some(self.colour.srv.clone()),
                ]),
            );
            ctx.PSSetSamplers(0, Some(&[Some(self.sampler.clone())]));
            let blend = if unified {
                &self.blend_unified
            } else {
                &self.blend_straight
            };
            ctx.OMSetBlendState(blend, Some(&[0.0, 0.0, 0.0, 0.0]), 0xFFFF_FFFF);
        }
    }

    pub fn render(&mut self, gpu: &Gpu, w: u32, h: u32) -> Result<()> {
        CONTEXT.with(|c| *c.borrow_mut() = Some(gpu.context.clone()));
        let ctx = gpu.context.clone();
        let Some(rtv) = gpu.rtv() else { return Ok(()) };

        if !self.done {
            self.done = true;
            self.verdict = self.verify(gpu, w, h)?;
            self.run_benchmarks(gpu, w, h)?;
        }

        // Leave a representative frame on screen.
        unsafe {
            ctx.ClearRenderTargetView(rtv, &self.clear);
            ctx.OMSetRenderTargets(Some(&[Some(rtv.clone())]), None);
        }
        let rows = fixture_mixed();
        self.build_instances(&rows, w, true);
        self.upload_instances(&ctx);
        self.set_state(&ctx, w, h, true);
        unsafe { ctx.DrawInstanced(6, self.instances.len() as u32, 0, 0) };
        Ok(())
    }

    /// One measured frame.
    ///
    /// `reps` repeats the draw inside the GPU timestamp bracket; the returned GPU time is per
    /// repetition. This is necessary because a single pass over a few thousand small quads is far
    /// below the resolution of a swapchain-coupled measurement — the first version of this
    /// experiment reported ~13 ms per frame for both configurations, which was the vblank wait,
    /// not the draw. `present` is therefore also off during timed phases.
    fn one_frame(
        &mut self,
        gpu: &Gpu,
        w: u32,
        h: u32,
        rows: &[Vec<(u16, u32)>],
        unified: bool,
        reps: u32,
        present: bool,
    ) -> Result<(f64, f64, u32)> {
        let ctx = gpu.context.clone();
        let Some(rtv) = gpu.rtv() else {
            return Ok((0.0, 0.0, 0));
        };
        self.frame += 1;

        let cpu0 = crate::now();
        self.build_instances(rows, w, true);

        let mut draws = 1;
        if !unified {
            // The comparison: mono in one pass with straight alpha, colour in a second with
            // premultiplied. The instance buffer must be partitioned, so this also costs a sort.
            self.instances.sort_by_key(|i| i.mode == MODE_COLOUR);
            draws = 2;
        }
        self.upload_instances(&ctx);
        let split = self
            .instances
            .iter()
            .position(|i| i.mode == MODE_COLOUR)
            .unwrap_or(self.instances.len());
        let cpu1 = crate::now();

        unsafe {
            ctx.OMSetRenderTargets(Some(&[Some(rtv.clone())]), None);
            ctx.ClearRenderTargetView(rtv, &self.clear);
            ctx.Begin(&self.q_disjoint);
            ctx.End(&self.q_start);
            for _ in 0..reps {
                if unified {
                    self.set_state(&ctx, w, h, true);
                    ctx.DrawInstanced(6, self.instances.len() as u32, 0, 0);
                } else {
                    self.set_state(&ctx, w, h, false);
                    ctx.DrawInstanced(6, split as u32, 0, 0);
                    self.set_state(&ctx, w, h, true);
                    ctx.DrawInstanced(6, (self.instances.len() - split) as u32, 0, split as u32);
                }
            }
            ctx.End(&self.q_end);
            ctx.End(&self.q_disjoint);
        }

        let gpu_ms = self.read_gpu_time(&ctx) / reps as f64;
        if present {
            gpu.present_now()?;
        }
        Ok((
            (cpu1 - cpu0) as f64 * 1000.0 / crate::qpc_freq() as f64,
            gpu_ms,
            draws,
        ))
    }

    /// Read the frame's GPU duration in ms, or NaN if the result was disjoint or never arrived.
    ///
    /// **`GetData` returns `S_FALSE` when the result is not ready yet, and `S_FALSE` is a *success*
    /// HRESULT** — so `windows`'s `Result<()>` is `Ok` in that case and `is_err()` is useless as a
    /// readiness test. The first version of this function span on `is_err()`, exited immediately,
    /// and reported every frame as 0.000 ms. Readiness is instead detected with a sentinel: the
    /// driver only writes the buffer when it has real data.
    fn read_gpu_time(&self, ctx: &ID3D11DeviceContext) -> f64 {
        unsafe {
            let sz_dj = std::mem::size_of::<D3D11_QUERY_DATA_TIMESTAMP_DISJOINT>() as u32;
            let mut dj = D3D11_QUERY_DATA_TIMESTAMP_DISJOINT {
                Frequency: 0,
                ..Default::default()
            };
            let mut spins = 0u32;
            loop {
                let _ = ctx.GetData(
                    &self.q_disjoint,
                    Some(&mut dj as *mut _ as *mut c_void),
                    sz_dj,
                    0,
                );
                if dj.Frequency != 0 {
                    break;
                }
                spins += 1;
                if spins > 5_000_000 {
                    return f64::NAN;
                }
            }
            if dj.Disjoint.as_bool() {
                return f64::NAN;
            }
            // Once the disjoint query has resolved, the enclosed timestamps are available.
            let sz = std::mem::size_of::<u64>() as u32;
            let read = |q: &ID3D11Query| -> Option<u64> {
                let mut v = u64::MAX;
                let mut n = 0u32;
                loop {
                    let _ = ctx.GetData(q, Some(&mut v as *mut _ as *mut c_void), sz, 0);
                    if v != u64::MAX {
                        return Some(v);
                    }
                    n += 1;
                    if n > 5_000_000 {
                        return None;
                    }
                }
            };
            let (Some(a), Some(b)) = (read(&self.q_start), read(&self.q_end)) else {
                return f64::NAN;
            };
            (b.saturating_sub(a)) as f64 * 1000.0 / dj.Frequency as f64
        }
    }

    fn run_benchmarks(&mut self, gpu: &Gpu, w: u32, h: u32) -> Result<()> {
        const WARM: u32 = 20;
        const N: u32 = 120;
        const REPS: u32 = 50;

        // --- Phase 1 & 2: mixed Latin + emoji, unified vs split. The question is whether one
        // instanced draw with one blend state can serve both glyph kinds, and what the split costs.
        let mixed = fixture_mixed();
        for (name, unified) in [
            ("unified — 1 draw, 1 blend state", true),
            ("split — 2 draws, 2 blend states", false),
        ] {
            for _ in 0..WARM {
                self.one_frame(gpu, w, h, &mixed, unified, REPS, false)?;
            }
            let before = self.cache.stats;
            let mut cpu = Vec::new();
            let mut gput = Vec::new();
            let mut draws = 0;
            for _ in 0..N {
                let (c, g, d) = self.one_frame(gpu, w, h, &mixed, unified, REPS, false)?;
                cpu.push(c);
                gput.push(g);
                draws = d;
            }
            let after = self.cache.stats;
            let colour_count = self
                .instances
                .iter()
                .filter(|i| i.mode == MODE_COLOUR)
                .count();
            self.phases.push(Phase {
                name: name.to_string(),
                draws,
                instances: self.instances.len() as u32,
                cpu_ms: cpu,
                gpu_ms: gput,
                note: format!(
                    "{colour_count} colour instances, {} mono; steady state misses {}, evictions {}",
                    self.instances.len() - colour_count,
                    after.misses - before.misses,
                    after.evictions - before.evictions,
                ),
            });
        }

        // --- Phase 3 & 4: CJK overflow, once per eviction policy. Each frame draws a different
        // slice of the 20,992-glyph ideograph block, so the working set never fits and eviction
        // runs on essentially every glyph.
        let per_frame = 1500usize;
        for (label, policy) in [
            ("scan LRU (O(capacity)/miss)", Policy::ScanLru),
            ("list LRU (O(1)/miss)", Policy::ListLru),
        ] {
            self.cache.reset(ATLAS_W, ATLAS_H, SLOT_W, SLOT_H, policy);
            let cap = self.cache.slots.capacity();
            let mut cpu = Vec::new();
            let mut gput = Vec::new();
            let before = self.cache.stats;
            for f in 0..N {
                let rows = fixture_cjk(f as usize * per_frame, per_frame);
                let (c, g, _) = self.one_frame(gpu, w, h, &rows, true, 1, false)?;
                cpu.push(c);
                gput.push(g);
            }
            let after = self.cache.stats;
            let n = N as f64;
            self.phases.push(Phase {
                name: format!("CJK overflow — {label}"),
                draws: 1,
                instances: self.instances.len() as u32,
                cpu_ms: cpu,
                gpu_ms: gput,
                note: format!(
                    "capacity {cap}, occupied {}, misses {}, evictions {}, failed {} \u{2014} \
                     per frame: raster {:.2} ms, upload {:.3} ms, **eviction bookkeeping {:.4} ms**",
                    self.cache.slots.occupied(),
                    after.misses - before.misses,
                    after.evictions - before.evictions,
                    after.failed - before.failed,
                    (after.raster_ns - before.raster_ns) as f64 / 1e6 / n,
                    (after.upload_ns - before.upload_ns) as f64 / 1e6 / n,
                    (after.alloc_ns - before.alloc_ns) as f64 / 1e6 / n,
                ),
            });
        }

        Ok(())
    }

    /// Render one unified frame and inspect the pixels, so the "one draw does both" claim rests on
    /// what was drawn rather than on timings alone.
    ///
    /// Two independent signatures are counted:
    /// - **subpixel fringing** — a pixel whose channels differ markedly while it is not saturated.
    ///   Only per-channel coverage can produce this, so its presence proves ClearType survived the
    ///   blend. Greyscale AA would give R == G == B everywhere.
    /// - **colour ink** — a strongly saturated, bright pixel. The text tint is near-neutral grey,
    ///   so saturation can only have come from the colour atlas.
    fn verify(&mut self, gpu: &Gpu, w: u32, h: u32) -> Result<String> {
        // Force both the tint and the background to be *exactly* neutral grey for this frame. With
        // a neutral tint on a neutral background, any channel spread in a mono glyph can only have
        // come from per-channel coverage surviving the blend. The default tint (0.87/0.89/0.92) and
        // clear (0.09/0.10/0.12) are themselves slightly non-neutral and would have made the test
        // meaningless.
        let (tint0, clear0) = (self.tint, self.clear);
        self.tint = [0.85, 0.85, 0.85, 1.0];
        self.clear = [0.10, 0.10, 0.10, 1.0];
        self.cache.map.clear();

        let rows = fixture_mixed();
        self.one_frame(gpu, w, h, &rows, true, 1, false)?;
        let px = gpu.readback(w, h)?;

        let bg = (0.10f32 * 255.0).round() as i32;
        let mut fringed = 0u32;
        let mut coloured = 0u32;
        let mut ink = 0u32;
        let mut max_spread = 0i32;
        for i in (0..px.len()).step_by(4) {
            let (b, g, r) = (px[i] as i32, px[i + 1] as i32, px[i + 2] as i32);
            let mx = r.max(g).max(b);
            let mn = r.min(g).min(b);
            if (mx - bg).abs() <= 2 && (mn - bg).abs() <= 2 {
                continue;
            }
            ink += 1;
            let spread = mx - mn;
            if spread > 60 && mx > 90 {
                // Strongly saturated: only the colour atlas can produce this, the tint is neutral.
                coloured += 1;
            } else if spread > 8 {
                max_spread = max_spread.max(spread);
                fringed += 1;
            }
        }
        self.tint = tint0;
        self.clear = clear0;
        self.cache.map.clear();

        Ok(format!(
            "readback {w}x{h}, neutral tint and background: {ink} ink pixels; \
             {fringed} with per-channel spread >8 (max {max_spread}); \
             {coloured} strongly saturated.\n\n\
             - **ClearType subpixel coverage: {}** \u{2014} a neutral tint cannot produce channel \
             spread unless per-channel coverage survived the blend.\n\
             - **Colour glyphs: {}** \u{2014} saturation cannot come from a neutral tint, so it is \
             the premultiplied colour atlas.\n\n\
             Both in the same single instanced draw with a single blend state.",
            if fringed > 200 { "PRESENT" } else { "ABSENT" },
            if coloured > 50 { "PRESENT" } else { "ABSENT" },
        ))
    }

    pub fn report(&self, _freq: i64, driver: Driver) -> String {
        let mut s = String::new();
        s.push_str("# G4 — glyph atlas composition\n\n");
        s.push_str(&format!("driver: {}\n", driver.name()));
        s.push_str(&format!(
            "fonts: mono={}, cjk={}, emoji={}\n",
            self.fonts.names[0], self.fonts.names[1], self.fonts.names[2]
        ));
        s.push_str(&format!(
            "atlas {ATLAS_W}x{ATLAS_H}, slot {SLOT_W}x{SLOT_H}, em {EM}, slots {}\n\n",
            self.cache.slots.capacity()
        ));
        s.push_str("| phase | draws | instances | cpu p50 | cpu p99 | gpu p50 | gpu p99 | gpu max |\n");
        s.push_str("|---|---|---|---|---|---|---|---|\n");
        for p in &self.phases {
            let c = pct(&p.cpu_ms);
            let g = pct(&p.gpu_ms);
            s.push_str(&format!(
                "| {} | {} | {} | {:.3} | {:.3} | {:.3} | {:.3} | {:.3} |\n",
                p.name, p.draws, p.instances, c.0, c.1, g.0, g.1, g.2
            ));
        }
        s.push('\n');
        for p in &self.phases {
            s.push_str(&format!("- **{}** — {}\n", p.name, p.note));
        }
        s.push_str(&format!("\n## Correctness\n\n{}\n", self.verdict));
        s
    }
}

thread_local! {
    static CONTEXT: std::cell::RefCell<Option<ID3D11DeviceContext>> =
        const { std::cell::RefCell::new(None) };
}

fn ns(a: i64, b: i64) -> u64 {
    let f = crate::qpc_freq();
    ((b - a) as u128 * 1_000_000_000u128 / f as u128) as u64
}

/// (p50, p99, max), NaNs dropped.
fn pct(v: &[f64]) -> (f64, f64, f64) {
    let mut x: Vec<f64> = v.iter().copied().filter(|f| f.is_finite()).collect();
    if x.is_empty() {
        return (f64::NAN, f64::NAN, f64::NAN);
    }
    x.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let at = |q: f64| x[((x.len() as f64 - 1.0) * q).round() as usize];
    (at(0.50), at(0.99), *x.last().expect("non-empty"))
}

fn make_structured(
    dev: &ID3D11Device,
    capacity: u32,
) -> Result<(ID3D11Buffer, ID3D11ShaderResourceView)> {
    let stride = std::mem::size_of::<Instance>() as u32;
    let desc = D3D11_BUFFER_DESC {
        ByteWidth: stride * capacity,
        Usage: D3D11_USAGE_DYNAMIC,
        BindFlags: D3D11_BIND_SHADER_RESOURCE.0 as u32,
        CPUAccessFlags: D3D11_CPU_ACCESS_WRITE.0 as u32,
        MiscFlags: D3D11_RESOURCE_MISC_BUFFER_STRUCTURED.0 as u32,
        StructureByteStride: stride,
    };
    let mut buf = None;
    unsafe { dev.CreateBuffer(&desc, None, Some(&mut buf))? };
    let buf = buf.expect("buffer");
    // Buffer SRVs are described explicitly. A null desc is only inferable for textures.
    let srv_desc = D3D11_SHADER_RESOURCE_VIEW_DESC {
        Format: windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_UNKNOWN,
        ViewDimension: D3D_SRV_DIMENSION_BUFFER,
        Anonymous: D3D11_SHADER_RESOURCE_VIEW_DESC_0 {
            Buffer: D3D11_BUFFER_SRV {
                Anonymous1: D3D11_BUFFER_SRV_0 { FirstElement: 0 },
                Anonymous2: D3D11_BUFFER_SRV_1 {
                    NumElements: capacity,
                },
            },
        },
    };
    let mut srv = None;
    unsafe { dev.CreateShaderResourceView(&buf, Some(&srv_desc), Some(&mut srv))? };
    Ok((buf, srv.expect("srv")))
}

/// Mixed Latin + emoji, shaped like real log lines with a trailing status emoji.
fn fixture_mixed() -> Vec<Vec<(u16, u32)>> {
    const EMOJI: [u32; 6] = [0x2705, 0x274C, 0x26A0, 0x1F525, 0x1F680, 0x1F41B];
    let template = "2026-07-30 09:14:02,431 INFO  Tailhawk.Grid  atlas warm, viewport painted ";
    (0..44)
        .map(|r| {
            let mut row: Vec<(u16, u32)> = template
                .chars()
                .map(|c| (FACE_MONO, c as u32))
                .collect();
            row.push((FACE_EMOJI, EMOJI[r % EMOJI.len()]));
            row
        })
        .collect()
}

/// A slice of the CJK unified-ideograph block, `count` distinct glyphs starting at `start`.
fn fixture_cjk(start: usize, count: usize) -> Vec<Vec<(u16, u32)>> {
    const BASE: u32 = 0x4E00;
    const SPAN: u32 = 20_992;
    let mut rows = Vec::new();
    let mut row = Vec::new();
    for i in 0..count {
        let cp = BASE + ((start + i) as u32 % SPAN);
        row.push((FACE_CJK, cp));
        if row.len() >= 60 {
            rows.push(std::mem::take(&mut row));
        }
    }
    if !row.is_empty() {
        rows.push(row);
    }
    rows
}
