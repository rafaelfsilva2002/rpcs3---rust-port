//! VFS — first lv2 filesystem oracle. Boots `single_fs_read_v1.self` (a PSL1GHT
//! homebrew that sysLv2FsOpen → Read 16 bytes → Close → byte-sum) through
//! `EmuCore::run_self`. The test pre-seeds "/dev_hdd0/test.bin" = 0x01..0x10
//! (sum 0x88) into the in-memory VFS before the run — a deterministic stand-in
//! for on-disk content. Exit 0xC0DE iff it read the seeded bytes.
//!
//! Skips gracefully when the `.self` is absent (gitignored; built via Docker).

use std::path::PathBuf;

use rpcs3_emu_core::EmuCore;

const CONTENT: [u8; 16] = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16]; // sum = 0x88

fn fixture_self() -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.pop(); // rust/
    p.pop(); // rpcs3-master/
    p.push("behavior-freeze");
    p.push("fixtures");
    p.push("hle");
    p.push("sources");
    p.push("single_fs_read_v1");
    p.push("single_fs_read_v1.self");
    p
}

#[test]
fn fs_read_seeded_file_via_real_homebrew() {
    let path = fixture_self();
    if !path.exists() {
        eprintln!(
            "[HLE fs-read] skip: {} not present (build via Docker PSL1GHT)",
            path.display()
        );
        return;
    }

    let bytes = std::fs::read(&path).expect("read .self");
    let mut core = EmuCore::new();
    core.step_budget = 5_000_000;
    core.permissive_unknown_syscalls = false;
    // Pre-seed the file BEFORE run_self (path byte-identical to the guest's).
    core.vfs_add_file("/dev_hdd0/test.bin", CONTENT.to_vec());

    let report = core.run_self(&bytes).expect("run_self");
    let status = report.exit_status.status;
    eprintln!(
        "[HLE fs-read] exit_status = {status} (0x{:08x})",
        status as u32,
    );

    // 0xC0DE = opened + read 16 bytes + sum == 0x88 (proves the VFS read path).
    assert_eq!(
        status as u32,
        0xC0DE,
        "expected 0xC0DE (read seeded /dev_hdd0/test.bin); got 0x{:08x}",
        status as u32,
    );
}

/// Negative control — same binary, NO pre-seed. sys_fs_open returns ENOENT, so
/// the homebrew returns 0xBAD0. Proves the 0xC0DE came from the seeded read.
#[test]
fn fs_read_without_seed_is_enoent() {
    let path = fixture_self();
    if !path.exists() {
        eprintln!("[HLE fs-read-neg] skip: {} not present", path.display());
        return;
    }

    let bytes = std::fs::read(&path).expect("read .self");
    let mut core = EmuCore::new();
    core.step_budget = 5_000_000;
    core.permissive_unknown_syscalls = false;
    // No pre-seeded file.

    let report = core.run_self(&bytes).expect("run_self");
    let status = report.exit_status.status;
    eprintln!(
        "[HLE fs-read-neg] exit_status = {status} (0x{:08x})",
        status as u32,
    );
    assert_eq!(
        status as u32,
        0xBAD0,
        "expected 0xBAD0 (no seeded file -> ENOENT); got 0x{:08x}",
        status as u32,
    );
}
