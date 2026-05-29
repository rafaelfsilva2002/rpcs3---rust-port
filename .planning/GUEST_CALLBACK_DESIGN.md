# Guest-PPU-Callback Implementation Blueprint

Status: DESIGN (pre-implementation). Date: 2026-05-29.
Scope: introduce the first re-entrant guest-PPU-call primitive in the RPCS3 -> Rust port and use it to drive `cellSysutilRegisterCallback` + `cellSysutilCheckCallback` end-to-end against a real PSL1GHT CC0 fixture.

This is greenfield control flow. All 137 wired HLE arms today only READ args from `gpr` and WRITE results; none ever invoke guest code. `call_guest_function` is the first primitive that runs guest PPU code from inside an HLE arm and resumes the original caller afterward.

---

## 1. Goal and Scope

### 1.1 The primitive
Add an inherent method on `EmuCore` (next to `run()` at `rust/rpcs3-emu-core/src/lib.rs:974`):

```rust
fn call_guest_function(&mut self, fd_or_code: u32, args: &[u64]) -> Result<u64, Error>
```

It must:
- Resolve `fd_or_code` to a real `.text` code address (compact 4-byte PSL1GHT FD deref, with the executable-segment heuristic from `load_elf` `lib.rs:564-578`).
- Snapshot the PPU register frame.
- Seed args into `r3..=r10`, install a 4-byte-aligned RETURN-SENTINEL into `lr`, set `cia = code_addr`.
- Drive a nested run loop that mirrors `EmuCore::run` (`step` + on `Syscall` call `dispatch_syscall`) until `cia == SENTINEL` or a budget is exhausted. This makes `dispatch_syscall` RE-ENTRANT (HLE arm -> call_guest_function -> nested loop -> dispatch_syscall), which Rust accepts because all state lives behind `&mut self`.
- Capture `r3` as the `u64` return value; memory side-effects in `self.mem` persist automatically (this is what we want).
- Restore the saved register frame so the outer HLE arm can do its normal `cia = lr & !3; return Ok(None)` and resume the original guest caller.

### 1.2 The first target
`cellSysutilCheckCallback` (NID `0x189a74da`) + `cellSysutilRegisterCallback` (NID `0x9d98afa0`).

- `RegisterCallback(slot, func_fd_ptr, userdata)` — pure table mutation, NO guest call.
- `CheckCallback()` — drains a pending-event queue and, per pending dispatch, calls `call_guest_function(cb.fn_addr, &[status, param, userdata])`. The guest callback signature (cellSysutil.h:155) is `void cb(u64 status, u64 param, vm::ptr<void> userdata)` => `r3=status`, `r4=param`, `r5=userdata`. It returns VOID, so `r3` is ignored for this family (but the primitive still supports non-void returns for future families like jpgDec/pngDec malloc callbacks).

The good news: the Rust library `rpcs3-hle-cellsysutil` ALREADY contains the full slot model (`Callback{fn_addr,user_data}`, `CallbackTable[8]`, `CallbackQueue`, `cell_sysutil_register_callback`, `cell_sysutil_check_callback -> Vec<PendingDispatch>`). It just has ZERO EmuCore wiring. This blueprint is mostly EmuCore plumbing + the new primitive, not new library code.

---

## 2. `call_guest_function` — exact algorithm

Reference template: upstream `ppu_thread::fast_call` (PPUThread.cpp:2852-2947) + `_func_caller` (PPUCallback.h:171-179). Rust template: `EmuCore::run` loop body (`lib.rs:980-988`).

### 2.1 Register frame to save/restore
`PpuThread` (`rust/rpcs3-ppu-thread/src/lib.rs:160-197`) has ALL register fields `pub`, but is NOT `Clone` (it carries a `CpuState` field). So copy the arch register fields individually into a lightweight local snapshot struct.

Save (mandatory): `gpr` (full `[u64;32]` — easiest and safest), `cia`, `lr`, `ctr`, `cr`, `xer`, `fpscr`, `vrsave`. Also save `fpr` and `vr` because a guest callback may clobber them. Cheapest correct choice: snapshot the entire arch register set.

```rust
struct PpuRegSnapshot {
    gpr: [u64; 32],
    fpr: [f64; 32],
    vr:  [u128; 32],
    cr:  CrBits,
    fpscr: u32,
    lr: u64,
    ctr: u64,
    xer: Xer,
    cia: u32,
    vrsave: u32,
}
```

Do NOT save `self.mem` — guest writes during the callback MUST persist (that is the observable behavior we capture).

### 2.2 Step-by-step

1. **Resolve code address** (reuse `load_elf` logic at `lib.rs:564-578`; ideally extract a shared helper `fn resolve_code_addr(&self, v: u32) -> Result<u32, Error>`):
   - If `v` lands inside an EXECUTABLE `PT_LOAD` segment (`p_flags & 0x1 != 0`), treat `v` as a direct code address.
   - Else FD-deref: `let mut b=[0u8;4]; self.mem.read(v, &mut b)?; let code = u32::from_be_bytes(b);`.
   - CRITICAL: use raw `self.mem.read` + `u32::from_be_bytes` (guest memory is BIG-endian). Do NOT use `read_le` (`rpcs3-memory-backing/src/lib.rs:335` is little-endian and would byte-swap the address).
   - r2/TOC: PSL1GHT compact FDs are code-only (import stubs hardcode `toc=0`, `lib.rs:904-906`, and nothing in the codebase reads `gpr[2]`). Leave `gpr[2]` untouched for current targets. Forward-compat note: a real signed-PRX OPD is 8 bytes `{addr, rtoc}`; if non-PSL1GHT binaries appear, also read `FD+4` into `gpr[2]`.

2. **Snapshot** the register frame (section 2.1) into a local.

3. **Seed the call frame**:
   - `for (i, a) in args.iter().take(8).enumerate() { self.ppu.gpr[3 + i] = *a; }` (PPC64 GPR args = `r3..=r10`, max 8).
   - `self.ppu.lr = SENTINEL as u64;`
   - `self.ppu.cia = code_addr;`
   - (Optional, leave as-is for PSL1GHT: `gpr[1]` stack frame. Upstream aligns SP to 16 and reserves 0x70; CellSysutilCallback's 3 GPR args all fit in registers, no stack spill needed. Skip unless a fixture proves otherwise.)

4. **Pick the SENTINEL** (see section 2.3) — a 4-byte-aligned address that is NOT a real code address and NOT inside the import-stub window.

5. **Nested run loop** (mirror `run`, but stop on the sentinel instead of budget-only):

```rust
const SENTINEL: u32 = 0xD0FF_0000; // 4-aligned, outside stub window 0xD0010000..0xD0020000
let budget = if self.step_budget == 0 { usize::MAX } else { self.step_budget };
let mut hit = false;
for _ in 0..budget {
    if self.ppu.cia == (SENTINEL & !0x3) { hit = true; break; }
    match step(&mut self.ppu, &mut self.mem)? {
        StepOutcome::Continue => {}
        StepOutcome::Syscall => {
            // re-entrant: nested HLE/import calls inside the callback resolve here
            if let Some(_exit) = self.dispatch_syscall()? {
                // callback triggered an exit-status arm; propagate or break per policy
                break;
            }
        }
    }
}
if !hit { /* restore frame, then */ return Err(Error::CallbackStepsExhausted); }
```

   - The check `cia == SENTINEL & !0x3` works because the guest callback's terminal `blr` sets `cia = (lr as u32) & !0x3` (`rpcs3-ppu-interpreter/src/lib.rs:2147-2157`). Installing `lr = SENTINEL` (already aligned) makes the guest return land exactly on the sentinel.
   - Do NOT use the interpreter crate's `run_n` (`lib.rs:3225`): it stops at the first `Syscall` and does NOT dispatch imports, so a callback that calls an import would deadlock.

6. **Capture return**: `let ret = self.ppu.gpr[3];` (ignored for void cellSysutil callbacks).

7. **Restore** the snapshot into `self.ppu` (all saved fields). This re-establishes the outer HLE arm's `cia`/`lr` so its trailing `cia = lr & !3; return Ok(None)` resumes the original caller correctly.

8. `Ok(ret)`.

### 2.3 Sentinel choice
- `0xD0FF_0000`: 4-byte aligned (survives `& !0x3`), never a real `.text` address, and OUTSIDE the import-stub window `0xD0010000..0xD0020000` so `is_in_import_stub_region` (`lib.rs:3005-3008`) returns false and the run loop never mistakes a sentinel landing for a trampoline `sc`.
- It must never be a mapped executable page. If it were ever mapped, `step` would try to execute it instead of the loop catching `cia == sentinel` first — the loop checks BEFORE `step`, so this is safe even if mapped, but keep it unmapped for clarity.

### 2.4 Borrow handling for `&mut self`
- `step(&mut self.ppu, &mut self.mem)` borrows two DISJOINT fields — the borrow checker accepts this (`lib.rs:981`, signature `rpcs3-ppu-interpreter/src/lib.rs:652`).
- The re-entrancy ownership trap: the OUTER `dispatch_syscall` holds `let plan = self.import_plan.as_ref()` (immutable borrow of `self`, `lib.rs:1012`) alive across the whole `match`. A NID arm that wants `self.call_guest_function(...)` (needs `&mut self`) MUST first drop that borrow. The existing dispatcher already copies scalars (`nid`, `module`, `r3_in`) into owned locals before the `match` (`lib.rs:1014-1017`). The callback-using arms MUST extract everything they need (the slot table read produces owned `Vec<PendingDispatch>`, owned `u32`/`u64`) and MUST NOT reference `plan`/`stub_meta` after calling `call_guest_function`. Cleanest insertion: gather the `Vec<PendingDispatch>` and all scalars into owned locals, ensure no live reference into `import_plan` remains, then loop calling `self.call_guest_function(...)`.

### 2.5 EmuCore fields/methods cited
- `EmuCore::run` step loop: `lib.rs:974`, body `980-988`.
- `EmuCore::dispatch_syscall`: `lib.rs:996`; import-stub detect `1011-1013`; match-nid block start `1033`; canonical return idiom `1045-1047` / `1817-1818`.
- `EmuCore::load_elf` FD-deref/heuristic: `lib.rs:554-579`.
- `step()` signature: `rpcs3-ppu-interpreter/src/lib.rs:652`. `StepOutcome{Continue,Syscall}`: `70-82`. `blr` handler: `2147-2157`.
- `PpuThread` pub fields: `rpcs3-ppu-thread/src/lib.rs:160-197`.
- `step_budget` default 100_000: `lib.rs:467`. `Error::StepsExhausted`: `lib.rs:205,990` (add a sibling `Error::CallbackStepsExhausted`).
- Memory `read`/`write`: `rpcs3-memory-backing/src/lib.rs:295/315` (raw, BE-correct).

---

## 3. State — EmuCore fields to add

EmuCore HLE-state fields live at `lib.rs:434-442` (`sysmodule: SysmoduleManager`, `netctl: NetCtlManager`, `videoout: VideoOutManager`), initialized in `EmuCore::new` at `lib.rs:473-475`. Mirror that pattern:

1. Extend the import at `lib.rs:62-64` to pull in the existing library types:
   `use rpcs3_hle_cellsysutil::{Callback, CallbackTable, CallbackQueue, PendingDispatch, cell_sysutil_register_callback, cell_sysutil_unregister_callback, cell_sysutil_check_callback};`
2. Add fields (alongside `sysmodule`/`netctl`/`videoout`):
   ```rust
   pub sysutil_callbacks: CallbackTable, // slot -> Option<Callback{fn_addr,user_data}>; 8 slots (lib already CB_SLOT_MAX=8)
   pub sysutil_queue: CallbackQueue,     // VecDeque<(event:u32, param:u64)> pending dispatches
   ```
3. Init in `EmuCore::new`: `sysutil_callbacks: CallbackTable::default(), sysutil_queue: CallbackQueue::default()` (or whatever the library's constructors are).

No new library code is required — `rpcs3-hle-cellsysutil` already provides:
- `Callback{ pub fn_addr: u32, pub user_data: u32 }` (`src/lib.rs:112-116`) — the exact (func_ptr, userdata) shape.
- `CallbackTable{ slots:[Option<Callback>;8] }` with register/unregister/get (`148-185`).
- `CallbackQueue` (`120-144`).
- `cell_sysutil_check_callback(table, &mut queue) -> Vec<PendingDispatch>` (`211-224`) where `PendingDispatch{ cb, event:u32, param:u64 }` (`227-232`).

This mirrors how `SysmoduleManager`/`NetCtlManager` are owned by EmuCore and mutated by their NID arms.

NOTE on slot count divergence: the Rust library uses 8 slots; upstream RPCS3 uses 4 (`cellSysutil.cpp:82`, bound `slot >= 4 -> 0x8002b102`). For byte-exact behavior freeze, the dispatcher arm should enforce the upstream bound `slot >= 4 => return CELL_SYSUTIL_ERROR_VALUE (0x8002b102)` even though the table physically has 8 slots, OR adjust `CB_SLOT_MAX` to 4. Flag for decision (see section 6).

---

## 4. NID arms

Real NIDs (computed via `ppu_generate_id` = first 4 bytes of `SHA1(name + suffix)` read LITTLE-endian; suffix `\x67\x59\x65\x99\x04\x25\x04\x90\x56\x64\x27\x49\x94\x89\x74\x1A`; verified against already-wired `0x40e895d3`/`0x938013a0`). CONFIRM at runtime against the fixture's libstub before committing.

| Function | NID | Behavior |
|---|---|---|
| `cellSysutilRegisterCallback` | `0x9d98afa0` | store slot; NO guest call |
| `cellSysutilCheckCallback` | `0x189a74da` | drain queue; per dispatch call guest cb |
| `cellSysutilUnregisterCallback` | `0x02ff3c1b` | clear slot |
| `cellSysutilRegisterCallbackDispatcher` | `0x886d0747` | store-only (32-slot table, NEVER invoked) |
| `cellSysutilUnregisterCallbackDispatcher` | `0x40c7538e` | clear dispatcher slot |

### 4.1 RegisterCallback arm (no guest call)
```rust
0x9d98afa0 => { // cellSysutilRegisterCallback(slot, func_fd, userdata)
    let slot = self.ppu.gpr[3] as u32;
    let func = self.ppu.gpr[4] as u32;
    let userdata = self.ppu.gpr[5] as u32;
    let rc = cell_sysutil_register_callback(&mut self.sysutil_callbacks, slot, func, userdata);
    self.ppu.gpr[3] = rc as u64; // CELL_OK or 0x8002b102 if slot out of range / func==0
    self.ppu.cia = (self.ppu.lr as u32) & !0x3;
    return Ok(None);
}
```

### 4.2 CheckCallback arm (drives call_guest_function)
```rust
0x189a74da => { // cellSysutilCheckCallback() -- no guest args
    // 1. Drain pending dispatches into an OWNED Vec (no borrow into import_plan held).
    let dispatches: Vec<PendingDispatch> =
        cell_sysutil_check_callback(&self.sysutil_callbacks, &mut self.sysutil_queue);
    // 2. For each, invoke the guest callback in FIFO (registration/enqueue) order.
    //    cellSysutil ABI: r3=event(status), r4=param, r5=userdata.
    for d in dispatches {
        // upstream increments read_counter BEFORE the call (termination-detection ordering)
        let _ = self.call_guest_function(d.cb.fn_addr, &[d.event as u64, d.param, d.cb.user_data as u64])?;
        // void callback -> ignore r3
    }
    self.ppu.gpr[3] = 0; // CELL_OK
    self.ppu.cia = (self.ppu.lr as u32) & !0x3;
    return Ok(None);
}
```

Notes:
- `dispatches` is OWNED before the first `call_guest_function`, so no live `import_plan`/`stub_meta` borrow crosses the `&mut self` call (section 2.4).
- Each callback fully completes (its `blr` -> sentinel) before the next runs; the whole batch finishes before CheckCallback returns to its caller.
- Drain order is FIFO (upstream `pop_all` is FILO-to-FIFO, lockless.h:402); the Rust `Vec<PendingDispatch>` must preserve enqueue order. Verify the library returns FIFO.
- The dispatcher table (Register/UnregisterCallbackDispatcher) is store-only in upstream — registered but NEVER invoked. Do NOT over-implement; just store/clear and return `CELL_OK` / `0x8002b004` (full) / `0x8002b005` (not found).

### 4.3 Engine enqueue path (host side)
`CheckCallback` only does work if something queued an event. Upstream `sysutil_send_system_cmd` (`cellSysutil.cpp:160`) pushes `{status,param,userdata}` to each registered slot. For the first fixture we need a deterministic way to enqueue exactly one event (e.g. push `(status=0x0101 REQUEST_EXITGAME, param=0)` into `sysutil_queue`). For the behavior-freeze fixture, the simplest hook is to seed the queue at a fixed point (e.g. right after boot, or via a tiny test-only `send_system_cmd` helper) so the run is deterministic.

Event/status constants (cellSysutil.h:136-153): `REQUEST_EXITGAME=0x0101`, `DRAWING_BEGIN=0x0121`, `DRAWING_END=0x0122`, `SYSTEM_MENU_OPEN=0x0131`, etc.

---

## 5. Behavior-freeze fixture

CC0 PSL1GHT homebrew `single_sysutil_callback_v1` (built via the existing PSL1GHT Docker toolchain), proving a guest callback ran end-to-end:

### 5.1 Guest program
```c
static volatile uint32_t g_observed = 0;     // sentinel cell in guest memory
static void my_cb(uint64_t status, uint64_t param, void* userdata) {
    g_observed = (uint32_t)status;            // callback writes what it saw
}
int main() {
    cellSysutilRegisterCallback(0, my_cb, (void*)0xABCD1234); // userdata marker
    // (host enqueues one event: status=0x0101)
    cellSysutilCheckCallback();                // drains -> invokes my_cb(0x0101, ...)
    if (g_observed == 0x0101) return 0x600D;   // success exit
    return 0xBAD0;                             // callback never ran / wrong value
}
```

### 5.2 Expected pre/post state
- PRE (before CheckCallback): `g_observed == 0`, queue holds 1 entry `(0x0101, 0)`, slot 0 = `{fn_addr=<my_cb FD>, user_data=0xABCD1234}`.
- DURING: `call_guest_function(my_cb_fd, [0x0101, 0, 0xABCD1234])` runs guest `my_cb`, which stores `0x0101` to `g_observed`. Memory side-effect persists in `self.mem`.
- POST: queue empty; `g_observed == 0x0101`; `CheckCallback` returns CELL_OK; `main` returns `0x600D`.

### 5.3 Oracle assertions (Rust test, e.g. `rsx_*`-style integration test in emu-core)
- `run_self` returns `ExitStatus` with exit code `0x600D` (NOT `0xBAD0`).
- After the run, read `g_observed` from `self.mem` == `0x0101`.
- The PPU register frame after CheckCallback returns is consistent (caller `cia`/`lr` restored — verified implicitly by main reaching its `return`).
- Negative control: with the CheckCallback arm stubbed to no-op, the fixture returns `0xBAD0` (proves the callback path, not luck, produced the success value).

This is the first oracle that exercises HLE -> guest -> HLE re-entry. It mirrors the 20 SPU oracles + the GCM oracles as a replay-validated CC0 fixture.

---

## 6. Risks and open questions

1. **Re-entrancy depth & determinism.** `dispatch_syscall` becomes re-entrant (HLE -> call_guest_function -> nested loop -> dispatch_syscall). If a callback itself calls `CheckCallback` (recursion) the depth is unbounded. First fixture must NOT recurse; add a depth guard later. Verify FIRST that a single non-recursive callback resumes the outer caller correctly.
2. **Sentinel collision.** `0xD0FF_0000` must never be a real mapped/executable address and must stay outside the import-stub window. If future memory maps grow into `0xD0FF_xxxx`, the sentinel breaks. Consider a dedicated reserved 1-page guard region documented centrally (analogous to `USER_IMPORT_STUB` at `lib.rs:301/324`).
3. **Register save/restore completeness.** A buggy/HLE-invoked callback may clobber callee-saved `r14-r31`, `cr`, `ctr`, `xer`, `fpr`, `vr`. Snapshot the ENTIRE arch register set; do NOT trust ABI callee-save. `PpuThread` is not `Clone`, so the manual snapshot struct must stay in sync with `PpuThread` fields.
4. **TOC / r2.** Confirmed unnecessary for PSL1GHT (compact 4-byte FD, `gpr[2]` never read, import stubs set `toc=0`). RISK: any non-PSL1GHT signed-PRX OPD is 8 bytes `{addr,rtoc}` and WOULD need `gpr[2]` loaded. Keep the primitive forward-compatible (optional `FD+4` read) but do not wire it now.
5. **Budget / infinite loop.** The nested loop needs its own bound (reuse `self.step_budget`) and a distinct error (`Error::CallbackStepsExhausted`) so a callback faulting to an unmapped page does not spin to `usize::MAX`. Restore the saved frame even on the error path.
6. **`permissive_unknown_syscalls`** is set true by `run_self` (`lib.rs:607`); the nested loop inherits it, so unknown syscalls inside a callback return `CELL_OK` instead of erroring. Acceptable but can MASK real callback failures — keep in mind during debugging.
7. **Slot-count divergence (4 vs 8).** Library `CB_SLOT_MAX=8` vs upstream 4. Decide: enforce upstream `slot >= 4 -> 0x8002b102` in the arm, or change the library constant. Affects byte-exact freeze.
8. **NID confirmation.** The five NIDs are computed, not yet runtime-verified for THIS fixture. Capture the fixture's libstub and confirm before locking the arms (two siblings `0x40e895d3`/`0x938013a0` already match, raising confidence).
9. **`read_le` trap.** FD deref MUST use raw `read` + `u32::from_be_bytes`. A `read_le` slip silently byte-swaps the code address and jumps to garbage.
10. **Enqueue determinism.** The fixture needs a deterministic single-event enqueue. Decide whether to expose a test-only `send_system_cmd` or seed `sysutil_queue` directly post-boot.

What to verify FIRST: a single, non-recursive, void callback that writes one word to guest memory and returns via `blr` -> the outer CheckCallback resumes and main reaches its success return. Everything else (recursion, multi-slot broadcast, non-void returns) is incremental on top of that.

---

## 7. Build order (minimal first slice)

Smallest thing that proves a guest callback ran end-to-end:

**Slice 1 — the primitive in isolation (synthetic, no fixture).**
1. Add `PpuRegSnapshot` + `EmuCore::call_guest_function` (section 2). Add `Error::CallbackStepsExhausted`.
2. Add `resolve_code_addr` helper (extract from `load_elf` `lib.rs:564-578`).
3. Unit test: hand-assemble a tiny guest function in `self.mem` that does `addi r3, r3, 1; blr` (or `mr r3,r4; blr`), point an FD at it, call `call_guest_function(fd, &[41])`, assert return `== 42` AND that `cia`/`lr`/`gpr` are restored to their pre-call values. This proves seed -> run -> sentinel-stop -> capture -> restore with NO HLE wiring.

**Slice 2 — wire the cellSysutil state + RegisterCallback.**
4. Add `sysutil_callbacks` + `sysutil_queue` fields + `new()` init + import (section 3).
5. Add the RegisterCallback / UnregisterCallback NID arms (pure table mutation). Unit test register -> get -> unregister via the library.

**Slice 3 — CheckCallback drives the primitive (synthetic).**
6. Add the CheckCallback NID arm (section 4.2). Add a test-only enqueue helper.
7. Integration test (still synthetic guest cb in mem): register slot 0 -> enqueue `(0x0101,0)` -> CheckCallback -> assert the synthetic cb wrote `0x0101` to a known guest cell and the queue is drained.

**Slice 4 — the real CC0 fixture (the oracle).**
8. Build `single_sysutil_callback_v1` (section 5) via PSL1GHT Docker. Capture its libstub, CONFIRM the 5 NIDs.
9. `run_self` it; assert exit `0x600D`, `g_observed == 0x0101`, negative control returns `0xBAD0`. This is the replay-validated end-to-end oracle.

Slice 1 alone proves the core mechanism (the genuinely new control flow); slices 2-3 are mostly wiring of existing library code; slice 4 is the behavior-freeze proof against real PSL1GHT bytes.
