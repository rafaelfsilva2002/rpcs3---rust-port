//! R17 — cellFont init lifecycle. Boots `single_font_init_v1.self` (a PSL1GHT
//! homebrew exercising `cellFontInitializeWithRevision` (fc_size>=24 invariant)
//! + `cellFontEnd`). No glyph rendering. Exit 0xC0DE iff the invariant holds
//! both ways (size<24 rejected, size>=24 accepted) and end succeeds.
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
    p.push("single_font_init_v1");
    p.push("single_font_init_v1.self");
    p
}

#[test]
fn font_init_lifecycle_via_real_homebrew() {
    let path = fixture_self();
    if !path.exists() {
        eprintln!("[HLE font-init] skip: {} not present", path.display());
        return;
    }
    let bytes = std::fs::read(&path).expect("read .self");
    let mut core = EmuCore::new();
    core.step_budget = 5_000_000;
    core.permissive_unknown_syscalls = false;

    let report = core.run_self(&bytes).expect("run_self");
    let status = report.exit_status.status;
    eprintln!(
        "[HLE font-init] exit_status = {status} (0x{:08x})",
        status as u32,
    );

    assert_eq!(
        status as u32,
        0xC0DE,
        "expected 0xC0DE (fc_size>=24 invariant enforced both ways + \
         cellFontEnd OK); got 0x{:08x}",
        status as u32,
    );
}
