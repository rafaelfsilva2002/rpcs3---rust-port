//! HLE backlog — cellPngDec pixel decode. Only built with `image-decode`. Boots
//! `single_pngdec_decode_v1.self` (Create -> Open -> ReadHeader -> SetParameter
//! -> DecodeData on an embedded baseline 16x16 RGB PNG) → exit 0xC0DE iff the
//! decoded RGBA checksum matches the stb_image golden (which, for a baseline PNG,
//! equals RPCS3's libpng output).
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
    p.push("single_pngdec_decode_v1");
    p
}

#[test]
fn pngdec_golden_checksum() {
    let mut p = fixture_dir();
    p.push("test.png");
    let Ok(bytes) = std::fs::read(&p) else {
        eprintln!("[pngdec-calib] skip: test.png absent");
        return;
    };
    let (w, h, rgba) = rpcs3_stb_image::decode_rgba(&bytes).expect("stb decode");
    let sum: u64 = rgba.iter().map(|&b| u64::from(b)).sum();
    eprintln!("[pngdec-calib] w={w} h={h} rgba_len={} checksum={sum}", rgba.len());
    assert_eq!((w, h), (16, 16));
    assert_eq!(rgba.len(), 16 * 16 * 4);
}

#[test]
fn pngdec_decode_via_real_homebrew() {
    let mut p = fixture_dir();
    p.push("single_pngdec_decode_v1.self");
    if !p.exists() {
        eprintln!("[HLE pngdec-decode] skip: {} not present", p.display());
        return;
    }
    let bytes = std::fs::read(&p).expect("read .self");
    let mut core = EmuCore::new();
    core.step_budget = 5_000_000;
    core.permissive_unknown_syscalls = false;
    let report = core.run_self(&bytes).expect("run_self");
    let status = report.exit_status.status;
    eprintln!(
        "[HLE pngdec-decode] exit_status = {status} (0x{:08x})",
        status as u32,
    );
    assert_eq!(status as u32, 0xC0DE, "got 0x{:08x}", status as u32);
}
