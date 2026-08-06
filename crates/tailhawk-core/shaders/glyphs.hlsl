// The glyph pass — `SPEC.md` §3.2's one instanced draw for the whole viewport.
//
// Derived from `experiments/g4-glyph-atlas`, which refuted the assumption that monochrome ClearType
// text and premultiplied colour emoji need two passes. The pixel shader emits **two** outputs and
// dual-source blending consumes the second:
//
//     dest = c0 + dest * (1 - c1)
//
// with SrcBlend = ONE and DestBlend = INV_SRC1_COLOR. That one equation is simultaneously the
// correct per-channel (ClearType) blend for a monochrome glyph whose mono path premultiplies here in
// the shader, and the correct premultiplied composite for a colour bitmap. Single-source straight
// alpha cannot do this: the blend equation has one alpha per pixel, so it cannot consume three
// independent coverages.
//
// Per-instance `mode` selects between them, so both glyph kinds live in one instance buffer and one
// DrawInstanced call, and per-token colouring is free.

cbuffer Consts : register(b0) {
    float2 viewport;   // client size in device pixels
    float2 atlas;      // sheet size in texels
    float4 pad_;
};

struct Instance {
    float2 pos;        // top-left in device pixels
    float2 size;       // quad size in device pixels
    float2 uv0;        // top-left in sheet texels
    float2 uv1;        // bottom-right in sheet texels
    float4 tint;       // rgb = foreground, a = extra coverage multiplier
    uint mode;
    uint3 pad_;
};

StructuredBuffer<Instance> instances : register(t0);
Texture2D<float4> mono_atlas         : register(t1);
// Bound only once the colour path exists. An unbound SRV samples as zero, so a stray MODE_COLOUR
// instance draws nothing rather than drawing garbage.
Texture2D<float4> colour_atlas       : register(t2);
SamplerState samp                    : register(s0);

struct VSOut {
    float4 pos  : SV_Position;
    float2 uv   : TEXCOORD0;
    float4 tint : TEXCOORD1;
    nointerpolation uint mode : TEXCOORD2;
};

// Two triangles per instance, from SV_VertexID. No vertex buffer and no index buffer: a glyph quad
// is four numbers, and reading them from a structured buffer keeps the whole viewport in one draw.
static const float2 CORNERS[6] = {
    float2(0, 0), float2(1, 0), float2(0, 1),
    float2(0, 1), float2(1, 0), float2(1, 1)
};

VSOut vs_main(uint vid : SV_VertexID, uint iid : SV_InstanceID) {
    Instance it = instances[iid];
    float2 corner = CORNERS[vid];
    float2 px = it.pos + corner * it.size;

    VSOut o;
    o.pos = float4(px.x / viewport.x * 2.0 - 1.0, 1.0 - px.y / viewport.y * 2.0, 0.0, 1.0);
    o.uv = lerp(it.uv0, it.uv1, corner) / atlas;
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
        // Already premultiplied by the colour rasteriser. dest = t + dest * (1 - a).
        float4 t = colour_atlas.Sample(samp, i.uv);
        o.c0 = float4(t.rgb, t.a);
        o.c1 = float4(t.a, t.a, t.a, t.a);
    } else {
        float4 s = mono_atlas.Sample(samp, i.uv);
        // mode 0 takes the three subpixel coverages; mode 2 takes the greyscale average from .a,
        // which the rasteriser wrote there so one sheet serves both.
        float3 cov = (i.mode == 0) ? s.rgb : float3(s.a, s.a, s.a);
        cov *= i.tint.a;
        o.c0 = float4(i.tint.rgb * cov, dot(cov, 1.0 / 3.0));
        o.c1 = float4(cov, dot(cov, 1.0 / 3.0));
    }
    return o;
}
