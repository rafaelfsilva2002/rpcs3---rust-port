//! First HLE-crate integration — cellSysutil routed to rpcs3-hle-cellsysutil.
//!
//! Boots `single_sysutil_param_v1.self` (a PSL1GHT homebrew calling
//! `cellSysutilGetSystemParamInt(ID_LANG, &lang)`) through `EmuCore::run_self`.
//! Before the NID is wired, emu-core answers it with the permissive return-0
//! stub (r3=0, no write), so the homebrew sees its sentinel. Once the NID is
//! routed to `rpcs3_hle_cellsysutil::cell_sysutil_get_system_param_int` backed
//! by emu-core's fixed system-config provider, the homebrew reads the real LANG
//! value — proving an HLE crate runs end-to-end through the boot path.
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
    p.push("single_sysutil_param_v1");
    p.push("single_sysutil_param_v1.self");
    p
}

#[test]
fn sysutil_get_system_param_int_via_real_homebrew() {
    let path = fixture_self();
    if !path.exists() {
        eprintln!(
            "[HLE sysutil] skip: {} not present (build via Docker PSL1GHT)",
            path.display()
        );
        return;
    }

    let bytes = std::fs::read(&path).expect("read .self");
    let mut core = EmuCore::new();
    core.step_budget = 5_000_000;
    core.permissive_unknown_syscalls = false;
    let report = core.run_self(&bytes).expect("run_self");

    let status = report.exit_status.status;
    eprintln!(
        "[HLE sysutil] exit_status = {status} (0x{:08x})",
        status as u32,
    );

    // Post-wire: emu-core routes NID 0x40e895d3 to rpcs3-hle-cellsysutil backed
    // by EmuSysutilConfig (Lang = CELL_SYSUTIL_LANG_ENGLISH_US = 1). The homebrew
    // reads that into `lang` and returns it — proving the HLE crate runs
    // end-to-end through the boot path (pre-wire it returned the -12345 sentinel).
    assert_eq!(
        status, 1,
        "homebrew should read LANG=1 from the wired cellSysutil HLE crate \
         (got {status}; -12345 = sentinel/unwired, 0xBAD = call error)"
    );
}
