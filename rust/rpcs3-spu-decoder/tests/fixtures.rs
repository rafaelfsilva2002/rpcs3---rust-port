//! Integration test: decode every committed SPU ELF fixture and
//! verify each block ends in a known terminator + every direct
//! successor lands inside another known block (no dangling jumps).
//!
//! Fixtures are loaded directly from the workspace `behavior-freeze/
//! fixtures/spu/` directory. Tests skip gracefully if the fixtures
//! aren't present.

use std::path::PathBuf;

use rpcs3_spu_decoder::{decode_function, BlockTerminator};

const SPU_LS_SIZE: usize = 0x40000;

fn workspace_root() -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.pop(); // rust/
    p.pop(); // workspace root
    p
}

fn load_fixture(name: &str) -> Option<(Vec<u8>, u32)> {
    let path = workspace_root().join("behavior-freeze/fixtures/spu").join(name);
    if !path.exists() {
        eprintln!("fixture not found: {} — run build_synthetic_fixtures.py", path.display());
        return None;
    }
    let bytes = std::fs::read(&path).ok()?;
    // Synthesised ELFs put the entry point at the e_entry field. Quick
    // and dirty parser: e_entry sits at offset 0x18 (24) in big-endian.
    let entry = u32::from_be_bytes([bytes[24], bytes[25], bytes[26], bytes[27]]);

    // Build a 256 KB LS, copy each PT_LOAD's bytes to its p_vaddr.
    let mut ls = vec![0u8; SPU_LS_SIZE];
    let phoff = u32::from_be_bytes([bytes[28], bytes[29], bytes[30], bytes[31]]) as usize;
    let phnum = u16::from_be_bytes([bytes[44], bytes[45]]) as usize;
    let phentsize = u16::from_be_bytes([bytes[42], bytes[43]]) as usize;
    for i in 0..phnum {
        let base = phoff + i * phentsize;
        let p_offset = u32::from_be_bytes([bytes[base+4], bytes[base+5], bytes[base+6], bytes[base+7]]) as usize;
        let p_vaddr = u32::from_be_bytes([bytes[base+8], bytes[base+9], bytes[base+10], bytes[base+11]]) as usize;
        let p_filesz = u32::from_be_bytes([bytes[base+16], bytes[base+17], bytes[base+18], bytes[base+19]]) as usize;
        if p_filesz > 0 {
            ls[p_vaddr..p_vaddr + p_filesz].copy_from_slice(&bytes[p_offset..p_offset + p_filesz]);
        }
    }

    Some((ls, entry))
}

fn decode_and_validate(name: &str) {
    let Some((ls, entry)) = load_fixture(name) else { return };
    let func = decode_function(&ls, entry, 64).expect("decode_function");

    assert!(func.block_count() >= 1, "{name}: no blocks decoded");
    assert!(func.blocks.contains_key(&entry), "{name}: entry {entry:#x} not a block leader");

    // Every direct successor must be either a known block leader OR
    // pointed at by a terminator that allows unknown targets (UncondIndirect).
    for (_, block) in &func.blocks {
        // Each block must end in *some* terminator.
        match &block.terminator {
            BlockTerminator::Stop { .. }
            | BlockTerminator::UncondIndirect
            | BlockTerminator::CondIndirect { .. } => {}
            BlockTerminator::UncondDirect { target } => {
                assert!(func.blocks.contains_key(target),
                    "{name}: block 0x{:x} jumps to 0x{target:x} which is not a leader",
                    block.start_pc);
            }
            BlockTerminator::CondDirect { taken, fall_through } => {
                assert!(func.blocks.contains_key(taken),
                    "{name}: cond branch from 0x{:x} taken→0x{taken:x} not a leader",
                    block.start_pc);
                assert!(func.blocks.contains_key(fall_through),
                    "{name}: cond branch from 0x{:x} fall_through→0x{fall_through:x} not a leader",
                    block.start_pc);
            }
            BlockTerminator::UnknownOpcode { .. }
            | BlockTerminator::FellThroughLimit { .. } => {
                panic!("{name}: block 0x{:x} ended with non-terminator: {:?}",
                    block.start_pc, block.terminator);
            }
        }
    }
}

#[test]
fn decode_il_stop() { decode_and_validate("synthetic_il_stop.elf"); }

#[test]
fn decode_arith() { decode_and_validate("synthetic_arith.elf"); }

#[test]
fn decode_loop() {
    let Some((ls, entry)) = load_fixture("synthetic_loop.elf") else { return };
    let func = decode_function(&ls, entry, 64).expect("decode_function");

    // Loop fixture has 3 reachable basic blocks: setup (0x100), body
    // re-entered via back-edge (0x108), and exit/stop (0x11C).
    let starts: Vec<u32> = func.blocks.keys().copied().collect();
    assert!(starts.contains(&0x100), "blocks: {starts:?}");
    assert!(starts.contains(&0x108), "blocks: {starts:?}");
    assert!(starts.contains(&0x11C), "blocks: {starts:?}");

    // Setup block should fall through into the loop body via UncondDirect.
    let setup = &func.blocks[&0x100];
    assert!(matches!(
        setup.terminator,
        BlockTerminator::UncondDirect { target: 0x108 }
    ), "setup terminator: {:?}", setup.terminator);

    // Loop body ends in br -4 → back to itself, but only after a brnz
    // splits the path. Whether the cut yields one or two blocks at
    // 0x108 depends on leader detection; either way 0x108 must exist.
    decode_and_validate("synthetic_loop.elf");
}

#[test]
fn decode_float_dot() { decode_and_validate("synthetic_float_dot.elf"); }

#[test]
fn decode_loadstore() { decode_and_validate("synthetic_loadstore.elf"); }

#[test]
fn decode_shifts() { decode_and_validate("synthetic_shifts.elf"); }

#[test]
fn decode_brsl_ret() {
    // brsl_ret has an indirect return via `bi r5`, so the subroutine
    // block ends in UncondIndirect (target unknown at decode time).
    let Some((ls, entry)) = load_fixture("synthetic_brsl_ret.elf") else { return };
    let func = decode_function(&ls, entry, 64).expect("decode_function");

    // Should contain at least: entry block (with brsl), the
    // subroutine block (a + bi).
    assert!(func.block_count() >= 2);

    // Subroutine sits at 0x110.
    let sub = func.blocks.get(&0x110).expect("subroutine block at 0x110 missing");
    match sub.terminator {
        BlockTerminator::UncondIndirect => {} // bi
        ref other => panic!("expected UncondIndirect (bi), got {other:?}"),
    }
}

#[test]
fn decode_orx_collapse() { decode_and_validate("synthetic_orx_collapse.elf"); }
