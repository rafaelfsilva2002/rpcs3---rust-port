//! VFS slice 4 — write round-trip + O_CREAT. Boots `single_fs_write_v1.self`
//! (a PSL1GHT homebrew that O_CREAT|O_WRONLY opens, writes 8 bytes, closes,
//! reopens RDONLY, reads back, compares). The host pre-creates the parent dir
//! "/dev_hdd0" so the O_CREAT succeeds; the file is created by the homebrew.
//! Exit 0xC0DE iff the read-back matches what was written.
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
    p.push("single_fs_write_v1");
    p.push("single_fs_write_v1.self");
    p
}

#[test]
fn fs_write_roundtrip_via_real_homebrew() {
    let path = fixture_self();
    if !path.exists() {
        eprintln!(
            "[HLE fs-write] skip: {} not present (build via Docker PSL1GHT)",
            path.display()
        );
        return;
    }

    let bytes = std::fs::read(&path).expect("read .self");
    let mut core = EmuCore::new();
    core.step_budget = 5_000_000;
    core.permissive_unknown_syscalls = false;
    // Pre-create the parent dir so the O_CREAT of /dev_hdd0/w.bin succeeds.
    core.vfs_add_dir("/dev_hdd0");

    let report = core.run_self(&bytes).expect("run_self");
    let status = report.exit_status.status;
    eprintln!(
        "[HLE fs-write] exit_status = {status} (0x{:08x})",
        status as u32,
    );

    assert_eq!(
        status as u32,
        0xC0DE,
        "expected 0xC0DE (O_CREAT write + read-back match); got 0x{:08x}",
        status as u32,
    );
}

/// Negative control — no parent dir → O_CREAT open fails (ENOENT) → 0xBAD0.
#[test]
fn fs_write_without_parent_dir_is_enoent() {
    let path = fixture_self();
    if !path.exists() {
        eprintln!("[HLE fs-write-neg] skip: {} not present", path.display());
        return;
    }
    let bytes = std::fs::read(&path).expect("read .self");
    let mut core = EmuCore::new();
    core.step_budget = 5_000_000;
    core.permissive_unknown_syscalls = false;
    // No /dev_hdd0 → ensure_parent fails.

    let report = core.run_self(&bytes).expect("run_self");
    let status = report.exit_status.status;
    eprintln!(
        "[HLE fs-write-neg] exit_status = {status} (0x{:08x})",
        status as u32,
    );
    assert_eq!(
        status as u32,
        0xBAD0,
        "expected 0xBAD0 (no parent dir -> O_CREAT ENOENT); got 0x{:08x}",
        status as u32,
    );
}
