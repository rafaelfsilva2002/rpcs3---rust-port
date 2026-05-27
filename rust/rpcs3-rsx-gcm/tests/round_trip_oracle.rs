//! R12.10b — GCM emit ↔ decode round-trip oracle.
//!
//! Builds a frame via [`GcmContext`] (the ported libgcm command
//! emission — a Tier-2 *emitted* stream, not hand-authored hex),
//! serializes it, replays it through `rpcs3_rsx_state::replay_gcm`,
//! and asserts the decoded [`RsxSnapshot`] matches what was emitted.
//!
//! This closes the emit↔decode loop: the producer (this crate) and
//! the decoder (`rpcs3-rsx-fifo`/`-state`) are proven inverse over a
//! realistic frame. A future Tier-3 capture (real PSL1GHT libgcm via
//! a cellGcm HLE, or RPCS3 `.rrc`) reuses the same `replay_gcm` +
//! `RsxSnapshot` comparison shape.

use rpcs3_rsx_gcm::*;
use rpcs3_rsx_state::*;

/// Emit the same realistic frame as the R12.10a golden, but via the
/// high-level GcmContext calls instead of hand-authored words.
fn emit_frame() -> GcmContext {
    let mut c = GcmContext::new();
    c.set_surface_clip(1280, 720);
    c.set_surface(
        0x08,        // color format
        0x02,        // depth format
        1,           // target A
        0x0100_0000, // color A offset
        0x1400,      // color A pitch
        0x0050_0000, // zeta offset
        0x1400,      // zeta pitch
    );
    c.set_clear_color(0xFF20_2020);
    c.clear_surface(0xF3);
    c.set_vertex_data_array(0, VTX_TYPE_FLOAT, 3, 12, 0x8000_1000);
    c.set_vertex_data_array(1, VTX_TYPE_UNORM8, 4, 4, 0x8000_2000);
    c.set_index_array(0x0010_0000, true); // u16
    c.set_texture(0, 0x0020_0000, 2, 2, 0x85, 3, 256, 256);
    c.draw_arrays(5, 0, 3); // triangles
    c.semaphore_release(0x1234_5678);
    c
}

#[test]
fn emit_then_decode_round_trip() {
    let ctx = emit_frame();
    let bytes = ctx.finish();
    let snap = replay_gcm(&bytes, ctx.put()).expect("replay emitted stream");

    // The emitted stream decodes to the expected descriptors.
    assert_eq!(snap.draw_calls.len(), 1);
    assert_eq!(snap.draw_calls[0].primitive, 5);
    assert_eq!(snap.draw_calls[0].kind, DrawKind::Arrays);
    assert_eq!(snap.draw_calls[0].ranges, vec![(0, 3)]);

    assert_eq!(
        snap.effects,
        vec![
            MethodEffect::ClearSurface(0xF3),
            MethodEffect::BeginEnd(5),
            MethodEffect::BeginEnd(0),
            MethodEffect::SemaphoreRelease(0x1234_5678),
        ]
    );

    assert_eq!(snap.vertex_attributes.len(), 2);
    assert_eq!(snap.vertex_attributes[0].1.base_type, VertexBaseType::F);
    assert_eq!(snap.vertex_attributes[0].1.count, 3);
    assert_eq!(snap.vertex_attributes[1].1.base_type, VertexBaseType::Ub);

    assert_eq!(snap.index_array.index_type, IndexType::U16);
    assert_eq!(snap.index_array.address, 0x0010_0000);

    assert_eq!(snap.textures.len(), 1);
    let (unit, tex) = snap.textures[0];
    assert_eq!(unit, 0);
    assert_eq!(tex.format_code, 0x85);
    assert_eq!(tex.dimension, TextureDimension::TwoD);
    assert_eq!(tex.mipmap_levels, 3);
    assert_eq!(tex.width, 256);
    assert_eq!(tex.height, 256);

    assert_eq!(snap.surface.color_format, 0x08);
    assert_eq!(snap.surface.depth_format, 0x02);
    assert_eq!(snap.surface.targets, SurfaceTargets(1));
    assert_eq!(snap.surface.color_offset[0], 0x0100_0000);
    assert_eq!(snap.surface.clip, (1280, 720));
}

#[test]
fn emitted_stream_equals_golden_words() {
    // The Tier-2 emitted stream must be byte-identical to the
    // R12.10a hand-authored golden words — proving the emitter and
    // the authored fixture agree on the wire encoding.
    let golden: Vec<u32> = include_str!(
        "../../../behavior-freeze/fixtures/rsx/streams/frame_clear_textured_draw_v1.gcmhex"
    )
    .lines()
    .map(|l| l.split('#').next().unwrap_or("").trim())
    .filter(|l| !l.is_empty())
    .map(|l| u32::from_str_radix(l.trim_start_matches("0x"), 16).unwrap())
    .collect();

    assert_eq!(emit_frame().words(), golden.as_slice());
}

#[test]
fn empty_context_replays_to_empty_snapshot() {
    let ctx = GcmContext::new();
    let snap = replay_gcm(&ctx.finish(), ctx.put()).expect("replay empty");
    assert!(snap.draw_calls.is_empty());
    assert!(snap.effects.is_empty());
    assert!(snap.vertex_attributes.is_empty());
    assert!(snap.textures.is_empty());
}
