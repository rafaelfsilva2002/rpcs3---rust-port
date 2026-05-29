//! VFS slice 3 — directory enumeration. Boots `single_fs_readdir_v1.self` (a
//! PSL1GHT homebrew that sysLv2FsOpenDir → ReadDir loop → CloseDir) against three
//! pre-seeded files under "/dev_hdd0/d/". Exit 0xC0DE iff exactly 3 regular
//! entries are enumerated.
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
    p.push("single_fs_readdir_v1");
    p.push("single_fs_readdir_v1.self");
    p
}

fn seed_three(core: &mut EmuCore) {
    core.vfs_add_file("/dev_hdd0/d/a.bin", vec![0xAA]);
    core.vfs_add_file("/dev_hdd0/d/b.bin", vec![0xBB]);
    core.vfs_add_file("/dev_hdd0/d/c.bin", vec![0xCC]);
}

#[test]
fn fs_readdir_enumerates_seeded_dir_via_real_homebrew() {
    let path = fixture_self();
    if !path.exists() {
        eprintln!(
            "[HLE fs-readdir] skip: {} not present (build via Docker PSL1GHT)",
            path.display()
        );
        return;
    }

    let bytes = std::fs::read(&path).expect("read .self");
    let mut core = EmuCore::new();
    core.step_budget = 5_000_000;
    core.permissive_unknown_syscalls = false;
    seed_three(&mut core);

    let report = core.run_self(&bytes).expect("run_self");
    let status = report.exit_status.status;
    eprintln!(
        "[HLE fs-readdir] exit_status = {status} (0x{:08x})",
        status as u32,
    );

    assert_eq!(
        status as u32,
        0xC0DE,
        "expected 0xC0DE (3 regular entries enumerated); got 0x{:08x}",
        status as u32,
    );
}

/// Negative control — no seed → /dev_hdd0/d doesn't exist → opendir ENOENT → 0xBAD0.
#[test]
fn fs_readdir_without_seed_is_enoent() {
    let path = fixture_self();
    if !path.exists() {
        eprintln!("[HLE fs-readdir-neg] skip: {} not present", path.display());
        return;
    }
    let bytes = std::fs::read(&path).expect("read .self");
    let mut core = EmuCore::new();
    core.step_budget = 5_000_000;
    core.permissive_unknown_syscalls = false;

    let report = core.run_self(&bytes).expect("run_self");
    let status = report.exit_status.status;
    eprintln!(
        "[HLE fs-readdir-neg] exit_status = {status} (0x{:08x})",
        status as u32,
    );
    assert_eq!(
        status as u32,
        0xBAD0,
        "expected 0xBAD0 (no dir -> opendir ENOENT); got 0x{:08x}",
        status as u32,
    );
}
