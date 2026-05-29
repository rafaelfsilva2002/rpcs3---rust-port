//! VFS slice 2 — stat / fstat / lseek. Boots `single_fs_stat_v1.self` (a PSL1GHT
//! homebrew that sysLv2FsStat → Open → FStat → LSeek64 SET 8 → Read 8) through
//! `EmuCore::run_self` against a pre-seeded "/dev_hdd0/test.bin" (0x01..0x10).
//! Exit 0xC0DE iff stat+fstat report size 16, lseek lands at 8, and the read
//! from offset 8 sums to 0x64.
//!
//! Skips gracefully when the `.self` is absent (gitignored; built via Docker).

use std::path::PathBuf;

use rpcs3_emu_core::EmuCore;

const CONTENT: [u8; 16] = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16];

fn fixture_self() -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.pop(); // rust/
    p.pop(); // rpcs3-master/
    p.push("behavior-freeze");
    p.push("fixtures");
    p.push("hle");
    p.push("sources");
    p.push("single_fs_stat_v1");
    p.push("single_fs_stat_v1.self");
    p
}

#[test]
fn fs_stat_fstat_lseek_via_real_homebrew() {
    let path = fixture_self();
    if !path.exists() {
        eprintln!(
            "[HLE fs-stat] skip: {} not present (build via Docker PSL1GHT)",
            path.display()
        );
        return;
    }

    let bytes = std::fs::read(&path).expect("read .self");
    let mut core = EmuCore::new();
    core.step_budget = 5_000_000;
    core.permissive_unknown_syscalls = false;
    core.vfs_add_file("/dev_hdd0/test.bin", CONTENT.to_vec());

    let report = core.run_self(&bytes).expect("run_self");
    let status = report.exit_status.status;
    eprintln!(
        "[HLE fs-stat] exit_status = {status} (0x{:08x})",
        status as u32,
    );

    assert_eq!(
        status as u32,
        0xC0DE,
        "expected 0xC0DE (stat/fstat size 16, lseek 8, read sum 0x64); got 0x{:08x}",
        status as u32,
    );
}

/// Negative control — no pre-seed → sys_fs_stat returns ENOENT → 0xBAD0.
#[test]
fn fs_stat_without_seed_is_enoent() {
    let path = fixture_self();
    if !path.exists() {
        eprintln!("[HLE fs-stat-neg] skip: {} not present", path.display());
        return;
    }
    let bytes = std::fs::read(&path).expect("read .self");
    let mut core = EmuCore::new();
    core.step_budget = 5_000_000;
    core.permissive_unknown_syscalls = false;

    let report = core.run_self(&bytes).expect("run_self");
    let status = report.exit_status.status;
    eprintln!(
        "[HLE fs-stat-neg] exit_status = {status} (0x{:08x})",
        status as u32,
    );
    assert_eq!(
        status as u32,
        0xBAD0,
        "expected 0xBAD0 (no seeded file -> stat ENOENT); got 0x{:08x}",
        status as u32,
    );
}
