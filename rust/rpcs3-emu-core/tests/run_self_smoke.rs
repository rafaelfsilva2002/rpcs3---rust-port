//! R9.1a smoke test — `EmuCore::run_self` parses an fself binary,
//! loads its PPU ELF, and starts executing.
//!
//! The PPU side of the SPU oracle fixtures uses SPU group syscalls
//! (`sys_spu_initialize`, `sys_spu_thread_group_create`,
//! `sys_spu_thread_group_start`, `sys_spu_thread_group_join`) which
//! R9.1a does **not** wire into `dispatch_syscall`. That work is
//! R9.1b. R9.1a proves only that the boot path itself works.
//!
//! Expected R9.1a outcomes (any of these passes the smoke test):
//!
//! 1. `Error::UnsupportedSyscall { number, cia }` — PSL1GHT's
//!    `_start` reached a syscall not yet wired. The number tells
//!    R9.1b's next arm.
//! 2. `Error::Interpreter(Unimplemented { .. })` — PPU interpreter
//!    coverage gap (PSL1GHT `_start` uses an opcode outside the
//!    current iteration-2 subset). Not an R9 issue — surfaces a
//!    pre-existing PPU stack limitation that R9 acknowledges.
//! 3. `Error::StepsExhausted` — boot path runs through 1M steps
//!    without hitting one of the above. Surprising but not a R9
//!    regression.
//!
//! What is a R9 REGRESSION (and panics the test):
//!
//! - `Error::Elf(*)` / `Error::ElfNotLoadable` — SCE parsing or
//!   ELF load broke. R9.1a's `run_self` wrapper has a bug.
//! - `Error::Memory(*)` — guest memory faulted unexpectedly
//!   (the loader or run_self mis-set up VM).
//! - A clean `Ok(_)` exit — surprising for R9.1a (SPU group
//!   syscalls aren't wired); log + investigate.

use std::path::PathBuf;

use rpcs3_emu_core::{EmuCore, Error};

fn mailbox_v1_self_path() -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.pop(); // rust/
    p.pop(); // rpcs3-master/
    p.push("behavior-freeze");
    p.push("fixtures");
    p.push("spu");
    p.push("sources");
    p.push("single_spu_mailbox_v1");
    p.push("build");
    p.push("single_spu_mailbox_v1.self");
    p
}

#[test]
fn r9_1a_run_self_parses_mailbox_v1_and_executes_ppu() {
    let self_path = mailbox_v1_self_path();
    if !self_path.exists() {
        // Fixture .self files are gitignored (built locally by Docker)
        // — skip the test gracefully when the artifact is absent
        // rather than failing CI environments that don't run the
        // PSL1GHT toolchain.
        eprintln!(
            "[R9.1a smoke] skip: {} not present (run Docker build first)",
            self_path.display()
        );
        return;
    }

    let self_bytes = std::fs::read(&self_path).expect("read .self");
    assert!(
        self_bytes.len() >= 4,
        ".self too small to have SCE magic"
    );
    assert_eq!(
        &self_bytes[..4],
        b"SCE\0",
        ".self does not start with SCE magic"
    );

    let mut core = EmuCore::new();
    // Give the PPU enough budget to chew through PSL1GHT's `_start`
    // and reach `main()` without exhausting the step counter.
    core.step_budget = 1_000_000;

    let result = core.run_self(&self_bytes);

    // R9.1c diagnostic — dump key PPU state at exit so each smoke
    // run reports where the boot path stopped and what came after.
    // The 20-oracle integration goal needs this trail to be
    // monotonically deeper across R9.x slices.
    eprintln!(
        "[R9.1c diag] post-run state: CIA=0x{:08x} r1=0x{:016x} r2=0x{:016x} r3=0x{:016x} LR=0x{:016x}",
        core.ppu.cia,
        core.ppu.gpr[1],
        core.ppu.gpr[2],
        core.ppu.gpr[3],
        core.ppu.lr,
    );

    match result {
        Ok(report) => {
            // R9.1a is NOT expected to reach process exit cleanly
            // (SPU group syscalls aren't wired yet). A clean exit
            // here would be surprising — log it for diagnosis.
            eprintln!(
                "[R9.1a smoke] surprising clean exit: status={} tty_ch3={:?}",
                report.exit_status.status, report.tty_output.get(3)
            );
        }
        Err(Error::UnsupportedSyscall { number, cia }) => {
            // Expected outcome (1) — log the first unimplemented
            // syscall so R9.1b knows where to start wiring.
            eprintln!(
                "[R9.1a smoke] reached unimplemented syscall #{number} \
                 at CIA 0x{cia:08x}. R9.1b targets: wire this syscall \
                 + chain through SPU group lifecycle."
            );
        }
        Err(Error::Interpreter(ie)) => {
            // Expected outcome (2) — PPU interpreter coverage gap.
            // Not a R9 regression; it's a pre-existing limitation
            // in the ppu-interpreter iteration-2 subset. R9 will
            // need it covered eventually but it's not gating R9.1a.
            eprintln!(
                "[R9.1a smoke] reached PPU interpreter coverage gap: \
                 {ie:?}. Not an R9 regression — surfaces a PPU stack \
                 limitation that R9.1b/follow-ups will need."
            );
        }
        Err(Error::StepsExhausted) => {
            // Expected outcome (3) — surprising but not a regression.
            eprintln!(
                "[R9.1a smoke] PPU boot ran 1M steps without exiting \
                 or hitting unimpl. Boot loop suspected."
            );
        }
        Err(e) => {
            // Anything else is a real regression — surface for triage.
            panic!(
                "[R9.1a smoke] unexpected error: {e:?} (R9 regression: \
                 SCE/ELF parse or VM setup broke)"
            );
        }
    }
}
