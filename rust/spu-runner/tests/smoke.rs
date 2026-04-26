//! End-to-end smoke test for `spu-runner`.
//!
//! Builds a synthetic SPU ELF in-memory (3 instructions: `il r3, 0x1234`
//! → `stop 0`), invokes the binary, and verifies the dumped state.
//!
//! No external fixture required — the ELF is generated here.

use std::path::PathBuf;
use std::process::Command;

// ---------------------------------------------------------------------
// Minimal ELF32 BE builder for SPU programs
// ---------------------------------------------------------------------

const EI_NIDENT: usize = 16;
const ELF_HEADER_SIZE: u32 = 52;
const PHDR_SIZE: u32 = 32;

const ET_EXEC: u16 = 2;
const EM_SPU: u16 = 0x17;
const PT_LOAD: u32 = 1;

fn write_be_u32(buf: &mut Vec<u8>, v: u32) {
    buf.extend_from_slice(&v.to_be_bytes());
}
fn write_be_u16(buf: &mut Vec<u8>, v: u16) {
    buf.extend_from_slice(&v.to_be_bytes());
}

fn build_minimal_spu_elf(load_lsa: u32, code: &[u32]) -> Vec<u8> {
    let mut elf = Vec::with_capacity(256);

    // -- ELF identification --
    elf.extend_from_slice(b"\x7fELF");
    elf.push(1); // EI_CLASS = ELFCLASS32
    elf.push(2); // EI_DATA = ELFDATA2MSB (big-endian)
    elf.push(1); // EI_VERSION
    elf.push(0); // EI_OSABI = ELFOSABI_NONE
    while elf.len() < EI_NIDENT { elf.push(0); }

    // -- ELF header --
    write_be_u16(&mut elf, ET_EXEC);
    write_be_u16(&mut elf, EM_SPU);
    write_be_u32(&mut elf, 1); // e_version
    write_be_u32(&mut elf, load_lsa); // e_entry
    write_be_u32(&mut elf, ELF_HEADER_SIZE); // e_phoff
    write_be_u32(&mut elf, 0); // e_shoff
    write_be_u32(&mut elf, 0); // e_flags
    write_be_u16(&mut elf, ELF_HEADER_SIZE as u16); // e_ehsize
    write_be_u16(&mut elf, PHDR_SIZE as u16); // e_phentsize
    write_be_u16(&mut elf, 1); // e_phnum
    write_be_u16(&mut elf, 0); // e_shentsize
    write_be_u16(&mut elf, 0); // e_shnum
    write_be_u16(&mut elf, 0); // e_shstrndx

    // -- Program header (PT_LOAD pointing at code that follows) --
    let code_size = (code.len() * 4) as u32;
    let code_offset = ELF_HEADER_SIZE + PHDR_SIZE;
    write_be_u32(&mut elf, PT_LOAD);
    write_be_u32(&mut elf, code_offset); // p_offset
    write_be_u32(&mut elf, load_lsa);    // p_vaddr
    write_be_u32(&mut elf, load_lsa);    // p_paddr
    write_be_u32(&mut elf, code_size);   // p_filesz
    write_be_u32(&mut elf, code_size);   // p_memsz
    write_be_u32(&mut elf, 5);           // p_flags = PF_X | PF_R
    write_be_u32(&mut elf, 4);           // p_align

    // -- Code --
    for inst in code {
        write_be_u32(&mut elf, *inst);
    }

    elf
}

// ---------------------------------------------------------------------
// Encoders mirrored from rpcs3-spu-interpreter::encode (kept local to
// avoid a dev-dep on that crate; the bytes are short and stable.)
// ---------------------------------------------------------------------

const fn pack_ri16(primary_9: u32, rt: u32, imm16: u16) -> u32 {
    ((primary_9 & 0x1FF) << 23) | ((imm16 as u32 & 0xFFFF) << 7) | (rt & 0x7F)
}
const fn il(rt: u32, imm16_signed: i16) -> u32 {
    pack_ri16(0x081, rt, imm16_signed as u16)
}
const fn stop(code: u32) -> u32 {
    // primary 0x000 in bits 0..10; 14-bit code at MSB bits 18..31 →
    // LSB bits 0..13 (no shift required).
    code & 0x3FFF
}

// ---------------------------------------------------------------------
// Test harness
// ---------------------------------------------------------------------

fn binary_path() -> PathBuf {
    // CARGO_BIN_EXE_<name> is set by Cargo for integration tests.
    PathBuf::from(env!("CARGO_BIN_EXE_spu-runner"))
}

fn workspace_tmp(tag: &str) -> PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let mut p = std::env::temp_dir();
    p.push(format!("spu-runner-test-{}-{}-{}", std::process::id(), tag, n));
    p
}

#[test]
fn loads_synthetic_elf_and_executes_to_stop() {
    let bin = binary_path();
    let tmp = workspace_tmp("smoke");
    std::fs::create_dir_all(&tmp).unwrap();

    // Program: il r3, 0x1234; stop 0
    let elf = build_minimal_spu_elf(0x100, &[il(3, 0x1234), stop(0)]);
    let elf_path = tmp.join("hello.elf");
    std::fs::write(&elf_path, &elf).unwrap();

    let out_dir = tmp.join("dumps");
    let status = Command::new(&bin)
        .arg(&elf_path)
        .arg("--out-dir").arg(&out_dir)
        .arg("--max-steps").arg("100")
        .status()
        .expect("spawn spu-runner");

    assert!(status.success(), "spu-runner exit code: {status:?}");

    let summary = std::fs::read_to_string(out_dir.join("summary.txt")).unwrap();
    assert!(summary.contains("STOP code=0x0000"), "summary was:\n{summary}");
    assert!(summary.contains("steps=2"));

    // r3 should hold 0x1234 broadcast across 4 lanes (il broadcasts).
    let gpr = std::fs::read_to_string(out_dir.join("gpr.csv")).unwrap();
    let r3_line = gpr.lines().find(|l| l.starts_with("r3,")).unwrap();
    // 0x1234 broadcast = 0x00001234_00001234_00001234_00001234
    assert!(
        r3_line.ends_with(",00001234000012340000123400001234"),
        "r3 line: {r3_line}"
    );

    // ls.bin should be exactly 256 KB.
    let ls = std::fs::metadata(out_dir.join("ls.bin")).unwrap();
    assert_eq!(ls.len(), 256 * 1024);

    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn rejects_non_spu_elf() {
    let bin = binary_path();
    let tmp = workspace_tmp("smoke");
    std::fs::create_dir_all(&tmp).unwrap();

    // Build an ELF with EM_PPC64 (0x15) instead of EM_SPU.
    let mut elf = build_minimal_spu_elf(0x100, &[stop(0)]);
    // Replace e_machine bytes (offset = EI_NIDENT + 2 (e_type)).
    let off = EI_NIDENT + 2;
    elf[off..off + 2].copy_from_slice(&0x15u16.to_be_bytes());

    let elf_path = tmp.join("not-spu.elf");
    std::fs::write(&elf_path, &elf).unwrap();

    let out = Command::new(&bin)
        .arg(&elf_path)
        .arg("--out-dir").arg(tmp.join("dumps"))
        .output()
        .expect("spawn spu-runner");
    assert_eq!(out.status.code(), Some(3), "stderr:\n{}", String::from_utf8_lossy(&out.stderr));
    assert!(String::from_utf8_lossy(&out.stderr).contains("not a SPU ELF"));

    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn rejects_missing_file() {
    let bin = binary_path();
    let out = Command::new(&bin)
        .arg("/nonexistent/path/to.elf")
        .output()
        .expect("spawn spu-runner");
    assert_eq!(out.status.code(), Some(3));
}

#[test]
fn cli_usage_error_returns_64() {
    let bin = binary_path();
    let out = Command::new(&bin)
        .arg("--unknown-flag")
        .output()
        .expect("spawn spu-runner");
    assert_eq!(out.status.code(), Some(64));
}

#[test]
fn max_steps_returns_1_without_stop() {
    let bin = binary_path();
    let tmp = workspace_tmp("smoke");
    std::fs::create_dir_all(&tmp).unwrap();

    // Program: il r3, 1; nop; nop; nop  — never hits Stop within
    // 3 steps of max_steps=2.
    let elf = build_minimal_spu_elf(
        0x100,
        &[il(3, 1), 0x4020_0000 /* nop */, 0x4020_0000, 0x4020_0000],
    );
    let elf_path = tmp.join("loop.elf");
    std::fs::write(&elf_path, &elf).unwrap();

    let status = Command::new(&bin)
        .arg(&elf_path)
        .arg("--max-steps").arg("2")
        .arg("--out-dir").arg(tmp.join("dumps"))
        .status()
        .expect("spawn spu-runner");
    assert_eq!(status.code(), Some(1));

    let _ = std::fs::remove_dir_all(&tmp);
}
