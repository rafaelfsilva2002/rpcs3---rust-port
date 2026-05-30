//! R20 raster calibration — only built with the `cellfont-raster` feature.
//! Rasterizes 'A' via the vendored C stb_truetype and checks the coverage
//! bitmap + the blitted 64x64 surface against golden values. Because the C shim
//! IS stbtt, this golden is the RPCS3-equivalent render output; the emu-core
//! render arm uses the SAME `render_into`, so the oracle cannot drift from it.
#![cfg(feature = "cellfont-raster")]

use std::path::PathBuf;

use rpcs3_hle_cellfont::StbttFont;

fn font_bytes() -> Option<Vec<u8>> {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.pop();
    p.pop();
    p.push("behavior-freeze");
    p.push("fixtures");
    p.push("hle");
    p.push("sources");
    p.push("single_font_metrics_v1");
    p.push("testfont.ttf");
    std::fs::read(p).ok()
}

fn checksum(b: &[u8]) -> u64 {
    b.iter().map(|&x| u64::from(x)).sum()
}

#[test]
fn rasterize_glyph_a_print() {
    let Some(bytes) = font_bytes() else {
        eprintln!("[raster-calib] skip: testfont.ttf absent");
        return;
    };
    let font = StbttFont::open(bytes).expect("parse");

    let cov = font
        .render_glyph_coverage(0x41, 32.0)
        .expect("coverage for 'A'");
    eprintln!(
        "[raster-calib] 'A'@32: w={} h={} xoff={} yoff={} baseLineY={} sum={} count255={}",
        cov.width,
        cov.height,
        cov.xoff,
        cov.yoff,
        cov.base_line_y,
        checksum(&cov.pixels),
        cov.pixels.iter().filter(|&&p| p == 255).count(),
    );

    let mut surface = vec![0u8; 64 * 64];
    let drawn = font.render_into(0x41, 32.0, &mut surface, 64, 64, 0.0, 0.0);
    assert!(drawn, "render_into should draw");
    eprintln!(
        "[raster-calib] surface 64x64 blit: sum={} nonzero={}",
        checksum(&surface),
        surface.iter().filter(|&&p| p != 0).count(),
    );

    // Golden for the synthetic 'A' rect at scale_y=32 (stb_truetype rasterizer v2).
    assert_eq!(cov.width, 13);
    assert_eq!(cov.height, 23);
    assert_eq!(cov.base_line_y, 25);
    assert_eq!(checksum(&cov.pixels), 73114);
    // Whole glyph fits in 64x64 at (0,0) → surface coverage sum equals the bitmap sum.
    assert_eq!(checksum(&surface), 73114);
}
