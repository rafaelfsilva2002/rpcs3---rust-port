//! R18 calibration — proves the `stb_truetype` crate reproduces the golden
//! glyph metrics bit-for-bit. The golden values come from the synthetic CC0
//! font's design (font-unit integers at scale 1.0) and an IEEE-754 single
//! reference (Python `struct`, identical to host f32) at a fractional scale.
//! If the crate ever diverges from this reference, the byte-exact contract with
//! RPCS3's stb_truetype is broken and this test fails loudly.
//!
//! Golden produced by `behavior-freeze/.../single_font_metrics_v1/gen_font.py`.

use std::path::PathBuf;

use rpcs3_hle_cellfont::StbttFont;

fn font_bytes() -> Option<Vec<u8>> {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.pop(); // rust/
    p.pop(); // rpcs3-master/
    p.push("behavior-freeze");
    p.push("fixtures");
    p.push("hle");
    p.push("sources");
    p.push("single_font_metrics_v1");
    p.push("testfont.ttf");
    std::fs::read(p).ok()
}

#[test]
fn metrics_match_golden_scale_one() {
    let Some(bytes) = font_bytes() else {
        eprintln!("[font-calib] skip: testfont.ttf absent (run gen_font.py)");
        return;
    };
    let font = StbttFont::open(bytes).expect("stbtt parse");

    // scale_y == ascent-descent (1000) → scale 1.0 → metrics == font-unit ints.
    let a = font.char_glyph_metrics(0x41, 1000.0); // 'A' rect (100,0,500,700) adv 600
    assert_eq!(a.width, 400.0);
    assert_eq!(a.height, 700.0);
    assert_eq!(a.h_bearing_x, 100.0);
    assert_eq!(a.h_advance, 600.0);
    assert_eq!(a.h_bearing_y, 0.0);
    assert_eq!(a.v_advance, 0.0);

    let b = font.char_glyph_metrics(0x42, 1000.0); // 'B' rect (50,0,450,800) adv 550
    assert_eq!(b.width, 400.0);
    assert_eq!(b.height, 800.0);
    assert_eq!(b.h_bearing_x, 50.0);
    assert_eq!(b.h_advance, 550.0);
}

#[test]
fn horizontal_layout_print_and_check() {
    let Some(bytes) = font_bytes() else {
        eprintln!("[font-calib] skip: testfont.ttf absent");
        return;
    };
    let font = StbttFont::open(bytes).expect("stbtt parse");
    // scale_y = 1000 -> scale 1.0. ascent=800, descent=-200, lineGap=0 (design).
    let l = font.horizontal_layout(1000.0);
    assert_eq!(l.base_line_y, 800.0); // ascent
    assert_eq!(l.line_height, 1000.0); // ascent - descent + lineGap
    assert_eq!(l.effect_height, 0.0); // lineGap
}

#[test]
fn metrics_match_golden_scale_32_bit_exact() {
    let Some(bytes) = font_bytes() else {
        eprintln!("[font-calib] skip: testfont.ttf absent");
        return;
    };
    let font = StbttFont::open(bytes).expect("stbtt parse");

    // 'A' at scale_y=32 → scale = 32/1000; bit patterns from the IEEE reference.
    let a = font.char_glyph_metrics(0x41, 32.0);
    assert_eq!(a.width.to_bits(), 0x414c_cccd, "A.width");
    assert_eq!(a.height.to_bits(), 0x41b3_3334, "A.height");
    assert_eq!(a.h_bearing_x.to_bits(), 0x404c_cccd, "A.h_bearingX");
    assert_eq!(a.h_advance.to_bits(), 0x4199_999a, "A.h_advance");

    let b = font.char_glyph_metrics(0x42, 32.0);
    assert_eq!(b.width.to_bits(), 0x414c_cccd, "B.width");
    assert_eq!(b.height.to_bits(), 0x41cc_cccd, "B.height");
    assert_eq!(b.h_bearing_x.to_bits(), 0x3fcc_cccd, "B.h_bearingX");
    assert_eq!(b.h_advance.to_bits(), 0x418c_cccd, "B.h_advance");
}
