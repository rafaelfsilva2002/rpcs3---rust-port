//! R18 — cellFont byte-exact glyph metrics. Boots `single_font_metrics_v1.self`
//! (a PSL1GHT homebrew that opens a synthetic embedded font and checks
//! `cellFontGetCharGlyphMetrics` against golden bit patterns at scale 1.0 and a
//! fractional scale). Exit 0xC0DE iff every metric matches bit-for-bit.
//!
//! Skips gracefully when the `.self` is absent (gitignored; built via Docker).

use std::path::PathBuf;

use rpcs3_emu_core::EmuCore;

fn fixture_self() -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.pop(); // rust/
    p.pop(); // rpcs3-master/
    p.push("behavior-freeze");
    p.push("fixtures");
    p.push("hle");
    p.push("sources");
    p.push("single_font_metrics_v1");
    p.push("single_font_metrics_v1.self");
    p
}

#[test]
fn font_metrics_via_real_homebrew() {
    let path = fixture_self();
    if !path.exists() {
        eprintln!("[HLE font-metrics] skip: {} not present", path.display());
        return;
    }
    let bytes = std::fs::read(&path).expect("read .self");
    let mut core = EmuCore::new();
    core.step_budget = 5_000_000;
    core.permissive_unknown_syscalls = false;

    let report = core.run_self(&bytes).expect("run_self");
    let status = report.exit_status.status;
    eprintln!(
        "[HLE font-metrics] exit_status = {status} (0x{:08x})",
        status as u32,
    );

    assert_eq!(
        status as u32,
        0xC0DE,
        "expected 0xC0DE (byte-exact glyph metrics both scales); got 0x{:08x}",
        status as u32,
    );
}
