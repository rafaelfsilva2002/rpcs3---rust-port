//! R10.1.d smoke test — `EmuCore::run_self` drives a PSL1GHT
//! `.self` that exercises the lwmutex syscall family
//! end-to-end (create → lock → unlock → destroy → return
//! `0xC0DE`).
//!
//! This is a behavioural smoke against the R10.1.b NID wiring,
//! not a byte-exact replay oracle (no JSONL trace; that work
//! requires extending the C++ capture writer, deferred).
//!
//! Skips gracefully when the fixture `.self` is absent — the
//! file is gitignored (`.self` extension blocked by the
//! path-lock hook) and built locally via the Docker toolchain
//! described in `behavior-freeze/fixtures/lv2/sources/single_lwmutex_v1/README.md`.

use std::path::PathBuf;

use rpcs3_emu_core::{EmuCore, Error};

fn lwmutex_self_path() -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.pop(); // rust/
    p.pop(); // rpcs3-master/
    p.push("behavior-freeze");
    p.push("fixtures");
    p.push("lv2");
    p.push("sources");
    p.push("single_lwmutex_v1");
    p.push("single_lwmutex_v1.self");
    p
}

#[test]
fn r10_1d_lwmutex_round_trip_returns_canonical_status() {
    let self_path = lwmutex_self_path();
    if !self_path.exists() {
        eprintln!(
            "[R10.1.d smoke] skip: {} not present (run Docker build first)",
            self_path.display()
        );
        return;
    }

    let self_bytes = std::fs::read(&self_path).expect("read .self");
    assert!(
        self_bytes.len() >= 4 && &self_bytes[..4] == b"SCE\0",
        ".self missing SCE magic"
    );

    let mut core = EmuCore::new();
    // Give the PPU enough budget to run PSL1GHT's crt0 + main.
    // The R9 mailbox_v1 path hit ~1M steps; lwmutex is shorter
    // (no SPU lifecycle) but the budget stays generous.
    core.step_budget = 2_000_000;

    let result = core.run_self(&self_bytes);

    // Diagnostic dump (matches R9.1f format) so a CI failure
    // surfaces enough context to debug without a re-run.
    eprintln!(
        "[R10.1.d diag] post-run: CIA=0x{:08x} LR=0x{:016x} CTR=0x{:016x}",
        core.ppu.cia, core.ppu.lr, core.ppu.ctr,
    );
    eprintln!(
        "[R10.1.d diag] r3={:#x} r4={:#x} r5={:#x}",
        core.ppu.gpr[3], core.ppu.gpr[4], core.ppu.gpr[5],
    );

    match result {
        Ok(report) => {
            // Canonical success: main returned 0xC0DE.
            assert_eq!(
                report.exit_status.status, 0xC0DE,
                "expected 0xC0DE canonical exit, got 0x{:x} (1=create, \
                 2=lock, 3=unlock, 4=destroy failure)",
                report.exit_status.status,
            );
        }
        Err(Error::UnsupportedSyscall { number, cia }) => {
            // R10.1.b only wired the lwmutex NIDs; if PSL1GHT crt0
            // takes a path through another unimplemented syscall,
            // surface it here so future slices know where to wire.
            panic!(
                "[R10.1.d smoke] reached unimplemented syscall #{number} \
                 at CIA 0x{cia:08x}. R10.1.b should have covered \
                 lwmutex_*; this is a new gap."
            );
        }
        Err(Error::Interpreter(ie)) => {
            // PPU interpreter coverage gap — not an R10 regression,
            // but worth surfacing.
            panic!(
                "[R10.1.d smoke] PPU interpreter coverage gap: {ie:?}. \
                 R10 doesn't add opcodes; gap predates this wave."
            );
        }
        Err(Error::StepsExhausted) => {
            panic!(
                "[R10.1.d smoke] PPU ran 2M steps without exiting. \
                 Boot loop or lwmutex contention block suspected."
            );
        }
        Err(e) => {
            panic!("[R10.1.d smoke] unexpected error: {e:?}");
        }
    }
}
