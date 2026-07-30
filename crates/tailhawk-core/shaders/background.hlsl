// The M0 background fill.
//
// This exists to prove the offline-compile-and-embed path end to end before M3 depends on it:
// fxc at build time, DXBC in the binary, no d3dcompiler_47.dll at runtime (SPEC.md §3.2).
//
// The colour arrives in a constant buffer rather than being written here, so that
// tailhawk_core::BACKGROUND stays the single source of truth. Both stages of the two-stage first
// paint have to agree exactly or the handover is visible, and a colour duplicated into HLSL is a
// third place for it to drift.

cbuffer Frame : register(b0)
{
    float4 background;
};

struct VsOut
{
    float4 pos : SV_Position;
};

// A fullscreen triangle synthesised from the vertex id, so no vertex or index buffer is bound:
// id 0,1,2 map to (-1,1), (3,1), (-1,-3) in clip space, which covers the viewport with one
// primitive and no interpolation seam down the diagonal that two triangles would have.
VsOut vs_main(uint id : SV_VertexID)
{
    float2 uv = float2((id << 1) & 2, id & 2);
    VsOut o;
    o.pos = float4(uv * float2(2.0, -2.0) + float2(-1.0, 1.0), 0.0, 1.0);
    return o;
}

float4 ps_main(VsOut i) : SV_Target
{
    return background;
}
