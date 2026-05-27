//! R13.1 diagnostic — locate the rsxInit null-call/fault: dump PPU
//! state at the stop point so we know which instruction + register
//! drives the addr-0 access. Diagnostic only (always passes).

use std::path::PathBuf;

use rpcs3_emu_core::{EmuCore, Error};

fn init_self() -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.pop();
    p.pop();
    p.push("behavior-freeze");
    p.push("fixtures");
    p.push("rsx");
    p.push("sources");
    p.push("single_gcm_init_v1");
    p.push("single_gcm_init_v1.self");
    p
}

#[test]
fn diag_rsx_init_fault_site() {
    let path = init_self();
    if !path.exists() {
        eprintln!("[R13.1] skip: {} absent", path.display());
        return;
    }
    let bytes = std::fs::read(&path).expect("read");
    let mut core = EmuCore::new();
    core.step_budget = 5_000_000;
    core.permissive_unknown_syscalls = false;
    let res = core.run_self(&bytes);
    eprintln!("[R13.1] outcome: {res:?}");
    eprintln!(
        "[R13.1] CIA=0x{:08x} LR=0x{:016x} CTR=0x{:016x}",
        core.ppu.cia, core.ppu.lr, core.ppu.ctr
    );
    for r in 0..=12u8 {
        eprintln!("[R13.1]   r{r:<2} = 0x{:016x}", core.ppu.gpr[r as usize]);
    }
    // Map the CTR import-stub trampoline (the last import called) to
    // its NID + module — that's the import whose null return left r9=0.
    if let Some(plan) = core.import_plan.as_ref() {
        let ctr = core.ppu.ctr as u32;
        eprintln!("[R13.1] import stubs near CTR=0x{ctr:08x}:");
        for s in &plan.stubs {
            if (s.trampoline_vaddr as i64 - ctr as i64).abs() <= 0x40 {
                eprintln!(
                    "[R13.1]   {}::0x{:08x} trampoline=0x{:08x}",
                    s.module_name, s.nid, s.trampoline_vaddr
                );
            }
        }
    }
}
