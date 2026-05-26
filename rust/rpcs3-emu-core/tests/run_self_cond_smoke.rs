//! R10.1.g smoke test — `EmuCore::run_self` drives a PSL1GHT
//! `.self` that exercises the single-PPU-reachable subset of the
//! `sys_cond_*` family (create → signal-empty → broadcast-empty →
//! destroy → return `0xC0DE`).
//!
//! Targets syscalls #105, #106, #108, #109 (wait #107 is wired but
//! not exercised — it blocks forever on a single PPU). Sits on top
//! of the R10.3 `CondRegistry` impl.
//!
//! Skips gracefully when the fixture `.self` is absent.

use std::path::PathBuf;

use rpcs3_emu_core::{EmuCore, Error};

fn cond_self_path() -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.pop(); // rust/
    p.pop(); // rpcs3-master/
    p.push("behavior-freeze");
    p.push("fixtures");
    p.push("lv2");
    p.push("sources");
    p.push("single_cond_v1");
    p.push("single_cond_v1.self");
    p
}

#[test]
fn r10_1g_cond_round_trip_returns_canonical_status() {
    let self_path = cond_self_path();
    if !self_path.exists() {
        eprintln!(
            "[R10.1.g smoke] skip: {} not present (run Docker build first)",
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
    core.step_budget = 2_000_000;

    let result = core.run_self(&self_bytes);

    eprintln!(
        "[R10.1.g diag] post-run: CIA=0x{:08x} LR=0x{:016x} CTR=0x{:016x}",
        core.ppu.cia, core.ppu.lr, core.ppu.ctr,
    );
    eprintln!(
        "[R10.1.g diag] r3={:#x} r4={:#x} r5={:#x}",
        core.ppu.gpr[3], core.ppu.gpr[4], core.ppu.gpr[5],
    );

    match result {
        Ok(report) => {
            // Step codes (main.c):
            //   1=mutexCreate 2=condCreate 3=signal 4=broadcast
            //   5=condDestroy 6=mutexDestroy
            assert_eq!(
                report.exit_status.status, 0xC0DE,
                "expected 0xC0DE canonical exit, got 0x{:x}",
                report.exit_status.status,
            );
        }
        Err(Error::UnsupportedSyscall { number, cia }) => {
            panic!(
                "[R10.1.g smoke] reached unimplemented syscall #{number} \
                 at CIA 0x{cia:08x}. R10.1.g covered sys_cond_*; \
                 this is a new gap."
            );
        }
        Err(Error::Interpreter(ie)) => {
            panic!(
                "[R10.1.g smoke] PPU interpreter coverage gap: {ie:?}. \
                 R10 doesn't add opcodes; gap predates this wave."
            );
        }
        Err(Error::StepsExhausted) => {
            panic!(
                "[R10.1.g smoke] PPU ran 2M steps without exiting. \
                 Boot loop or cond wait block suspected."
            );
        }
        Err(e) => {
            panic!("[R10.1.g smoke] unexpected error: {e:?}");
        }
    }
}
