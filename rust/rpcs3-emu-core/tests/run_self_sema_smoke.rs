//! R10.1.e smoke test — `EmuCore::run_self` drives a PSL1GHT
//! `.self` that exercises the kernel `sys_semaphore_*` syscall
//! family end-to-end (create → wait → post → trywait → get_value
//! → post → get_value → destroy → return `0xC0DE`).
//!
//! Targets syscalls #90-#94 + #114, wired into the dispatcher by
//! this same slice on top of the R10.4 `SyncTable` (sema half)
//! impl.
//!
//! Skips gracefully when the fixture `.self` is absent.

use std::path::PathBuf;

use rpcs3_emu_core::{EmuCore, Error};

fn sema_self_path() -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.pop(); // rust/
    p.pop(); // rpcs3-master/
    p.push("behavior-freeze");
    p.push("fixtures");
    p.push("lv2");
    p.push("sources");
    p.push("single_sema_v1");
    p.push("single_sema_v1.self");
    p
}

#[test]
fn r10_1e_kernel_sema_round_trip_returns_canonical_status() {
    let self_path = sema_self_path();
    if !self_path.exists() {
        eprintln!(
            "[R10.1.e smoke] skip: {} not present (run Docker build first)",
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
        "[R10.1.e diag] post-run: CIA=0x{:08x} LR=0x{:016x} CTR=0x{:016x}",
        core.ppu.cia, core.ppu.lr, core.ppu.ctr,
    );
    eprintln!(
        "[R10.1.e diag] r3={:#x} r4={:#x} r5={:#x}",
        core.ppu.gpr[3], core.ppu.gpr[4], core.ppu.gpr[5],
    );

    match result {
        Ok(report) => {
            // Canonical success: main returned 0xC0DE.
            // Step failure codes (in main.c):
            //   1=create  2=wait  3=post  4=trywait  5=get_value
            //   6=val!=0  7=post2 8=get2  9=val!=2  10=destroy
            assert_eq!(
                report.exit_status.status, 0xC0DE,
                "expected 0xC0DE canonical exit, got 0x{:x}",
                report.exit_status.status,
            );
        }
        Err(Error::UnsupportedSyscall { number, cia }) => {
            panic!(
                "[R10.1.e smoke] reached unimplemented syscall #{number} \
                 at CIA 0x{cia:08x}. R10.1.e covered sys_semaphore_*; \
                 this is a new gap."
            );
        }
        Err(Error::Interpreter(ie)) => {
            panic!(
                "[R10.1.e smoke] PPU interpreter coverage gap: {ie:?}. \
                 R10 doesn't add opcodes; gap predates this wave."
            );
        }
        Err(Error::StepsExhausted) => {
            panic!(
                "[R10.1.e smoke] PPU ran 2M steps without exiting. \
                 Boot loop or sema contention block suspected."
            );
        }
        Err(e) => {
            panic!("[R10.1.e smoke] unexpected error: {e:?}");
        }
    }
}
