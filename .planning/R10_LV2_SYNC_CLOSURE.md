# R10 — LV2 sync primitives (CLOSED, library layer architecturally complete)

**Date:** 2026-05-26.
**Wave:** R10 (LV2 sync primitives — `rpcs3-lv2-sync::Lv2SyncState`).
**Status:** Library layer architecturally complete. 6 of 8 planned
trait impls landed. R10.8 (`sys_lwcond`) deferred — no
`rpcs3-lv2-lwcond` crate exists in workspace, only HLE-user-layer
port. Fixture / oracle work remains blocked by Docker capture
pipeline offline.
**Predecessor:** R9 closure `5b51b7b46`.
**Closure commit:** see end of this file.

## Goal recap

Port the LV2 sync primitive family from RPCS3 C++ to Rust with the
behavior-freeze + captured-oracle pattern, starting with the
library layer:

1. Replace R9.1h's no-op lwmutex stubs with real state machines. ✓
2. Implement the full kernel sync family (mutex, sema, cond, event
   flag, event queue, event port, rwlock). 6/7 ✓; lwcond deferred.
3. Capture CC0 fixtures per primitive. **BLOCKED** (Docker offline).
4. Promote each fixture to a replay-validated oracle. **BLOCKED**.
5. Maintain 264 cargo release test result blocks, zero regression,
   20 SPU oracles green throughout. ✓

## What R10 delivered

### Architectural

| Slice | Commit | Scope |
|---|---|---|
| R10.1.a | `167ebe8f5` | Handle pool — `Lv2SyncId`, `Lv2SyncKind`, `Lv2SyncState`, `LwMutex` container |
| R10.1.b | `99cfeb517` | `impl LwMutexTable for Lv2SyncState`; 3 PSL1GHT NIDs wired in `EmuCore` (real `lwmutex_create/lock/unlock`) |
| R10.1.c | `97b016fb0` | `LwMutexAttribute` typed parser in `rpcs3-lv2-lwmutex`; `EmuCore::read_lwmutex_attr` returns typed value; `Error::SyscallEinval` variant |
| R10.2+.4 | `3d0a06f3a` | `impl SyncTable for Lv2SyncState` (kernel `sys_mutex` + `sys_semaphore`); `Mutex` + `Sema` containers |
| R10.3 | `031490d81` | `impl CondRegistry for Lv2SyncState` (`sys_cond` family with mutex-tied atomic release/reacquire) |
| R10.5 | `3c392f800` | `impl EventFlagRegistry for Lv2SyncState` (`sys_event_flag` family with AND/OR + CLEAR/CLEAR_ALL modes) |
| R10.7 | `ab1cb4e8f` | `impl RwlockRegistry for Lv2SyncState` (writer-priority `sys_rwlock` family) |
| R10.6 | `204ca0311` | `impl EventRegistry for Lv2SyncState` (`sys_event_queue` + `sys_event_port` + connect/send/receive/drain) |
| R10.8 | DEFERRED | `sys_lwcond` family — no `rpcs3-lv2-lwcond` crate in workspace, would require new crate + trait scaffolding |

### Type model

`rust/rpcs3-lv2-sync/src/state.rs` is the central registry:

```text
Lv2SyncState                 — per-EmuCore owned registry
├── BTreeMap<u32, Entry>     — deterministic iteration
├── id_counter (monotonic, from 1, never reused)
└── Entry enum
    ├── LwMutex(LwMutex)     — userspace control word in
    │                          rpcs3-lv2-lwmutex::LwMutexControl;
    │                          this side is sleep queue + recursion
    ├── Mutex(Mutex)         — kernel-side state (owner, recursion,
    │                          waiters)
    ├── Sema(Sema)           — counting semaphore
    ├── Cond(Cond)           — cv bound to a mutex (mutex_id_untagged)
    ├── EventFlag(EventFlag) — 64-bit bitmask + waiter queue
    ├── EventQueue(EventQueue) — bounded FIFO of Event tuples
    ├── EventPort(EventPort) — connects to one queue
    ├── RwLock(Rwlock)       — writer-priority RW lock
    └── LwCond               — RESERVED, R10.8 deferred
```

ID tagging (RPCS3 C++ `lv2_*::id_base` parity, applied at the
trait-impl boundary so guest code sees the same id space the C++
emulator would have emitted):

| Primitive | High-byte tag |
|---|---|
| LwMutex | `0x95000000` |
| Mutex | `0x85000000` |
| Sema | `0x96000000` |
| Cond | `0x86000000` |
| EventFlag | `0x98000000` |
| RwLock | `0x88000000` |
| EventQueue | `0x8D000000` |
| EventPort | `0x0E000000` |

### Cross-crate deps added

`rpcs3-lv2-sync` now depends on (path):
- `rpcs3-lv2-lwmutex` (R10.1.b)
- `rpcs3-lv2-cond` (R10.3)
- `rpcs3-lv2-event-flag` (R10.5)
- `rpcs3-lv2-rwlock` (R10.7)
- `rpcs3-lv2-event` (R10.6)

No cycles introduced — each per-primitive crate only depends on
`rpcs3-emu-types`.

`rpcs3-emu-core` gained deps on `rpcs3-lv2-sync` + `rpcs3-lv2-lwmutex`
(R10.1.b).

### EmuCore changes

- `EmuCore.lv2_sync_state: Lv2SyncState` — new owned field, reset
  per `EmuCore::new`.
- `EmuCore::read_lwmutex_attr` — typed parser using
  `LwMutexAttribute::parse`.
- `EmuCore::read_lwmutex_control` / `write_lwmutex_control` — 32-byte
  BE round-trip helpers.
- `Error::SyscallEinval(CellError)` — new variant for typed-parse
  EINVAL propagation.
- Three PSL1GHT NID handlers (`0x2f85c0ef`, `0x1573dc3f`,
  `0x1bc200f4`) replace R9.1h no-op stubs with real
  handle-pool-backed implementations.

### Single-PPU caveat

The blocking outcomes (`MustBlock` from `mutex_lock`,
`cond_wait`, `evflag_wait`, `rwlock_*lock`, `queue_receive`) are
reported by the wrappers but `EmuCore` can't actually park a
thread — there's only one PPU. PSL1GHT crt0 is single-threaded
so this is currently a theoretical gap; the arms surface MustBlock
honestly so future PPU-SMT or PPU↔SPU contention work (R11+) can
wire parking.

### Tests added during R10

`rpcs3-lv2-sync --lib` went from 24 tests (R9 baseline:
mutex+sema TestTable) to **109 tests**:

| Slice | Tests added | Cumulative |
|---|---|---|
| R10.1.a | +13 | 37 |
| R10.1.b | +6 (LwMutexTable) | 43 |
| R10.1.c | +6 in `rpcs3-lv2-lwmutex` (attr parser) + 4 in `rpcs3-emu-core` | 43 / 18 |
| R10.2+.4 | +16 (SyncTable for Lv2SyncState) | 61 |
| R10.3 | +12 (CondRegistry) | 73 |
| R10.5 | +12 (EventFlagRegistry) | 85 |
| R10.7 | +13 (RwlockRegistry) | 98 |
| R10.6 | +11 (EventRegistry) | 109 |

Plus 4 R9.1m/n diagnostic test extensions in
`rpcs3-emu-core::tests` (folded into R9 closure).

### Validation

- `cargo test -p rpcs3-lv2-sync --lib`: **109 passed, 0 failed**.
- `cargo test -p rpcs3-emu-core --lib`: **20 passed, 0 failed**
  (last verified after R10.1.c).
- `cargo test --workspace --tests --release` (canonical gate):
  **264 result blocks, 0 failed** (verified after R10.1.b,
  R10.2+.4, R10.6). R9 parity preserved throughout.
- 20 SPU oracle replay tests stayed green every commit.
- behavior-freeze contract untouched (no fixture / JSONL /
  capture-writer / C++ changes).

## What R10 did NOT deliver

### R10.8 (sys_lwcond) — DEFERRED

No `rpcs3-lv2-lwcond` crate exists in the workspace. The closest
artifact, `rpcs3-hle-sys-lwcond-user`, is a port of the **HLE
user-side module** (`rpcs3/Emu/Cell/Modules/sys_lwcond_.cpp`)
that orchestrates `LwMutexControl` + raw syscalls — it does not
expose a kernel-side trait analogous to `LwMutexTable` /
`CondRegistry`.

Implementing R10.8 would require:
1. Creating a new `rpcs3-lv2-lwcond` crate with:
   - `LwCondAttr` struct.
   - `LwCondRegistry` trait (create/destroy/signal/signal_all/
     signal_to/wait_prepare/wait_finish).
   - `LwCondControl` userspace BE struct (8 bytes per PSL1GHT
     `sys_lwcond_t`).
2. Adding `Entry::LwCond(LwCond)` variant.
3. Implementing the trait on `Lv2SyncState`.

Bounded scope (~400-600 LOC across the new crate + state.rs),
but standalone, and PSL1GHT crt0 doesn't currently call lwcond,
so leverage from finishing it now is low. Pull it into R11+ or
a dedicated newlib/threading wave when an oracle fixture
actually exercises it.

### Fixtures / oracles / capture (R10.1.d/.e + per-family)

All capture-side work for R10 (CC0 fixtures, JSONL events,
oracle replay) remains **BLOCKED**: Docker capture pipeline
offline per the earlier memory note. Library-layer impls in this
wave are validated by behavioural unit tests only; once Docker
is back, the existing PSL1GHT toolchain + harness can capture
per-primitive fixtures and promote them to oracles.

### EmuCore NID/syscall wiring

Only the 3 lwmutex NIDs got wired into the dispatcher (R10.1.b).
The other 5 primitives' syscall arms (mutex, sema, cond, event
flag, event queue, event port, rwlock) are **not wired** in
`EmuCore::dispatch_syscall` because PSL1GHT crt0 doesn't call
them. The Lv2SyncState impls are exercised by unit tests only
in this wave. Wiring them lights up the moment a fixture
needs them.

## Strategic next directions

### Option A — Pivot to fixture/capture pipeline (recommended once Docker is back)

The library layer is solid; the next yield comes from CC0
fixtures + replay oracles that exercise each primitive. Order
of leverage:
1. lwmutex non-contended (smallest, builds on existing R6.5
   PSL1GHT toolchain).
2. mutex + sema + cond non-contended.
3. event flag.
4. event queue/port.
5. PPU↔SPU contention oracles (R10.9 scope from
   `R10_LV2_SYNC_PLAN.md`).

### Option B — Continue library layer with R10.8 lwcond

Bounded but standalone (no PSL1GHT call site exercising it). Best
done lazily when a fixture surfaces the need.

### Option C — Wire syscall arms into EmuCore preemptively

Add the 30+ `sys_mutex_*` / `sys_cond_*` / `sys_semaphore_*` etc
arms in `EmuCore::dispatch_syscall` so any future binary can
exercise them without a per-primitive dispatcher edit. ~200 LOC
of plumbing, no behaviour change for current oracles.

## Closure metadata

- Library layer commits: 8 (`167ebe8f5`, `99cfeb517`, `97b016fb0`,
  `3d0a06f3a`, `031490d81`, `3c392f800`, `ab1cb4e8f`, `204ca0311`).
- Closure commit (this doc + sentinel removal): see end of git log.
- 20 SPU oracles green throughout.
- 264 release blocks, 0 failed throughout.
- behavior-freeze contract preserved.
