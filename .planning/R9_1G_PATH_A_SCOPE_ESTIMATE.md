# R9.1g.1 — Path A scope estimate (PSL1GHT runtime init)

**Status:** INVESTIGATION (read-only, 2026-05-23 follow-up to R9.1f).
**Trigger:** User chose Path A (full PSL1GHT runtime init) over
Path B (main() bypass) for completeness/fidelity.
**Outcome:** Path A scope is **significantly larger than R9.1f's
"200-500 LOC" estimate** — closer to multi-week, multi-subsystem
work. This document presents the honest scope so the user can
re-decide.

## What R9.1f missed

R9.1f said Path A would be "200-500 LOC of loader patch code +
audit of all PSL1GHT runtime tables". The audit step is what
this R9.1g.1 actually did, and it surfaces that the work is
much broader.

## What PSL1GHT/lv2 actually expects at load time

`mailbox_v1.self` reveals **8 program headers**, of which the
last 3 are PSL1GHT-specific:

| PHDR | Type | vaddr | size | Role |
|------|------|-------|------|------|
| 0 | PT_LOAD R+X | 0x10000 | 0x1BF08 | `.text` (code) |
| 1 | PT_LOAD R+W | 0x30000 | 0x2D08 | `.data` / `.got` (FD table at +0x108!) |
| 2 | PT_LOAD R | 0x10000000 | 0x8E0 | `.opd` + `.rodata` |
| 3 | PT_LOAD R+W | 0x10010000 | 0x14D8 | `.bss` / heap area |
| 4 | PT_LOAD | (zero-size) | — | (placeholder) |
| 5 | PT_TLS | 0x32D08 | — | Thread-local storage descriptor |
| 6 | **PT_SCE_0x60000001** | 0x2BEC0 | 0x20 | `sys_process_param` (prio + stack size) |
| 7 | **PT_SCE_0x60000002** | 0x2BEE0 | 0x28 | `proc_param` (malloc/libc init hooks) |

The `.shstrtab` is also stripped — only DEBUG section names
remain. **All standard ELF section names** (`.text`, `.data`,
`.got`, `.opd`, `.rela.opd`, etc.) **are gone**. The runtime
must use the PT_SCE program headers as the only source of
truth.

## Decoded PT_SCE_0x60000001 (sys_process_param)

```
size:    0x20 (32 bytes)
magic:   0x13BCC5F6  
sdk_ver: 0x00009000
?:       0x00192001
prio:    0x000003E9 = 1001 (matches SYS_PROCESS_PARAM macro)
stack:   0x00010000 = 64 KB (matches USER_STACK_SIZE R9.1c chose)
malloc_pagesize: 0x00100000
?:       0x00000000
```

Confirms R9.1c's hard-coded stack constants. No new info beyond
what we already have.

## Decoded PT_SCE_0x60000002 (proc_param)

```
size:    0x28 (40 bytes)
magic:   0x1B434CEC  
version: 0x00000002
zero:    0x00000000
malloc_init_func: 0x0002BE94 → points to ANOTHER struct (44 bytes)
malloc_term_func: 0x0002BE94
fixed_alloc_func: 0x0002BE94
sys_proc_param_ptr: 0x0002BEC0 (= PT_SCE_0x60000001's address)
flags:   0x01010000
```

`malloc_init_func` (0x2BE94) points to a **44-byte struct** with
several internal pointers (0x2BCA8, 0x2BCB8, etc.). These are
malloc/free hook function pointers that **lv2's process loader
calls** before transferring control to `_start`.

## What PSL1GHT crt0 / lv2 loader actually does pre-`_start`

Based on the binary structure + standard PSL1GHT pattern:

1. **Parse sys_process_param** → set process priority + stack size.
2. **Allocate stack** → already done in R9.1c.
3. **Parse proc_param** → identify malloc/free/init hooks.
4. **Initialize TLS** → walk PT_TLS, allocate per-thread storage,
   set TLS base pointer.
5. **Populate `.opd` / `.got`** → here's the missing piece. The
   PSL1GHT runtime expects to populate FD pointers in the
   `.got` (at 0x30108 onwards) so PLT thunks like the one at
   `0x2AB60` can dereference them. The mechanism is likely:
   - The `.got` initially contains relative offsets or zero.
   - Lv2's loader (or PSL1GHT crt0) walks the function-pointer
     table and converts each entry into a proper FD pointer.
   - Each FD points to PHDR[2]'s `.opd` area (8-byte
     `{u32 code, u32 toc}` entries).
6. **Resolve dynamic library imports** → PSL1GHT links against
   `liblv2`, `libsysmodule`, `libsysutil`, `libio`. The runtime
   loads these at boot. We need to provide their entry points
   (or stub them) — for our 20 CC0 oracles, the only library
   functions used are syscall wrappers, which we already have
   per-syscall in `rpcs3-lv2-*` crates.
7. **Initialize libsysmodule / libc** → call the malloc init
   func + any libc constructor table.
8. **Transfer to `_start`** → finally, jump to the ELF entry.

## Scope estimate for Path A

| Step | Estimated LOC | Risk |
|------|---|------|
| 1. sys_process_param parser | 50 | Low — well-understood |
| 2. Stack alloc (DONE in R9.1c) | 0 | — |
| 3. proc_param parser | 80 | Medium — struct layout partly inferred |
| 4. TLS init | 200 | Medium — PT_TLS layout standard |
| 5. .opd / .got populator | **300-600** | **HIGH — exact mechanism unknown without reading PSL1GHT crt0 source / RPCS3 C++ loader** |
| 6. Library import resolution | 400-800 | HIGH — need to enumerate every PSL1GHT-exported function + map to existing Rust lv2 syscall crates |
| 7. libsysmodule / libc init | 100-300 | Medium — runtime hooks |
| 8. Transfer to `_start` | 20 | Low |
| **TOTAL** | **~1150-2050 LOC** | **HIGH overall** |

Plus debugging / testing time: each step has its own
"unimplemented X" cascade, similar to the opcode coverage gaps
we hit in R9.1b-e but at a higher layer of abstraction.

**Realistic wall-clock: 2-6 weeks of focused work**, possibly
longer if PSL1GHT's runtime has undocumented quirks (it almost
certainly does).

## Honest comparison with R9.1f estimate

R9.1f underestimated:
- Said Path A = 200-500 LOC → reality is ~1150-2050 LOC (5-10×
  larger).
- Did not account for library import resolution.
- Did not account for TLS init complexity.
- Did not account for the **iterative debug cycle** where each
  step's failure surfaces a new layer of needed init.

R9.1f was right that Path A is "the right thing" architecturally
— but the cost is multi-week, not multi-day.

## Strategic re-decision options

### Option α — Commit to Path A long-term

Accept that R9 is now a multi-week wave. Pace at ~1-2 commits
per slice, with each slice being one of the 8 init steps above.
Total ~15-25 commits to reach `main()` running for ANY oracle.
Repeated cycles per oracle to handle their specific needs.

**Pros:** Faithful to behavior-freeze contract. Builds the
actual RPCS3 process loader. Future-proof for all PSL1GHT
binaries (homebrew, commercial later).

**Cons:** 2-6 weeks of focused work before ANY oracle runs
end-to-end. Hard to estimate when done.

### Option β — Time-box Path A, fallback to Path B

Commit to 3-5 days on Path A. If by then we have at least
`mailbox_v1` running through `main()`, continue. Otherwise,
implement Path B (main() bypass) as the **near-term** R9
deliverable and re-attempt Path A later when the rest of the
emulator catches up.

**Pros:** Bounded commitment. Empirical decision point.

**Cons:** May end up doing both Path A AND B partial work.

### Option γ — Pause R9 entirely, re-strategize

The SPU stack is MVP-complete at R8.5e (20 oracles). R9's
"integrate end-to-end" goal may be premature given the lv2
process loader scope. Consider:
- Continue R8.x deepening (atomics, multi-SPU, PUTRL family)
- OR work on a different RPCS3 subsystem (RSX scaffolding, audio,
  filesystem) that may be more tractable.

The R8.5e empirical insight ("PUTL stall passed zero-code")
was that the SPU stack saturated. But maybe other subsystems
haven't, and they offer better near-term value than lv2 loader.

**Pros:** Avoids committing weeks to lv2 loader without
broader project alignment.

**Cons:** Loses momentum on R9; the 20 oracles remain capture-
only validated until we revisit.

## R9.1g.1 outcome

**This commit ships only this analysis doc — no code.** The
honest finding is: Path A is multi-week scope, not multi-day.
The user's choice of Path A "because more complete" remains
valid, but they should re-decide knowing the actual scope.

## Recommendation

**Option β (time-box Path A, ~5 days, then re-decide).** This
gives empirical progress without open-ended commitment. If
after R9.1g.2 + R9.1g.3 (sys_process_param + proc_param + TLS
init) we have NO oracle running, that's a strong signal that
Path A is too expensive for the current project phase.

If the user prefers full commitment (Option α), proceed
without time-box but be ready to iterate for 2-6 weeks.

If the user prefers Option γ, this concludes the R9 wave for
now; revisit when project priorities change.

## Validation

- cargo test --workspace --tests --release: 264 blocks, 0 fails
- 20 SPU oracles ✓
- This commit lands ONLY this doc (no code)
- No SHA bumps
- Behavior-freeze contract preserved
