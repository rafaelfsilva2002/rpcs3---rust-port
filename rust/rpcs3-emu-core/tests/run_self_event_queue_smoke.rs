//! R10.1.f smoke test — `EmuCore::run_self` drives a PSL1GHT
//! `.self` that exercises the `sys_event_queue_*` +
//! `sys_event_port_*` syscall family end-to-end (create queue +
//! port → connect → send → receive → verify → teardown → return
//! `0xC0DE`).
//!
//! Targets syscalls #128-#130 + #134-#138, wired into the
//! dispatcher by this same slice on top of the R10.6
//! `EventRegistry` impl.
//!
//! Skips gracefully when the fixture `.self` is absent.

use std::path::PathBuf;

use rpcs3_emu_core::{EmuCore, Error};

fn event_queue_self_path() -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.pop(); // rust/
    p.pop(); // rpcs3-master/
    p.push("behavior-freeze");
    p.push("fixtures");
    p.push("lv2");
    p.push("sources");
    p.push("single_event_queue_v1");
    p.push("single_event_queue_v1.self");
    p
}

#[test]
fn r10_1f_event_queue_round_trip_returns_canonical_status() {
    let self_path = event_queue_self_path();
    if !self_path.exists() {
        eprintln!(
            "[R10.1.f smoke] skip: {} not present (run Docker build first)",
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
        "[R10.1.f diag] post-run: CIA=0x{:08x} LR=0x{:016x} CTR=0x{:016x}",
        core.ppu.cia, core.ppu.lr, core.ppu.ctr,
    );
    eprintln!(
        "[R10.1.f diag] r3={:#x} r4={:#x} r5={:#x}",
        core.ppu.gpr[3], core.ppu.gpr[4], core.ppu.gpr[5],
    );

    match result {
        Ok(report) => {
            // Step failure codes (main.c):
            //   1=qcreate 2=pcreate 3=connect 4=send 5=receive
            //   6=d1 7=d2 8=d3 9=disconnect 10=pdestroy 11=qdestroy
            assert_eq!(
                report.exit_status.status, 0xC0DE,
                "expected 0xC0DE canonical exit, got 0x{:x}",
                report.exit_status.status,
            );
        }
        Err(Error::UnsupportedSyscall { number, cia }) => {
            panic!(
                "[R10.1.f smoke] reached unimplemented syscall #{number} \
                 at CIA 0x{cia:08x}. R10.1.f covered sys_event_*; \
                 this is a new gap."
            );
        }
        Err(Error::Interpreter(ie)) => {
            panic!(
                "[R10.1.f smoke] PPU interpreter coverage gap: {ie:?}. \
                 R10 doesn't add opcodes; gap predates this wave."
            );
        }
        Err(Error::StepsExhausted) => {
            panic!(
                "[R10.1.f smoke] PPU ran 2M steps without exiting. \
                 Boot loop or event-queue receive block suspected."
            );
        }
        Err(e) => {
            panic!("[R10.1.f smoke] unexpected error: {e:?}");
        }
    }
}
