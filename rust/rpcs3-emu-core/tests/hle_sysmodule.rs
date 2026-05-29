//! HLE-crate integration — cellSysModule routed to rpcs3-hle-cellsysmodule.
//!
//! Boots `single_sysmodule_v1.self` (a PSL1GHT homebrew exercising the module
//! load lifecycle via `sysModuleIsLoaded` / `sysModuleLoad`) through
//! `EmuCore::run_self`. Establishes the stateful HLE pattern: a `SysmoduleManager`
//! field on `EmuCore` survives across guest calls, so the load lifecycle is
//! observable.
//!
//! Exit-code encoding (see fixture README):
//!   bit0 (0x1): module NOT loaded before the load call (real impl)
//!   bit1 (0x2): the load call returned CELL_OK
//!   bit2 (0x4): module loaded after the load call
//! => pre-wire stub (all-zero returns): 0x6 ; post-wire real lifecycle: 0x7.
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
    p.push("single_sysmodule_v1");
    p.push("single_sysmodule_v1.self");
    p
}

#[test]
fn sysmodule_load_lifecycle_via_real_homebrew() {
    let path = fixture_self();
    if !path.exists() {
        eprintln!(
            "[HLE sysmodule] skip: {} not present (build via Docker PSL1GHT)",
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
        "[HLE sysmodule] exit_status = {status} (0x{:08x})",
        status as u32,
    );

    // Post-wire: emu-core routes cellSysmoduleLoadModule (NID 0x32267a31) and
    // cellSysmoduleIsLoaded (NID 0x5a59e258) to rpcs3-hle-cellsysmodule, backed
    // by EmuCore's persistent SysmoduleManager. The homebrew observes the real
    // load lifecycle:
    //   bit0 (0x1): NOT loaded before the load call (UNLOADED)
    //   bit1 (0x2): the load call returned CELL_OK
    //   bit2 (0x4): loaded after the load call
    // => 0x7. Pre-wire (return-0 stub) it was 0x6 (is-loaded wrongly read loaded
    // before the load), proving the stateful HLE crate runs end-to-end.
    assert_eq!(
        status, 0x7,
        "expected the full module load lifecycle (0x7); got 0x{:x} \
         (0x6 = unwired stub)",
        status as u32,
    );
}
