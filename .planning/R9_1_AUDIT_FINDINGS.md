# R9.1 Audit Findings — LV2/PPU integration gaps mapped

**Status:** AUDIT (read-only, drafted 2026-05-23 post R9 plan approval).
**Input:** [`R9_LV2_PPU_INTEGRATION_PLAN.md`](./R9_LV2_PPU_INTEGRATION_PLAN.md) § 5 (5 open questions).
**Deliverable:** Concrete answers + refined R9.1 implementation scope.
**Decision:** This audit unblocks R9.1's code phase. Headline below.

## Headline

R9.1 is **significantly smaller than the original plan estimated**.
The LV2/PPU stack is closer to runnable than § 2.1 acknowledged:
`rpcs3-emu-core::EmuCore` already has `load_elf` + `run` +
`dispatch_syscall` + `mem` (`SparseBackend`) wired together, and
`rpcs3-loader-elf-self::parse_self_ext_header` exposes the
`elf_offset` field that unwraps fself SCE headers to plaintext
PPU64 ELF.

**The actual R9.1 implementation work is:**

1. Add an `EmuCore::run_self(self_bytes)` thin wrapper that unwraps
   the SCE header and delegates to `load_elf` + `run`.
2. Wire 7-8 new arms in `dispatch_syscall` for the SPU thread
   group lifecycle + sys_tty_write.
3. The SPU group syscall arms must chain into actual SPU
   execution (current `EmuCore::run_spu_group_single` is
   test-only and bypasses the dispatcher).
4. Integration test that boots `single_spu_mailbox_v1.self` and
   asserts the captured TTY matches `[mailbox_v1] OK status=0x453`
   (or whatever string the fixture's printf produces).

No new crates required. No greenfield modules. The mailbox_v1
fixture exercises ~7 syscalls + SPU mailbox channels — that
surface is bounded.

## Answers to the 5 open questions

### Q1 — Is `rpcs3-ppu-decoder` a real gap?

**Answer: VESTIGIAL. No gap.**

`rust/rpcs3-ppu-decoder/` does not exist as a directory.
`rpcs3-ppu-interpreter` (3,862 LOC) inlines its own
fetch/decode/execute cycle via the bit-extraction helpers in
`rpcs3-ppu-opcodes`. The "decoder" crate name in § 2.1 of the
R9 plan was speculation — there is no separate decoder crate to
fill.

No action needed.

### Q2 — EA / VM model

**Answer: `rpcs3-memory-backing::SparseBackend` (701 LOC).**

```
rpcs3-memory-backing/src/lib.rs:
    pub struct SparseBackend { /* HashMap<page_idx, [u8; 4096]> */ }
    impl SparseBackend {
        pub fn read(&self, addr: u32, dst: &mut [u8]) -> Result<(), Error>
        pub fn write(&mut self, addr: u32, src: &[u8]) -> Result<(), Error>
        pub fn read_le<T>(&self, addr: u32) -> Result<T, Error>
        // ... plus write_le, alloc_page, etc.
    }
```

`EmuCore::mem: SparseBackend` is the PPU's memory backing.
Existing `dispatch_syscall` arms already use it (e.g. syscall 12
writes the result `nump` pointer via `self.mem.write(nump,
&buf)?`).

For the SPU side, the existing bridge runtime uses
`vm::_ptr<u8>(ea)` in C++; the Rust equivalent for integration
testing would route SPU MFC DMA through the same `SparseBackend`.
This means: the EA addresses the SPU sees are PPU-VM addresses,
backed by the same sparse map. **No additional crate work needed.**

### Q3 — PPU entry sequence

**Answer: `EmuCore::load_elf` already handles ELF load + PC set;
PSL1GHT `_start` startup stubs are LIKELY UNNECESSARY for our
fixtures.**

```
rpcs3-emu-core/src/lib.rs:
    pub fn load_elf(&mut self, elf_bytes: &[u8]) -> Result<ElfInfo, Error> {
        let info = parse_elf(elf_bytes)?;
        if !info.is_ppu64() { return Err(Error::ElfNotLoadable("not a PPU64 ELF")); }
        // ... loads segments into self.mem, sets PC to entry
    }
```

PSL1GHT's typical `_start` does minimal init (TLS, argc/argv setup)
before calling `main()`. Our fixtures don't use TLS, don't read
argc/argv, and have a single `main()`. The ELF's e_entry points
to PSL1GHT's `_start`, which calls `main()` directly. The risk
is in what `_start` runs before `main()` — possibly a few
syscalls for env setup. R9.1 will surface these as
"unimplemented syscall N" errors and we add arms as they come up.

**Action: confirm during R9.1 implementation by running the
fixture and observing where it fails. Don't preemptively
implement `_start` stubs.**

### Q4 — printf / stdout

**Answer: `rpcs3-lv2-tty::sys_tty_write` exists (323 LOC) with
per-channel output buffer. Not yet wired into `EmuCore`'s
syscall dispatcher.**

```
rpcs3-lv2-tty/src/lib.rs:
    pub const REGISTERED_ENTRY_POINTS: &[&str] = &["sys_tty_read", "sys_tty_write"];
    // sys_tty_write appends to per-channel output buffer
```

The fixtures end with `printf("[fixture] OK ...")`. PSL1GHT's
`printf` eventually calls `sys_tty_write` (syscall 403 on PS3).
We need to:
1. Add syscall 403 arm in `EmuCore::dispatch_syscall`.
2. Expose a way to read the captured TTY output post-run
   (`EmuCore::tty_output() -> &str` or similar).
3. Pass that to the integration test for the assertion.

**Action: ship in R9.1.**

### Q5 — Scope: extend `rpcs3-emu-core` or new `rpcs3-app-runner`?

**Answer: EXTEND `rpcs3-emu-core`. No new crate.**

`EmuCore` already integrates LV2 + PPU thread + SPU group +
memory backing. Adding `run_self` + the missing syscall arms is
strictly additive. A separate `rpcs3-app-runner` crate would add
dependency-graph complexity for no benefit.

The integration test goes in `rpcs3-emu-core/tests/` (new
directory; `rpcs3-emu-core/tests/` doesn't currently exist).

## Refined R9.1 scope

### Code work

1. **`rpcs3-emu-core::EmuCore::run_self(self_bytes: &[u8]) -> Result<RunReport, Error>`**
   - Validates SCE magic + parses `SelfExtHeader`
   - Calls `EmuCore::load_elf(&self_bytes[elf_offset..])`
   - Calls `EmuCore::run()`
   - Returns `RunReport { exit_status, tty_output }`

2. **New syscall arms in `dispatch_syscall`:**
   - **403 sys_tty_write(ch, data_ptr, len, pwritelen_ptr)** —
     read data from guest mem, append to per-channel tty buffer,
     write bytes-written count back to *pwritelen_ptr.
   - **160 sys_spu_initialize(max_usable_spu, max_raw_spu)** —
     verify spu count via lv2 process state.
   - **156 sys_spu_image_import(*image, src_ptr, type)** — read
     SPU image from guest mem at src_ptr, parse via
     `rpcs3-loader-elf-self` ELF path (SPU ELF, not PPU ELF; new
     code), store into image. The `image` arg is a struct in
     guest memory: write back the SPU image descriptor.
   - **170 sys_spu_thread_group_create** — already imported in
     emu-core; chain via syscall.
   - **172 sys_spu_thread_group_destroy** — same.
   - **173 sys_spu_thread_group_start** — same, PLUS actually
     execute the SPU threads to completion (this is the chain-in
     gap — currently `run_spu_group_single` is test-only).
   - **174 sys_spu_thread_group_suspend** (optional, fixtures
     don't use) — defer.
   - **176 sys_spu_thread_group_join** — return captured
     cause + status (= OUT_MBOX value the SPU sentinel-wrote).
   - **169 sys_spu_thread_initialize(*thread_id, group_id,
     thread_index, *image, *attr, *args)** — store args into
     the lazy-launch state; actual execution happens at
     group_start.

3. **Integration test:**
   - `rpcs3-emu-core/tests/mailbox_v1_end_to_end.rs`
   - Load `behavior-freeze/fixtures/spu/sources/single_spu_mailbox_v1/build/single_spu_mailbox_v1.self`
   - `EmuCore::run_self`
   - Assert `tty_output` matches expected canonical TTY string.

### Defer to R9.2+

- Memory backing for EA-mapped PPU `static u8 ea_buf[]` arrays.
  Mailbox_v1 doesn't use these — defer to R9.2 (GET/PUT fixtures
  trigger this need).
- Multi-SPU dispatch. Mailbox_v1 uses 1 SPU per group.
- Image decryption. fself binaries are unencrypted (elf_offset
  jumps over the SCE header to plaintext ELF).
- Stub-ing other PSL1GHT-injected init syscalls — surface as
  they fail.

### Out of scope explicitly

- Recompiler path for PPU (interpreter only).
- RSX / graphics.
- Audio.
- Multi-PPU-thread.

## Estimated effort

- **Code phase**: ~500 LOC of new emu-core arms + ~50 LOC of
  `run_self` wrapper + ~80 LOC of integration test.
- **Debug phase**: realistic, because this is the first time
  many subsystems integrate. Expect 1-2 cycles of "unimplemented
  syscall N" → add arm → re-run.
- **Total**: 1-2 sessions for R9.1 mailbox_v1 oracle.

## Open question deferred to R9.1 implementation

How does the existing `EmuCore::run_spu_group_single` reconcile
with a syscall-dispatched start? Two possible designs:

- **Eager**: `sys_spu_thread_group_start` runs all SPU threads
  synchronously to completion. Simple, matches single-PPU
  single-SPU fixture topology. `sys_spu_thread_group_join` just
  reads the captured outcome.
- **Lazy / interleaved**: SPU threads run in a separate harness
  step. Required for fixtures where the PPU does work BETWEEN
  start and join (none of our 20 fixtures do this).

**Default for R9.1**: eager. Revisit if R9.4+ stall fixtures
or future multi-PPU work demands lazy.

## Recommendation

Approve audit findings + ship R9.1 in two commits:
- **R9.1a**: `run_self` wrapper + `run_report` struct + tty arm
  (160 LOC, tests). No SPU group integration yet — proves
  `EmuCore::run_self` boots a PPU binary and captures TTY.
- **R9.1b**: SPU group syscall arms (image_import, group_create,
  thread_initialize, group_start chains into SPU exec,
  group_join returns outcome) + mailbox_v1 integration test
  (350-450 LOC).

R9.1a-then-R9.1b lets us validate the PPU boot path independently
before adding SPU coupling — cheaper debugging if the boot path
itself has issues.

## Confidence

- Q1: high (verified directory absence).
- Q2: high (verified SparseBackend exists + used by EmuCore).
- Q3: medium (PSL1GHT `_start` not inspected; expecting minimal
  surface but could surprise us).
- Q4: high (verified sys_tty_write impl present).
- Q5: high (verified EmuCore has the integration points).
- Scope estimate: medium (depends on Q3 surprises and on
  how many lv2 calls PSL1GHT's `_start` injects before main).
