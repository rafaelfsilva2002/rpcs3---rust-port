//! HLE backlog — cellJpgDec pixel decode. Only built with `image-decode` (the
//! decode path needs the vendored C stb_image). Boots
//! `single_jpgdec_decode_v1.self` (Create -> Open -> ReadHeader -> SetParameter
//! -> DecodeData on an embedded 16x16 JPEG) → exit 0xC0DE iff the decoded RGBA
//! checksum matches the stb_image golden.
#![cfg(feature = "image-decode")]

use std::path::PathBuf;

use rpcs3_emu_core::EmuCore;

fn fixture_dir() -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.pop();
    p.pop();
    p.push("behavior-freeze");
    p.push("fixtures");
    p.push("hle");
    p.push("sources");
    p.push("single_jpgdec_decode_v1");
    p
}

/// Calibration: decode the fixture's test.jpg via the SAME stb_image emu-core
/// uses, and print the golden RGBA checksum (hardcoded in the homebrew). Also
/// the byte-exactness anchor — the homebrew's decoded buffer must match this.
#[test]
fn jpgdec_golden_checksum() {
    let mut p = fixture_dir();
    p.push("test.jpg");
    let Ok(bytes) = std::fs::read(&p) else {
        eprintln!("[jpgdec-calib] skip: test.jpg absent");
        return;
    };
    let (w, h, rgba) = rpcs3_stb_image::decode_rgba(&bytes).expect("stb decode");
    let sum: u64 = rgba.iter().map(|&b| u64::from(b)).sum();
    eprintln!("[jpgdec-calib] w={w} h={h} rgba_len={} checksum={sum}", rgba.len());
    assert_eq!((w, h), (16, 16));
    assert_eq!(rgba.len(), 16 * 16 * 4);
}

#[test]
fn jpgdec_decode_via_real_homebrew() {
    let mut p = fixture_dir();
    p.push("single_jpgdec_decode_v1.self");
    if !p.exists() {
        eprintln!("[HLE jpgdec-decode] skip: {} not present", p.display());
        return;
    }
    let bytes = std::fs::read(&p).expect("read .self");
    let mut core = EmuCore::new();
    core.step_budget = 5_000_000;
    core.permissive_unknown_syscalls = false;
    let report = core.run_self(&bytes).expect("run_self");
    let status = report.exit_status.status;
    eprintln!(
        "[HLE jpgdec-decode] exit_status = {status} (0x{:08x})",
        status as u32,
    );
    assert_eq!(status as u32, 0xC0DE, "got 0x{:08x}", status as u32);
}
