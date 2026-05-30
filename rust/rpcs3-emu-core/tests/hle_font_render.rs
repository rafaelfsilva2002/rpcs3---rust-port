//! R20 — cellFont byte-exact glyph rasterization. Only built with the
//! `cellfont-raster` feature (the render path needs the vendored C
//! stb_truetype). Boots `single_font_render_v1.self` (renders 'A' to a 64x64
//! surface, checksums it). Exit 0xC0DE iff the rendered surface is byte-exact.
#![cfg(feature = "cellfont-raster")]

use std::path::PathBuf;

use rpcs3_emu_core::EmuCore;

fn fixture_self() -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.pop();
    p.pop();
    p.push("behavior-freeze");
    p.push("fixtures");
    p.push("hle");
    p.push("sources");
    p.push("single_font_render_v1");
    p.push("single_font_render_v1.self");
    p
}

#[test]
fn font_render_via_real_homebrew() {
    let path = fixture_self();
    if !path.exists() {
        eprintln!("[HLE font-render] skip: {} not present", path.display());
        return;
    }
    let bytes = std::fs::read(&path).expect("read .self");
    let mut core = EmuCore::new();
    core.step_budget = 5_000_000;
    core.permissive_unknown_syscalls = false;

    let report = core.run_self(&bytes).expect("run_self");
    let status = report.exit_status.status;
    eprintln!(
        "[HLE font-render] exit_status = {status} (0x{:08x})",
        status as u32,
    );

    assert_eq!(
        status as u32,
        0xC0DE,
        "expected 0xC0DE (byte-exact rasterized surface, checksum 73114); got 0x{:08x}",
        status as u32,
    );
}
