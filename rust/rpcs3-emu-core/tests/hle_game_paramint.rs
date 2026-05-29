//! HLE wave — cellGameGetParamInt routed to rpcs3-hle-cellgame.
//!
//! Boots `single_game_paramint_v1.self` (a PSL1GHT homebrew calling
//! `cellGameGetParamInt(PARENTAL_LEVEL=103, &v)`). Pre-wire the return-0 stub
//! never writes `v`; once routed to rpcs3-hle-cellgame backed by a fixed
//! GameState provider, v = the configured parental level.
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
    p.push("single_game_paramint_v1");
    p.push("single_game_paramint_v1.self");
    p
}

#[test]
fn game_get_param_int_via_real_homebrew() {
    let path = fixture_self();
    if !path.exists() {
        eprintln!(
            "[HLE game-paramint] skip: {} not present (build via Docker PSL1GHT)",
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
        "[HLE game-paramint] exit_status = {status} (0x{:08x})",
        status as u32,
    );

    // Post-wire: cellGameGetParamInt(PARENTAL_LEVEL=103) routes to
    // rpcs3-hle-cellgame backed by EmuGameConfig (parental level = 1). The
    // homebrew reads that value. Pre-wire it was 0x55 (sentinel, OUT untouched).
    assert_eq!(
        status, 1,
        "expected the configured parental level (1); got {status} \
         (0x55 = unwired/OUT untouched, 0xBAD = call error)"
    );
}
