//! R16 — cellSaveData AutoSave2/AutoLoad2 callback bridge. Boots
//! `single_savedata_autosave_v1.self` (a PSL1GHT homebrew driving the
//! callback-based savedata protocol via `sysSaveAutoSave2`/`sysSaveAutoLoad2`).
//! Save 8 bytes to SLOTAUTO00/DATA.BIN, wipe the local buffer, load it back,
//! compare → 0xC0DE on round-trip match.
//!
//! This is the first HLE family where the *system* calls back INTO guest code:
//! the emu-core bridge invokes the game's status + file callbacks via
//! `EmuCore::call_guest_function` (R14 guest re-entry), marshalling the
//! `sysSave*` structs through a scratch page and performing the file I/O against
//! the in-memory VFS (R15).
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
    p.push("single_savedata_autosave_v1");
    p.push("single_savedata_autosave_v1.self");
    p
}

#[test]
fn savedata_autosave_roundtrip_via_real_homebrew() {
    let path = fixture_self();
    if !path.exists() {
        eprintln!(
            "[HLE savedata] skip: {} not present (build via Docker PSL1GHT)",
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
        "[HLE savedata] exit_status = {status} (0x{:08x})",
        status as u32,
    );

    assert_eq!(
        status as u32,
        0xC0DE,
        "expected 0xC0DE (AutoSave2 → AutoLoad2 round-trip match via the \
         callback bridge); got 0x{:08x}",
        status as u32,
    );

    // The bridge persisted the payload into the VFS at the savedata path —
    // confirm the WRITE landed (independent of the in-guest comparison).
    let saved = core
        .vfs
        .read_file("/dev_hdd0/home/00000001/savedata/SLOTAUTO00/DATA.BIN");
    assert_eq!(
        saved.as_deref(),
        Some(&[0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88][..]),
        "savedata WRITE should have persisted the 8-byte payload to the VFS",
    );
}
