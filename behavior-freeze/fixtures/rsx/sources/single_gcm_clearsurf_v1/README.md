# single_gcm_clearsurf_v1 (GPU backend — surface + clear, end-to-end)

rsxSetSurface(16x16 A8R8G8B8 @offset 0) -> rsxSetClearColor(0xAABBCCDD) ->
rsxClearSurface(0xF3). The captured NV4097 stream decodes (replay_gcm) to a
SurfaceDescriptor + clear value/mask; rpcs3-rsx-render::execute_clear fills the
render target, which the test checks pixel-for-pixel (byte-exact: a clear writes a
constant color). Consumed by rsx_gcm_clearsurf.rs. CC0 1.0 — see LICENSE.md.
