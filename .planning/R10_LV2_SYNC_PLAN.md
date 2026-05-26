# R10 — LV2 sync primitives (plan)

**Status:** PLAN (not started).
**Date:** 2026-05-25.
**Predecessor:** R9 closure `5b51b7b46` (LV2/PPU integration
architecturally complete).
**Strategic context:** Option A selected — pivot away from
PSL1GHT TTY emit toward new RPCS3 subsystems. LV2 sync is the
natural next layer on top of R9's syscall dispatcher.

## Goal

Port the LV2 sync primitive family from RPCS3 C++ to Rust with
the behavior-freeze + captured-oracle pattern:

1. Replace R9's no-op lwmutex stubs (`lib.rs:982-996`) with
   real state machines.
2. Implement the full kernel sync family: sys_mutex, sys_cond,
   sys_semaphore, sys_event_flag, sys_event_queue/port,
   sys_lwmutex, sys_lwcond, sys_rwlock.
3. Capture CC0 fixtures exercising each primitive deterministically.
4. Promote each fixture to a replay-validated oracle.
5. Maintain 264 cargo test result blocks, 0 failures and 20
   existing SPU oracles green throughout.

## Current state (post-R9)

| Primitive | R9 state | What's missing |
|---|---|---|
| `sys_lwmutex_create` (NID `0x2f85c0ef`) | no-op stub (zeros 8 bytes) | real handle, attr parse, state |
| `sys_lwmutex_lock` (NID `0x1573dc3f`) | no-op stub (returns 0) | ownership state, recursion, contention |
| `sys_lwmutex_unlock` (NID `0x1bc200f4`) | no-op stub | ownership check, waiter wake |
| `sys_lwmutex_trylock` | not implemented | full implementation |
| `sys_lwcond_*` | not implemented | full family |
| `sys_mutex_*` (syscalls 100-105) | not implemented | full family |
| `sys_cond_*` (syscalls 106-111) | not implemented | full family |
| `sys_semaphore_*` (syscalls 90-95) | not implemented | full family |
| `sys_event_flag_*` (syscalls 82-89) | not implemented | full family |
| `sys_event_queue_*` (syscalls 128-138) | not implemented | full family |
| `sys_rwlock_*` (syscalls 112-117) | not implemented | full family |

Total LV2 syscall arms to add: ~50. NID handlers (lwmutex /
lwcond go through sysPrxForUser PRX imports): ~20.

## Threading model — capture-time vs replay-time

PS3 sync primitives are inherently multi-threaded. R10 needs
testable concurrency without requiring full PPU SMT.

**Three viable test surfaces:**

1. **Single-thread state coverage** — non-contended ops
   (create/lock-immediately-unlock/destroy) on a single PPU
   thread. Tests state machine correctness only, no waiter
   queueing. ~80% of slice coverage.
2. **PPU↔SPU contention** — SPU thread (already running real
   via R9.1g.10 spu-interpreter) calls lv2 syscalls via the
   sys_spu_thread_send_event channel. Two-thread contention
   exercisable without PPU SMT. Tests waiter queueing +
   wake-up ordering.
3. **PPU↔PPU multi-thread** — requires running multiple PPU
   contexts. Out of R10 scope; defer to R11+ if needed.

R10 targets (1) for non-contention slices and (2) for the
single contention slice per primitive.

## Slice decomposition

### Phase A — lwmutex (PSL1GHT-facing)

PSL1GHT already calls these from crt0 (see R9.1h handoff doc).
Replacing the no-op stubs with real state lets us:
- Detect double-unlock bugs in PSL1GHT runtime,
- Surface waiter queueing,
- Build the LV2 sync handle-pool infrastructure that the rest
  of R10 reuses.

| Slice | Scope |
|---|---|
| R10.1.a | Sync handle pool design + `LV2SyncId` allocator in `rpcs3-lv2-sync` (new crate) |
| R10.1.b | `LwMutex` state machine (owner, recursion count, waiters Vec) + `sys_lwmutex_lock/unlock/trylock` arms |
| R10.1.c | `LwMutexAttr` parser (recursive / non-recursive, protocol fifo/prio) |
| R10.1.d | CC0 fixture `single_thread_lwmutex_v1` — non-contended lock+unlock+destroy roundtrip |
| R10.1.e | Capture pipeline + replay test — first LV2 sync oracle |

### Phase B — kernel mutex

| Slice | Scope |
|---|---|
| R10.2.a | `sys_mutex_create/destroy` syscalls + state machine (heavier than lwmutex: kernel-side handle, protocol attr) |
| R10.2.b | `sys_mutex_lock/unlock/trylock` |
| R10.2.c | CC0 fixture + replay |

### Phase C — cond + semaphore (sit on top of mutex)

| Slice | Scope |
|---|---|
| R10.3 | `sys_cond_*` family (depends on R10.2 mutex) |
| R10.4 | `sys_semaphore_*` family |

### Phase D — event flag + queue/port

| Slice | Scope |
|---|---|
| R10.5 | `sys_event_flag_*` (8 syscalls) |
| R10.6 | `sys_event_queue_*` + `sys_event_port_*` (event-driven, most complex) |

### Phase E — rwlock + lwcond (round-out)

| Slice | Scope |
|---|---|
| R10.7 | `sys_rwlock_*` |
| R10.8 | `sys_lwcond_*` |

### Phase F — PPU↔SPU contention

| Slice | Scope |
|---|---|
| R10.9 | Per-family contention oracle (one per primitive) — SPU thread takes the lock the PPU is waiting on |

## Open questions blocking R10.1

1. **New crate or extend existing?** Should the sync state
   live in a new `rpcs3-lv2-sync` crate, or extend
   `rpcs3-emu-core` directly? RPCS3 C++ uses
   `rpcs3/Emu/Cell/lv2/sys_*.cpp` (one file per family). A
   new Rust crate per the existing pattern (`rpcs3-lv2-spu-image`,
   `rpcs3-lv2-spu-thread`) is most consistent.
2. **Handle ID space.** RPCS3 uses 32-bit IDs with high-byte
   type tags. Match exactly (for capture-trace byte
   compatibility) or use our own?
3. **Capture writer extension.** behavior-freeze writer needs
   to emit sync-primitive ops as a new JSONL event class.
   What's the event schema?
4. **First fixture target.** Start with lwmutex (R10.1) or
   kernel mutex (R10.2)? lwmutex is simpler (PSL1GHT already
   uses it; we already have no-op stubs to replace) BUT it's
   userspace-cooperative so the real test surface is
   smaller. Kernel mutex is more representative but bigger
   first-slice cost.
5. **Replay isolation.** SPU oracle replay tests don't
   exercise sync primitives. Does the LV2 sync state need to
   be threaded through `EmuCore` differently to avoid
   cross-test contamination?

## Validation gates (per slice)

Same as R8/R9:
- `cargo test --workspace --tests --release`: ≥264 result
  blocks, 0 failures (additive: new tests bump the count,
  never reduce it).
- 20 existing SPU oracle replay tests stay green.
- behavior-freeze contract preserved (no fixture edits, no
  fake JSONL, no fake syscall returns for known syscalls).
- New fixture's `.self` + `.jsonl` + `.spuimg` follow the
  pool-dedup-by-hash convention.

## Estimated scope

- **R10.1 (lwmutex)** — ~400-700 LOC across `rpcs3-lv2-sync`
  (new crate) + `rpcs3-emu-core` dispatcher edits + new
  test. ~3-5 commits if smooth.
- **R10 total** — ~2500-4000 LOC across 9 phases. Comparable
  to R8 (DMA family) + R9 combined.
- **Calendar** — depends on cadence; R8 ran ~6 days, R9
  ~5 days at autonomous pace.

## Out of scope

- PPU SMT (multiple PPU threads). Single PPU + SPU
  contention is the multi-thread surface for R10.
- Real-time priority scheduling. We use FIFO ordering for
  waiters in all primitives (matches RPCS3's
  SYS_SYNC_FIFO protocol default).
- Cancellation / cleanup semantics on
  sys_process_exit. Handled best-effort; full audit
  deferred unless a fixture surfaces a bug.

## Sign-off

R10 plan written 2026-05-25 post-R9 closure. Awaiting user
confirmation to start R10.1.a (handle pool + new crate
scaffolding).
