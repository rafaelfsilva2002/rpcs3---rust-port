//! `spu-runner` — load a SPU ELF, run it through any backend that
//! implements `rpcs3_spu_differential::SpuExecutor`, dump the final
//! state for differential comparison.
//!
//! Today the only backend wired in is `InterpreterExecutor`. When the
//! future SPU recompiler lands, it will implement the same trait and
//! become selectable via `--backend recompiler` without further plumbing.
//!
//! ## CLI
//!
//! ```text
//! spu-runner <ELF> [--max-steps N] [--out-dir DIR] [--backend NAME]
//! ```
//!
//! Currently `--backend` accepts only `interpreter` (default).
//!
//! ## Output (in `--out-dir`, default = current dir)
//!
//! - `gpr.csv`     — 128 lines, `r{i},{u128_hex}`
//! - `pc.txt`      — final PC and entry PC
//! - `ls.bin`      — full 256 KB local store dump
//! - `summary.txt` — steps + outcome + backend used
//!
//! ## Exit codes
//!
//! - `0` — execution finished cleanly with `Stop`
//! - `1` — execution hit `max_steps` without stopping
//! - `2` — backend returned a runtime error (out-of-bounds LS,
//!         unimplemented opcode, channel stall reported as an error
//!         when the backend cannot resume, etc.)
//! - `3` — ELF parse / load error
//! - `64` — CLI usage error

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use rpcs3_loader_elf_self::{parse_elf, ElfInfo, ProgramHeader};
use rpcs3_spu_differential::{
    ExecutionStopReason, InterpreterExecutor, SpuExecutionResult, SpuExecutor, SpuProgram,
};
use rpcs3_spu_recompiler::RecompilerExecutor;
use rpcs3_spu_thread::SPU_LS_SIZE;

const DEFAULT_MAX_STEPS: u64 = 1_000_000;

#[derive(Debug)]
struct Args {
    elf: PathBuf,
    out_dir: PathBuf,
    max_steps: u64,
    backend: BackendKind,
}

#[derive(Debug, Clone, Copy)]
enum BackendKind {
    Interpreter,
    Recompiler,
}

fn parse_args() -> Result<Args, String> {
    let mut a = std::env::args().skip(1);
    let mut elf: Option<PathBuf> = None;
    let mut out_dir = PathBuf::from(".");
    let mut max_steps = DEFAULT_MAX_STEPS;
    let mut backend = BackendKind::Interpreter;

    while let Some(arg) = a.next() {
        match arg.as_str() {
            "--max-steps" => {
                let v = a.next().ok_or("--max-steps needs a value")?;
                max_steps = v.parse().map_err(|_| format!("bad --max-steps: {v}"))?;
            }
            "--out-dir" => {
                out_dir = PathBuf::from(a.next().ok_or("--out-dir needs a value")?);
            }
            "--backend" => {
                let v = a.next().ok_or("--backend needs a value")?;
                backend = match v.as_str() {
                    "interpreter" => BackendKind::Interpreter,
                    "recompiler" => BackendKind::Recompiler,
                    other => return Err(format!("unknown backend: {other}")),
                };
            }
            "-h" | "--help" => return Err("help".into()),
            other if other.starts_with("--") => {
                return Err(format!("unknown flag: {other}"));
            }
            _ => {
                if elf.is_some() {
                    return Err(format!("unexpected positional: {arg}"));
                }
                elf = Some(PathBuf::from(arg));
            }
        }
    }

    let elf = elf.ok_or("missing ELF path")?;
    Ok(Args { elf, out_dir, max_steps, backend })
}

fn print_usage() {
    eprintln!("usage: spu-runner <ELF> [--max-steps N] [--out-dir DIR] [--backend interpreter|recompiler]");
    eprintln!();
    eprintln!("Loads a SPU ELF, executes it on the chosen backend, and dumps");
    eprintln!("{{gpr.csv, pc.txt, ls.bin, summary.txt}} into --out-dir for");
    eprintln!("differential comparison against RPCS3 C++ or another backend.");
    eprintln!();
    eprintln!("Backends:");
    eprintln!("  interpreter  (default) — rpcs3-spu-interpreter (reference oracle)");
    eprintln!("  recompiler            — rpcs3-spu-recompiler (scaffold; today");
    eprintln!("                          delegates to interpreter — JIT lands in R2)");
}

fn main() -> ExitCode {
    let args = match parse_args() {
        Ok(a) => a,
        Err(e) if e == "help" => { print_usage(); return ExitCode::from(64); }
        Err(e) => { eprintln!("error: {e}"); print_usage(); return ExitCode::from(64); }
    };

    let bytes = match fs::read(&args.elf) {
        Ok(b) => b,
        Err(e) => { eprintln!("error: read {}: {e}", args.elf.display()); return ExitCode::from(3); }
    };

    let info = match parse_elf(&bytes) {
        Ok(i) => i,
        Err(e) => { eprintln!("error: parse ELF: {e}"); return ExitCode::from(3); }
    };

    if !info.is_spu() {
        eprintln!(
            "error: not a SPU ELF (e_machine = 0x{:x}, expected 0x17)",
            info.e_machine
        );
        return ExitCode::from(3);
    }

    let program = match build_program_from_elf(&info, &bytes, args.max_steps) {
        Ok(p) => p,
        Err(e) => { eprintln!("error: build program: {e}"); return ExitCode::from(3); }
    };

    let backend_name = match args.backend {
        BackendKind::Interpreter => "interpreter",
        BackendKind::Recompiler => "recompiler-scaffold",
    };
    let mut exec: Box<dyn SpuExecutor> = match args.backend {
        BackendKind::Interpreter => Box::new(InterpreterExecutor::default()),
        BackendKind::Recompiler => Box::new(RecompilerExecutor::new()),
    };
    let result = exec.execute(&program);

    if let Err(e) = fs::create_dir_all(&args.out_dir) {
        eprintln!("error: create out-dir: {e}");
        return ExitCode::from(3);
    }
    if let Err(e) = dump_state(&args.out_dir, &result, program.entry_pc, backend_name) {
        eprintln!("error: dump state: {e}");
        return ExitCode::from(3);
    }

    let exit_code = match &result.stop_reason {
        ExecutionStopReason::Stop(code) => {
            println!(
                "steps={} entry=0x{:x} pc=0x{:x} -- STOP code=0x{:04x}",
                result.steps_executed, program.entry_pc, result.final_state.pc, code
            );
            ExitCode::from(0)
        }
        ExecutionStopReason::ChannelStall { channel, is_write } => {
            println!(
                "steps={} entry=0x{:x} pc=0x{:x} -- STALL channel={channel} write={is_write}",
                result.steps_executed, program.entry_pc, result.final_state.pc
            );
            ExitCode::from(1)
        }
        ExecutionStopReason::MaxStepsExceeded => {
            println!(
                "steps={} entry=0x{:x} pc=0x{:x} -- MAX_STEPS reached without Stop ({})",
                result.steps_executed,
                program.entry_pc,
                result.final_state.pc,
                result.steps_executed
            );
            ExitCode::from(1)
        }
        ExecutionStopReason::Error(msg) => {
            println!(
                "steps={} entry=0x{:x} pc=0x{:x} -- ERROR {msg}",
                result.steps_executed, program.entry_pc, result.final_state.pc
            );
            ExitCode::from(2)
        }
    };

    exit_code
}

/// Read every PT_LOAD segment of `info`, validate it fits inside LS,
/// and produce an [`SpuProgram`] ready for any backend.
fn build_program_from_elf(
    info: &ElfInfo,
    bytes: &[u8],
    max_steps: u64,
) -> Result<SpuProgram, String> {
    let entry = info.e_entry as u32;
    if (entry as usize) >= SPU_LS_SIZE {
        return Err(format!("e_entry 0x{entry:x} out of LS range"));
    }
    if entry & 0x3 != 0 {
        return Err(format!("e_entry 0x{entry:x} not 4-byte aligned"));
    }

    let mut prog = SpuProgram::new(entry, max_steps);
    for ph in info.pt_load_iter() {
        let segment = extract_segment(ph, bytes)?;
        if let Some((lsa, data)) = segment {
            prog = prog.with_segment(lsa, data);
        }
    }
    prog.validate().map_err(|e| e.to_string())?;
    Ok(prog)
}

/// Pull the bytes of a single PT_LOAD into a `Vec<u8>`. Returns `None`
/// for BSS-only segments (filesz == 0); the caller skips those because
/// LS starts at zero.
fn extract_segment(ph: &ProgramHeader, bytes: &[u8]) -> Result<Option<(u32, Vec<u8>)>, String> {
    let offset = ph.p_offset as usize;
    let filesz = ph.p_filesz as usize;
    let lsa = ph.p_vaddr as u32;

    if filesz == 0 {
        return Ok(None);
    }
    if offset.saturating_add(filesz) > bytes.len() {
        return Err(format!(
            "PT_LOAD off=0x{offset:x} sz=0x{filesz:x} exceeds file (size={})",
            bytes.len()
        ));
    }
    if (lsa as usize).saturating_add(filesz) > SPU_LS_SIZE {
        return Err(format!(
            "PT_LOAD lsa=0x{lsa:x} sz=0x{filesz:x} exceeds 256 KB LS"
        ));
    }
    Ok(Some((lsa, bytes[offset..offset + filesz].to_vec())))
}

fn dump_state(
    out: &Path,
    result: &SpuExecutionResult,
    entry: u32,
    backend: &str,
) -> std::io::Result<()> {
    let snap = &result.final_state;
    let mut gpr = fs::File::create(out.join("gpr.csv"))?;
    for (i, reg) in snap.gpr.iter().enumerate() {
        writeln!(gpr, "r{i},{reg:032x}")?;
    }
    fs::write(
        out.join("pc.txt"),
        format!("entry=0x{entry:08x}\npc=0x{:08x}\n", snap.pc),
    )?;
    fs::write(out.join("ls.bin"), snap.ls.as_ref())?;

    let outcome = match &result.stop_reason {
        ExecutionStopReason::Stop(code) => format!("STOP code=0x{code:04x}"),
        ExecutionStopReason::ChannelStall { channel, is_write } => {
            format!("STALL channel={channel} write={is_write}")
        }
        ExecutionStopReason::MaxStepsExceeded => {
            format!("MAX_STEPS reached without Stop ({})", result.steps_executed)
        }
        ExecutionStopReason::Error(msg) => format!("ERROR {msg}"),
    };
    fs::write(
        out.join("summary.txt"),
        format!(
            "backend={backend}\nsteps={}\nentry=0x{entry:08x}\nfinal_pc=0x{:08x}\noutcome={outcome}\n",
            result.steps_executed, snap.pc
        ),
    )?;
    Ok(())
}
