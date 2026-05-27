//! R12.11b — real captured RSX stream oracle (Tier 3).
//!
//! Runs the PSL1GHT fixture `single_gcm_clear_v1.self` through
//! `EmuCore::run_self`. The fixture emits a frame using PSL1GHT's
//! **real** librsx command-emission (`rsxSetClearColor` /
//! `rsxClearSurface` / `rsxSetWriteCommandLabel`) into a manual
//! command buffer, then dumps the command words as hex via
//! `sysTtyWrite`. We capture that TTY hex, parse it back to the GCM
//! byte stream, and decode it with `rpcs3_rsx_state::replay_gcm`.
//!
//! This is the Tier-3 byte origin: the command words come from
//! PSL1GHT's own libgcm running in our emulator — not hand-authored
//! (R12.10a) and not our Rust `GcmContext` (R12.10b). The decoder
//! (frozen by those golden oracles) now validates against real bytes.
//!
//! Skips gracefully when the fixture `.self` is absent (built locally
//! via the Docker PSL1GHT toolchain).

use std::path::PathBuf;

use rpcs3_emu_core::EmuCore;
use rpcs3_rsx_state::{replay_gcm, MethodEffect};

fn fixture_self() -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.pop(); // rust/
    p.pop(); // rpcs3-master/
    p.push("behavior-freeze");
    p.push("fixtures");
    p.push("rsx");
    p.push("sources");
    p.push("single_gcm_clear_v1");
    p.push("single_gcm_clear_v1.self");
    p
}

/// Parse the fixture's TTY hex dump (one 8-digit word per line) into
/// big-endian GCM stream bytes.
fn hex_lines_to_bytes(text: &str) -> Vec<u8> {
    let mut bytes = Vec::new();
    for line in text.lines() {
        let l = line.trim();
        if l.is_empty() {
            continue;
        }
        if let Ok(w) = u32::from_str_radix(l, 16) {
            bytes.extend_from_slice(&w.to_be_bytes());
        }
    }
    bytes
}

#[test]
fn captured_gcm_stream_decodes_to_clear_effect() {
    let self_path = fixture_self();
    if !self_path.exists() {
        eprintln!(
            "[R12.11b] skip: {} not present (build via Docker PSL1GHT)",
            self_path.display()
        );
        return;
    }

    let self_bytes = std::fs::read(&self_path).expect("read .self");
    let mut core = EmuCore::new();
    core.step_budget = 2_000_000;
    let report = core.run_self(&self_bytes).expect("run_self");

    eprintln!(
        "[R12.11b] exit_status=0x{:x}",
        report.exit_status.status
    );
    assert_eq!(
        report.exit_status.status, 0xC0DE,
        "fixture should reach its 0xC0DE success return"
    );

    // The fixture dumped the command words as hex on TTY channel 0.
    let tty = report
        .tty_output
        .get(0)
        .cloned()
        .unwrap_or_default();
    let stream = hex_lines_to_bytes(&tty);
    assert!(
        !stream.is_empty(),
        "expected a captured GCM hex dump on TTY ch0, got: {tty:?}"
    );
    assert_eq!(stream.len() % 4, 0, "stream must be whole words");

    // Decode the REAL captured stream through the frozen decoder.
    let put = stream.len() as u32;
    let snap = replay_gcm(&stream, put).expect("replay captured stream");

    // PSL1GHT's rsxClearSurface emitted NV4097_CLEAR_SURFACE(0xF3);
    // the decoder must surface it as a ClearSurface effect.
    assert!(
        snap.effects
            .iter()
            .any(|e| matches!(e, MethodEffect::ClearSurface(0xF3))),
        "captured stream should contain ClearSurface(0xF3); effects={:?}",
        snap.effects
    );

    // rsxSetClearColor(0xff202020) set the clear-color register.
    // (Read it back via the snapshot's surface path is indirect; the
    // ClearSurface effect above is the load-bearing assertion. The
    // stream decoding without error is itself the Tier-3 result.)
    eprintln!(
        "[R12.11b] captured {} words, {} effects, {} draw calls",
        stream.len() / 4,
        snap.effects.len(),
        snap.draw_calls.len(),
    );
}
