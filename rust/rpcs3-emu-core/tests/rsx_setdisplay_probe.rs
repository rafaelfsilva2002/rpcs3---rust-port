//! R13.4 probe — identify the first flip-path NID gap surfaced by
//! `gcmSetDisplayBuffer`. Diagnostic only (always passes); the test
//! body just dumps the outcome + PPU state + import-stub neighbours
//! around the failure site so we know exactly which import handler
//! (or syscall) needs to land next.
//!
//! Pattern mirrors `rsx_init_probe.rs` (R13.1).

use std::path::PathBuf;

use rpcs3_emu_core::EmuCore;

fn fixture_self() -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.pop();
    p.pop();
    p.push("behavior-freeze");
    p.push("fixtures");
    p.push("rsx");
    p.push("sources");
    p.push("single_gcm_setdisplay_v1");
    p.push("single_gcm_setdisplay_v1.self");
    p
}

#[test]
fn diag_setdisplay_first_gap() {
    let path = fixture_self();
    if !path.exists() {
        eprintln!("[R13.4] skip: {} absent", path.display());
        return;
    }
    let bytes = std::fs::read(&path).expect("read");
    let mut core = EmuCore::new();
    core.step_budget = 5_000_000;
    core.permissive_unknown_syscalls = false;
    let res = core.run_self(&bytes);
    eprintln!("[R13.4] outcome: {res:?}");
    eprintln!(
        "[R13.4] CIA=0x{:08x} LR=0x{:016x} CTR=0x{:016x}",
        core.ppu.cia, core.ppu.lr, core.ppu.ctr
    );
    for r in 0..=12u8 {
        eprintln!("[R13.4]   r{r:<2} = 0x{:016x}", core.ppu.gpr[r as usize]);
    }
    if let Some(plan) = core.import_plan.as_ref() {
        let ctr = core.ppu.ctr as u32;
        let cia = core.ppu.cia;
        eprintln!("[R13.4] import stubs near CTR=0x{ctr:08x} / CIA=0x{cia:08x}:");
        for s in &plan.stubs {
            let dctr = (s.trampoline_vaddr as i64 - ctr as i64).abs();
            let dcia = (s.trampoline_vaddr as i64 - cia as i64).abs();
            if dctr <= 0x40 || dcia <= 0x40 {
                eprintln!(
                    "[R13.4]   {}::0x{:08x} trampoline=0x{:08x}",
                    s.module_name, s.nid, s.trampoline_vaddr
                );
            }
        }
    }
}
