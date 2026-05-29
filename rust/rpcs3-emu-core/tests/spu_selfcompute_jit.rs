//! End-to-end validation of the SPU JIT backend through `EmuCore::run_self`.
//!
//! Boots the self-contained `single_spu_selfcompute_v1` homebrew (no IN_MBOX,
//! no DMA — see the fixture README for why that shape is required by the
//! synchronous single-SPU `run_self` path) under both SPU backends and asserts
//! the SPU computed the expected `OUT_MBOX` value. The `spu-recompiler` feature
//! test additionally asserts the Cranelift JIT result is byte-identical to the
//! interpreter — the previously-missing end-to-end coverage for lever #1.
//!
//! We assert on `EmuCore::spu_exit_status` (the OUT_MBOX value the SPU group
//! emitted, captured by `sys_spu_thread_group_start`) rather than the PPU
//! process exit code: PSL1GHT's printf/`sysSpuImageClose` teardown reaches an
//! unimplemented import (`sysPrxForUser 0xe0da8efd`) that perturbs the PPU exit
//! code. The SPU's OUT_MBOX is exactly the value each backend produced, which
//! is what this test exists to compare.
//!
//! Skips gracefully when the fixture `.self` is absent (gitignored; built via
//! the Docker toolchain — see the fixture README).

use std::path::PathBuf;

use rpcs3_emu_core::{EmuCore, SpuBackend};

/// sum(1..=1000) = 500500.
const EXPECTED_OUT_MBOX: u32 = 0x0007_A314;

fn self_path() -> Option<PathBuf> {
    let mut base = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    base.pop(); // -> rust/
    base.pop(); // -> rpcs3-master/
    base.push("behavior-freeze");
    base.push("fixtures");
    base.push("spu");
    base.push("sources");
    base.push("single_spu_selfcompute_v1");
    // SPU fixtures usually land the .self under build/; tolerate dir-root too.
    for cand in [
        base.join("build").join("single_spu_selfcompute_v1.self"),
        base.join("single_spu_selfcompute_v1.self"),
    ] {
        if cand.exists() {
            return Some(cand);
        }
    }
    None
}

/// Boot the homebrew and return the SPU group's captured OUT_MBOX value.
fn spu_out_mbox(self_bytes: &[u8], backend: SpuBackend) -> Option<u32> {
    let mut core = EmuCore::new();
    core.step_budget = 5_000_000;
    core.spu_backend = backend;
    // run_self may report a quirky PPU exit code (see module docs); we only
    // care that the SPU executed and stashed its OUT_MBOX.
    let _ = core.run_self(self_bytes);
    core.spu_exit_status
}

#[test]
fn selfcompute_interpreter_runs_spu() {
    let Some(p) = self_path() else {
        eprintln!("[selfcompute] skip: .self not built yet (run the Docker make)");
        return;
    };
    let bytes = std::fs::read(&p).expect("read .self");
    assert_eq!(
        spu_out_mbox(&bytes, SpuBackend::Interpreter),
        Some(EXPECTED_OUT_MBOX),
        "interpreter: SPU OUT_MBOX should be sum(1..=1000)=0x7A314",
    );
}

#[cfg(feature = "spu-recompiler")]
#[test]
fn selfcompute_jit_byte_identical_to_interpreter() {
    let Some(p) = self_path() else {
        eprintln!("[selfcompute] skip: .self not built yet (run the Docker make)");
        return;
    };
    let bytes = std::fs::read(&p).expect("read .self");
    let interp = spu_out_mbox(&bytes, SpuBackend::Interpreter);
    let jit = spu_out_mbox(&bytes, SpuBackend::Recompiler);
    assert_eq!(
        interp,
        Some(EXPECTED_OUT_MBOX),
        "interpreter baseline OUT_MBOX must be 0x7A314",
    );
    assert_eq!(
        jit, interp,
        "JIT OUT_MBOX ({jit:x?}) diverged from interpreter ({interp:x?})",
    );
}
