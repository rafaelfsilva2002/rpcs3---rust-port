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

    // R9.1k — scan PHDR[3] data segment post-run for the
    // `__syscalls` table (~41 sequential u32 BE function-
    // pointers, each pointing into .text 0x10000..0x2C000 or
    // the .opd FD area near 0x10000000).
    {
        const DATA_BASE: u32 = 0x10010000;
        const DATA_SIZE: u32 = 0x14D8;
        let mut buf = vec![0u8; DATA_SIZE as usize];
        if core.mem.read(DATA_BASE, &mut buf).is_ok() {
            let mut best_start = 0u32;
            let mut best_len = 0usize;
            let mut run_start = 0u32;
            let mut run_len = 0usize;
            let mut i = 0usize;
            while i + 4 <= buf.len() {
                let v = u32::from_be_bytes([
                    buf[i], buf[i + 1], buf[i + 2], buf[i + 3],
                ]);
                let looks_like_text_ptr = (0x10000..0x2_C000).contains(&v);
                if looks_like_text_ptr {
                    if run_len == 0 {
                        run_start = DATA_BASE + i as u32;
                    }
                    run_len += 1;
                    if run_len > best_len {
                        best_len = run_len;
                        best_start = run_start;
                    }
                } else {
                    run_len = 0;
                }
                i += 4;
            }
            if best_len > 0 {
                eprintln!(
                    "[R9.1k scan] longest text-ptr run in .data: \
                     start=0x{best_start:08x} len={best_len} u32s",
                );
                // Print first ~16 ptrs.
                let mut sub = vec![0u8; (best_len.min(16) * 4) as usize];
                if core.mem.read(best_start, &mut sub).is_ok() {
                    for k in 0..(sub.len() / 4) {
                        let v = u32::from_be_bytes([
                            sub[k * 4], sub[k * 4 + 1],
                            sub[k * 4 + 2], sub[k * 4 + 3],
                        ]);
                        eprintln!(
                            "[R9.1k scan]   [{k:02}] +0x{:04x}: 0x{v:08x}",
                            k * 4,
                        );
                    }
                }
            } else {
                eprintln!("[R9.1k scan] NO text-pointer runs in .data — __syscalls not populated; constructors likely did NOT run");
            }
        }
    }

    // R9.1h slice 2 — dump full import plan AFTER run_self
    // initializes it.
    if let Some(plan) = core.import_plan.as_ref() {
        eprintln!(
            "[R9.1h dump] {} imported stubs:",
            plan.stubs.len()
        );
        for s in &plan.stubs {
            eprintln!(
                "  {}::0x{:08x} trampoline=0x{:08x} addrs=0x{:08x}",
                s.module_name, s.nid, s.trampoline_vaddr, s.addrs_slot,
            );
        }
    } else {
        eprintln!("[R9.1h dump] no import plan");
    }

    // R9.1c-R9.1f diagnostic — dump key PPU state at exit so each
    // smoke run reports where the boot path stopped and what came
    // after. R9.1f extension: GPR0-12 + CTR + LR for bug isolation
    // when control-flow corruption (CIA in unmapped region) is
    // suspected.
    eprintln!(
        "[R9.1f diag] post-run: CIA=0x{:08x} LR=0x{:016x} CTR=0x{:016x}",
        core.ppu.cia, core.ppu.lr, core.ppu.ctr,
    );
    for r in 0..=12u8 {
        eprintln!(
            "[R9.1f diag]   r{:<2} = 0x{:016x}",
            r, core.ppu.gpr[r as usize]
        );
    }

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
