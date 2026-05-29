//! HLE wave — cellSysutil string param routed to rpcs3-hle-cellsysutil.
//!
//! Boots `single_sysutil_string_v1.self` (a PSL1GHT homebrew calling
//! `cellSysutilGetSystemParamString(ID_NICKNAME, buf, sizeof buf)`) through
//! `EmuCore::run_self`. Extends R13.6 to the string param path, reusing the same
//! dep + `EmuSysutilConfig` provider. Pre-wire the return-0 stub never writes the
//! buffer (exit 0); once the NID is routed, the homebrew reads the default
//! nickname and the exit code carries its byte-sum.
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
    p.push("single_sysutil_string_v1");
    p.push("single_sysutil_string_v1.self");
    p
}

#[test]
fn sysutil_get_system_param_string_via_real_homebrew() {
    let path = fixture_self();
    if !path.exists() {
        eprintln!(
            "[HLE sysutil-string] skip: {} not present (build via Docker PSL1GHT)",
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
        "[HLE sysutil-string] exit_status = {status} (0x{:08x})",
        status as u32,
    );

    // Post-wire: the dispatcher routes cellSysutilGetSystemParamString to
    // rpcs3-hle-cellsysutil backed by EmuSysutilConfig (Nickname = "RPCS3") and
    // copies it into the guest buffer. The homebrew returns the byte-sum of the
    // buffer: 363 = 'R'+'P'+'C'+'S'+'3'. Pre-wire it was 0 (buffer untouched).
    assert_eq!(
        status, 363,
        "expected byte-sum of \"RPCS3\" (363); got {status} \
         (0 = unwired stub, 0xBAD = call error)"
    );
}
