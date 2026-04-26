//! Integration test against the committed synthetic fixtures.
//!
//! Each fixture exercises a distinct opcode family. If anyone changes
//! the encoder format, regenerates fixtures, or modifies an opcode's
//! semantics, this test will catch the divergence.

use std::path::{Path, PathBuf};
use std::process::Command;

fn binary_path() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_spu-runner"))
}

fn workspace_root() -> PathBuf {
    // tests/ runs from the crate root.
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.pop(); // rust/
    p.pop(); // workspace root (rpcs3-master)
    p
}

fn fixture(name: &str) -> PathBuf {
    let mut p = workspace_root();
    p.push("behavior-freeze/fixtures/spu");
    p.push(name);
    p
}

fn unique_out_dir(tag: &str) -> PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    static N: AtomicU64 = AtomicU64::new(0);
    let n = N.fetch_add(1, Ordering::Relaxed);
    let mut p = std::env::temp_dir();
    p.push(format!("spu-runner-fixtures-{}-{}-{}", std::process::id(), tag, n));
    p
}

fn run_fixture(name: &str, max_steps: u32) -> PathBuf {
    let elf = fixture(name);
    if !elf.exists() {
        // Skip gracefully if fixtures haven't been generated.
        eprintln!("fixture missing: {} — run build_synthetic_fixtures.py", elf.display());
        std::process::exit(0);
    }
    let out = unique_out_dir(name);
    let status = Command::new(binary_path())
        .arg(&elf)
        .arg("--out-dir").arg(&out)
        .arg("--max-steps").arg(max_steps.to_string())
        .status()
        .expect("spawn spu-runner");
    assert!(status.success(), "spu-runner failed on {name}: {status:?}");
    out
}

fn read_gpr(out: &Path, idx: u32) -> u128 {
    let csv = std::fs::read_to_string(out.join("gpr.csv")).unwrap();
    let prefix = format!("r{idx},");
    let line = csv.lines().find(|l| l.starts_with(&prefix))
        .unwrap_or_else(|| panic!("no r{idx} line in gpr.csv"));
    let hex = &line[prefix.len()..];
    u128::from_str_radix(hex, 16).unwrap()
}

fn read_summary(out: &Path) -> String {
    std::fs::read_to_string(out.join("summary.txt")).unwrap()
}

#[test]
fn synthetic_il_stop_terminates_with_code_zero() {
    let out = run_fixture("synthetic_il_stop.elf", 100);
    let summary = read_summary(&out);
    assert!(summary.contains("STOP code=0x0000"), "{summary}");
    assert!(summary.contains("steps=2"));
    let r3 = read_gpr(&out, 3);
    assert_eq!(r3, 0x00001234_00001234_00001234_00001234);
    let _ = std::fs::remove_dir_all(&out);
}

#[test]
fn synthetic_arith_executes_all_alu_ops() {
    let out = run_fixture("synthetic_arith.elf", 100);
    let summary = read_summary(&out);
    assert!(summary.contains("steps=9"), "{summary}");
    // Code 0x4242 truncated to 14 bits = 0x0242.
    assert!(summary.contains("STOP code=0x0242"), "{summary}");

    // Each register contains the broadcast result of one ALU op.
    assert_eq!(read_gpr(&out, 3),  0x00000005_00000005_00000005_00000005);
    assert_eq!(read_gpr(&out, 4),  0x00000003_00000003_00000003_00000003);
    assert_eq!(read_gpr(&out, 5),  0x00000008_00000008_00000008_00000008); // 5+3
    assert_eq!(read_gpr(&out, 6),  0x00000002_00000002_00000002_00000002); // 5-3
    assert_eq!(read_gpr(&out, 7),  0x00000014_00000014_00000014_00000014); // 5<<2
    assert_eq!(read_gpr(&out, 8),  0x00000006_00000006_00000006_00000006); // 5^3
    assert_eq!(read_gpr(&out, 9),  0x00000007_00000007_00000007_00000007); // 5|3
    assert_eq!(read_gpr(&out, 10), 0x00000001_00000001_00000001_00000001); // 5&3

    let _ = std::fs::remove_dir_all(&out);
}

#[test]
fn synthetic_loop_sums_one_through_ten() {
    let out = run_fixture("synthetic_loop.elf", 1000);
    // Loop body: 5 instructions × 10 iterations + setup + exit ~= 50+
    let summary = read_summary(&out);
    assert!(summary.contains("STOP code=0x0055"), "{summary}");
    // r3 = 1+2+...+10 = 55 = 0x37, broadcast across 4 lanes.
    assert_eq!(read_gpr(&out, 3), 0x00000037_00000037_00000037_00000037);
    let _ = std::fs::remove_dir_all(&out);
}

#[test]
fn synthetic_float_dot_chains_fa_and_fm() {
    let out = run_fixture("synthetic_float_dot.elf", 100);
    let summary = read_summary(&out);
    assert!(summary.contains("STOP code=0x0066"), "{summary}");
    // r7 = 8.0 (= 0x41000000 IEEE) broadcast — chain: 2.0+2.0=4.0 then 4.0+4.0=8.0.
    assert_eq!(read_gpr(&out, 7), 0x41000000_41000000_41000000_41000000);
    let _ = std::fs::remove_dir_all(&out);
}

#[test]
fn synthetic_loadstore_round_trips_via_ls() {
    let out = run_fixture("synthetic_loadstore.elf", 100);
    let summary = read_summary(&out);
    assert!(summary.contains("STOP code=0x00ab"), "{summary}");
    // r5 should hold the same pattern that r3 held when stored.
    assert_eq!(read_gpr(&out, 5), 0x00005A5A_00005A5A_00005A5A_00005A5A);
    let _ = std::fs::remove_dir_all(&out);
}

#[test]
fn synthetic_shifts_exercise_word_shift_family() {
    let out = run_fixture("synthetic_shifts.elf", 100);
    let summary = read_summary(&out);
    assert!(summary.contains("STOP code=0x0077"), "{summary}");
    assert_eq!(read_gpr(&out, 4), 0x00000020_00000020_00000020_00000020); // 1<<5
    assert_eq!(read_gpr(&out, 6), 0x00FFFFFF_00FFFFFF_00FFFFFF_00FFFFFF); // rotmi
    assert_eq!(read_gpr(&out, 7), 0xFFFFFFFF_FFFFFFFF_FFFFFFFF_FFFFFFFF); // rotmai sign-fill
    assert_eq!(read_gpr(&out, 8), 0x00000010_00000010_00000010_00000010); // roti
    let _ = std::fs::remove_dir_all(&out);
}

#[test]
fn synthetic_brsl_ret_executes_subroutine() {
    let out = run_fixture("synthetic_brsl_ret.elf", 100);
    let summary = read_summary(&out);
    assert!(summary.contains("STOP code=0x0099"), "{summary}");
    // r3 = 10 (initial) + 7 (subroutine) = 17 = 0x11.
    assert_eq!(read_gpr(&out, 3), 0x00000011_00000011_00000011_00000011);
    let _ = std::fs::remove_dir_all(&out);
}

#[test]
fn synthetic_orx_collapses_word_lanes() {
    let out = run_fixture("synthetic_orx_collapse.elf", 100);
    let summary = read_summary(&out);
    assert!(summary.contains("STOP code=0x00cc"), "{summary}");
    // ah(0x1234, 0x5678) per halfword = 0x68AC broadcast.
    // orx then collapses to preferred slot (lane 0).
    assert_eq!(read_gpr(&out, 6), 0x000068AC_00000000_00000000_00000000);
    let _ = std::fs::remove_dir_all(&out);
}

// ---------------------------------------------------------------------
// Differential: --backend recompiler must produce byte-identical
// state to --backend interpreter on every committed fixture.
// ---------------------------------------------------------------------

fn run_with_backend(name: &str, max_steps: u32, backend: &str) -> PathBuf {
    let elf = fixture(name);
    if !elf.exists() {
        std::process::exit(0);
    }
    let out = unique_out_dir(&format!("{name}-{backend}"));
    let status = Command::new(binary_path())
        .arg(&elf)
        .arg("--out-dir").arg(&out)
        .arg("--max-steps").arg(max_steps.to_string())
        .arg("--backend").arg(backend)
        .status()
        .expect("spawn spu-runner");
    assert!(status.success(), "spu-runner ({backend}) failed on {name}: {status:?}");
    out
}

fn diff_dirs_identical(a: &Path, b: &Path) -> bool {
    let gpr_a = std::fs::read_to_string(a.join("gpr.csv")).unwrap();
    let gpr_b = std::fs::read_to_string(b.join("gpr.csv")).unwrap();
    if gpr_a != gpr_b { return false; }
    let pc_a = std::fs::read_to_string(a.join("pc.txt")).unwrap();
    let pc_b = std::fs::read_to_string(b.join("pc.txt")).unwrap();
    if pc_a != pc_b { return false; }
    std::fs::read(a.join("ls.bin")).unwrap() == std::fs::read(b.join("ls.bin")).unwrap()
}

#[test]
fn recompiler_byte_identical_to_interpreter_on_il_stop() {
    let a = run_with_backend("synthetic_il_stop.elf", 100, "interpreter");
    let b = run_with_backend("synthetic_il_stop.elf", 100, "recompiler");
    assert!(diff_dirs_identical(&a, &b), "diff between backends");
    let _ = std::fs::remove_dir_all(&a);
    let _ = std::fs::remove_dir_all(&b);
}

#[test]
fn recompiler_byte_identical_to_interpreter_on_arith() {
    let a = run_with_backend("synthetic_arith.elf", 100, "interpreter");
    let b = run_with_backend("synthetic_arith.elf", 100, "recompiler");
    assert!(diff_dirs_identical(&a, &b));
    let _ = std::fs::remove_dir_all(&a);
    let _ = std::fs::remove_dir_all(&b);
}

#[test]
fn recompiler_byte_identical_to_interpreter_on_loop() {
    let a = run_with_backend("synthetic_loop.elf", 1000, "interpreter");
    let b = run_with_backend("synthetic_loop.elf", 1000, "recompiler");
    assert!(diff_dirs_identical(&a, &b));
    let _ = std::fs::remove_dir_all(&a);
    let _ = std::fs::remove_dir_all(&b);
}

#[test]
fn recompiler_byte_identical_to_interpreter_on_brsl_ret() {
    let a = run_with_backend("synthetic_brsl_ret.elf", 100, "interpreter");
    let b = run_with_backend("synthetic_brsl_ret.elf", 100, "recompiler");
    assert!(diff_dirs_identical(&a, &b));
    let _ = std::fs::remove_dir_all(&a);
    let _ = std::fs::remove_dir_all(&b);
}

#[test]
fn synthetic_halfword_shifts_round_trip() {
    let out = run_fixture("synthetic_halfword_shifts.elf", 100);
    let summary = read_summary(&out);
    assert!(summary.contains("STOP code=0x00dd"), "{summary}");
    // r4 = 0x00FF << 4 per halfword = 0x0FF0 broadcast.
    assert_eq!(read_gpr(&out, 4),
               0x0FF0_0FF0_0FF0_0FF0_0FF0_0FF0_0FF0_0FF0);
    // r5 = 0x00FF >> 4 per halfword = 0x000F broadcast.
    assert_eq!(read_gpr(&out, 5),
               0x000F_000F_000F_000F_000F_000F_000F_000F);
    // r6 = rotate-left 0x00FF by 8 per halfword = 0xFF00 broadcast.
    assert_eq!(read_gpr(&out, 6),
               0xFF00_FF00_FF00_FF00_FF00_FF00_FF00_FF00);
    let _ = std::fs::remove_dir_all(&out);
}

#[test]
fn recompiler_byte_identical_to_interpreter_on_halfword_shifts() {
    let a = run_with_backend("synthetic_halfword_shifts.elf", 100, "interpreter");
    let b = run_with_backend("synthetic_halfword_shifts.elf", 100, "recompiler");
    assert!(diff_dirs_identical(&a, &b));
    let _ = std::fs::remove_dir_all(&a);
    let _ = std::fs::remove_dir_all(&b);
}
