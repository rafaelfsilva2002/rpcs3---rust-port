//! R12.10a — RSX GCM command-stream replay oracle (authored golden).
//!
//! Builds an **authored** GCM command stream faithful to the NV4097
//! method encoding `libgcm` emits for a realistic frame (surface
//! setup → clear → vertex/index/texture setup → triangle draw →
//! frame-end semaphore), replays it through the pure decode → state →
//! descriptor pipeline ([`replay_gcm`]), and asserts the **complete**
//! [`RsxSnapshot`]: draw calls, control effects, and every resource
//! descriptor.
//!
//! Provenance: this stream is **authored from the method-encoding
//! spec**, NOT captured from real hardware/RPCS3. It is the closed
//! test rail that freezes the pure decoder. A future R12.10b/R12.11
//! will feed a *real captured* stream through the same `replay_gcm`
//! harness + the same `RsxSnapshot` comparison shape; if that breaks,
//! this golden oracle isolates decoder bugs from capture/cellGcm-HLE
//! bugs.
//!
//! The committed text fixture
//! `behavior-freeze/fixtures/rsx/streams/frame_clear_textured_draw_v1.gcmhex`
//! mirrors `golden_frame()`; the `golden_hex_fixture_matches_code`
//! test guards them against drift.

use std::path::PathBuf;

use rpcs3_rsx_state::*;

/// Emit a single-argument increment method write (`header` + `arg`).
fn m(words: &mut Vec<u32>, reg: u32, arg: u32) {
    // increment method, count 1: header = (1<<18) | (reg<<2).
    words.push((1 << 18) | (reg << 2));
    words.push(arg);
}

/// Texture FORMAT word: location, cubemap, border, dimension,
/// format code, mip count.
fn tex_fmt(loc: u32, dim: u32, fmt: u32, mips: u32) -> u32 {
    (loc & 0x3) | ((dim & 0xF) << 4) | ((fmt & 0xFF) << 8) | ((mips & 0xFFFF) << 16)
}

/// Vertex array FORMAT word: type, count, stride, frequency.
fn vtx_fmt(ty: u32, count: u32, stride: u32, freq: u32) -> u32 {
    (ty & 0xF) | ((count & 0xF) << 4) | ((stride & 0xFF) << 8) | ((freq & 0xFFFF) << 16)
}

/// The authored golden frame as a list of big-endian command words.
fn golden_frame_words() -> Vec<u32> {
    let mut w = Vec::new();

    // --- surface / render-target setup ---
    m(&mut w, SURFACE_CLIP_HORIZONTAL, 1280 << 16);
    m(&mut w, SURFACE_CLIP_VERTICAL, 720 << 16);
    m(&mut w, SURFACE_FORMAT, 0x08 | (0x2 << 5)); // color 0x08, depth 0x2
    m(&mut w, SURFACE_COLOR_TARGET, 1); // target A
    m(&mut w, SURFACE_COLOR_A_OFFSET, 0x0100_0000);
    m(&mut w, SURFACE_PITCH_A, 0x1400); // 1280 * 4
    m(&mut w, SURFACE_ZETA_OFFSET, 0x0050_0000);
    m(&mut w, SURFACE_PITCH_Z, 0x1400);

    // --- clear ---
    m(&mut w, COLOR_CLEAR_VALUE, 0xFF20_2020);
    m(&mut w, CLEAR_SURFACE, 0xF3); // color + depth + stencil

    // --- vertex attribute arrays ---
    m(&mut w, VERTEX_DATA_ARRAY_FORMAT, vtx_fmt(2, 3, 12, 0)); // attr0: F x3
    m(&mut w, VERTEX_DATA_ARRAY_OFFSET, 0x8000_1000); // main memory
    m(&mut w, VERTEX_DATA_ARRAY_FORMAT + 1, vtx_fmt(4, 4, 4, 0)); // attr1: UB x4
    m(&mut w, VERTEX_DATA_ARRAY_OFFSET + 1, 0x8000_2000);

    // --- index array ---
    m(&mut w, INDEX_ARRAY_ADDRESS, 0x0010_0000);
    m(&mut w, INDEX_ARRAY_DMA, 0x10); // u16

    // --- texture unit 0 ---
    m(&mut w, TEXTURE_OFFSET_BASE, 0x0020_0000);
    m(&mut w, TEXTURE_FORMAT_BASE, tex_fmt(2, 2, 0x85, 3)); // 2D, fmt 0x85, 3 mips
    m(&mut w, TEXTURE_CONTROL0_BASE, 0x8000_0000); // enable
    m(&mut w, TEXTURE_IMAGE_RECT_BASE, (256 << 16) | 256);

    // --- draw ---
    m(&mut w, BEGIN_END, 5); // triangles
    m(&mut w, DRAW_ARRAYS, 0x0200_0000); // first 0, count 3
    m(&mut w, BEGIN_END, 0); // end

    // --- frame-end semaphore release ---
    m(&mut w, SEMAPHORE_RELEASE, 0x1234_5678);

    w
}

/// Serialize words to a big-endian byte buffer.
fn to_be_bytes(words: &[u32]) -> Vec<u8> {
    let mut v = Vec::with_capacity(words.len() * 4);
    for x in words {
        v.extend_from_slice(&x.to_be_bytes());
    }
    v
}

/// The expected snapshot for the golden frame.
fn expected_snapshot() -> RsxSnapshot {
    RsxSnapshot {
        draw_calls: vec![DrawCall {
            primitive: 5,
            kind: DrawKind::Arrays,
            ranges: vec![(0, 3)],
        }],
        effects: vec![
            MethodEffect::ClearSurface(0xF3),
            MethodEffect::BeginEnd(5),
            MethodEffect::BeginEnd(0),
            MethodEffect::SemaphoreRelease(0x1234_5678),
        ],
        vertex_attributes: vec![
            (
                0,
                VertexAttribute {
                    base_type: VertexBaseType::F,
                    count: 3,
                    stride: 12,
                    frequency: 0,
                    offset: 0x8000_1000,
                },
            ),
            (
                1,
                VertexAttribute {
                    base_type: VertexBaseType::Ub,
                    count: 4,
                    stride: 4,
                    frequency: 0,
                    offset: 0x8000_2000,
                },
            ),
        ],
        index_array: IndexArray {
            address: 0x0010_0000,
            index_type: IndexType::U16,
            location: 0,
        },
        textures: vec![(
            0,
            TextureDescriptor {
                format_code: 0x85,
                dimension: TextureDimension::TwoD,
                mipmap_levels: 3,
                width: 256,
                height: 256,
                cubemap: false,
                border: false,
                location: 2,
                offset: 0x0020_0000,
            },
        )],
        surface: SurfaceDescriptor {
            color_format: 0x08,
            depth_format: 0x2,
            antialias: 0,
            targets: SurfaceTargets(1),
            color_offset: [0x0100_0000, 0, 0, 0],
            color_pitch: [0x1400, 0, 0, 0],
            zeta_offset: 0x0050_0000,
            zeta_pitch: 0x1400,
            clip: (1280, 720),
        },
    }
}

#[test]
fn golden_frame_replays_to_expected_snapshot() {
    let words = golden_frame_words();
    let bytes = to_be_bytes(&words);
    let put = bytes.len() as u32;
    let snap = replay_gcm(&bytes, put).expect("replay");
    assert_eq!(snap, expected_snapshot());
}

fn fixture_path() -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.pop(); // rust/
    p.pop(); // rpcs3-master/
    p.push("behavior-freeze");
    p.push("fixtures");
    p.push("rsx");
    p.push("streams");
    p.push("frame_clear_textured_draw_v1.gcmhex");
    p
}

/// Parse a `.gcmhex` text fixture: one hex u32 per line, `#` comments
/// and blank lines ignored.
fn parse_gcmhex(text: &str) -> Vec<u32> {
    text.lines()
        .map(|l| l.split('#').next().unwrap_or("").trim())
        .filter(|l| !l.is_empty())
        .map(|l| u32::from_str_radix(l.trim_start_matches("0x"), 16).expect("hex word"))
        .collect()
}

#[test]
fn golden_hex_fixture_matches_code() {
    let path = fixture_path();
    if !path.exists() {
        eprintln!(
            "[R12.10a] skip fixture sync check: {} not present",
            path.display()
        );
        return;
    }
    let text = std::fs::read_to_string(&path).expect("read fixture");
    let from_fixture = parse_gcmhex(&text);
    assert_eq!(
        from_fixture,
        golden_frame_words(),
        "committed .gcmhex drifted from golden_frame_words()"
    );
    // And it replays to the same expected snapshot.
    let bytes = to_be_bytes(&from_fixture);
    let put = bytes.len() as u32;
    let snap = replay_gcm(&bytes, put).expect("replay fixture");
    assert_eq!(snap, expected_snapshot());
}

/// Helper to (re)generate the `.gcmhex` fixture body. Run with
/// `--nocapture` and paste the output into the fixture file. Ignored
/// by default so it doesn't spam normal runs.
#[test]
#[ignore]
fn emit_golden_hex() {
    for x in golden_frame_words() {
        println!("0x{x:08x}");
    }
}
