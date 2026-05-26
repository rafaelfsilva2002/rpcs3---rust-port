# R10 fixture pipeline — CLOSED (all PSL1GHT-exposed sync families covered)

**Date:** 2026-05-26.
**Context:** Post-R10-closure extra slices (Option A from
`R10_LV2_SYNC_CLOSURE.md`): build CC0 fixtures + end-to-end
smokes that drive the R10 library-layer trait impls through real
PSL1GHT-compiled binaries via `EmuCore::run_self`.

## What landed

Four PPU-only CC0 fixtures under
`behavior-freeze/fixtures/lv2/sources/`, each compiled to a
signed `.self` via the ps3toolchain Docker image and validated by
a Rust smoke test asserting `exit_status == 0xC0DE`:

| Slice | Commit | Fixture | Family | Syscall arms wired |
|---|---|---|---|---|
| R10.1.d | `aec8a92b5` | `single_mutex_v1` | sys_mutex | #100-#104 |
| R10.1.e | `cc820a092` | `single_sema_v1` | sys_semaphore | #90-#94, #114 |
| R10.1.f | `4866ed489` | `single_event_queue_v1` | sys_event_queue + port | #128-#130, #133-#138 |
| R10.1.g | `5329c5919` | `single_cond_v1` | sys_cond | #105-#109 |

All four exercise the create → operate → destroy round-trip
through real PSL1GHT `crt0` → `main()` → the new dispatcher arms
→ `Lv2SyncState` trait impls → `sys_process_exit` returning the
canonical `0xC0DE` sentinel.

## Dispatcher additions (rpcs3-emu-core/src/lib.rs)

- `PPU_THREAD_TID: u32 = 1` — synthetic tid for all sync calls
  (single-PPU model; becomes per-PpuThread in R11+).
- 4 BE attr parsers: `read_sys_mutex_attr` (40 B),
  `read_sys_sem_attr` (32 B), `read_sys_event_queue_attr` (16 B),
  `read_sys_cond_attr` (24 B).
- 24 new syscall arms total (5 mutex + 6 sema + 8 event + 5 cond).
- New deps: rpcs3-lv2-event, rpcs3-lv2-cond
  (rpcs3-lv2-sync + lwmutex were already in from R10.1.b).

## Key ABI discoveries

1. **PSL1GHT doesn't expose lwmutex / event_flag / rwlock / lwcond
   to homebrew.** Only mutex/sem/cond/event_queue have `<sys/*.h>`
   headers. The original R10.1.d lwmutex framing was wrong;
   pivoted to kernel sys_mutex. lwmutex is exercised implicitly by
   every PSL1GHT crt0 (so R9 mailbox_v1.self is its de-facto
   oracle). event_flag/rwlock/lwcond stay unit-test-only
   (validated by the 109 lv2-sync tests).
2. **sys_event_queue_receive returns the event in REGISTERS
   r4-r7, not via the event pointer.** PSL1GHT's
   `REG_PASS_SYS_EVENT_QUEUE_RECEIVE` macro (ppu-lv2.h) copies
   r4→source, r5→data_1, r6→data_2, r7→data_3. First cut wrote to
   the struct pointer and failed (exit 6); fixed to set registers.
3. **sys_semaphore_get_value lives at #114**, outside the 90-95
   band (PSL1GHT puts it after the rwlock family).

## Single-PPU limits (honest)

- Every blocking outcome (mutex_lock contended, cond_wait,
  sema_wait at 0, queue_receive empty) surfaces as EDEADLK or
  ETIMEDOUT because the EmuCore can't actually park a thread.
  All four fixtures are deliberately non-contended so they never
  hit those paths.
- `sys_cond_wait` (#107) is wired but unexercisable single-PPU
  (no second thread to signal). The wait/reacquire handshake is
  unit-tested in rpcs3-lv2-sync::state::tests (R10.3).

## Validation

- Workspace canonical gate grew 264 → **268 result blocks**
  (+1 per fixture smoke), **0 failed** throughout.
- 109 lv2-sync unit tests, 20 SPU oracles, all R9 tests stay
  green.
- behavior-freeze contract preserved: no C++ / capture-writer /
  JSONL changes. Fixtures are smoke tests, NOT byte-exact replay
  oracles.

## .gitignore fix (R10.1.d side-effect)

Added `!behavior-freeze/fixtures/**/Makefile` negation — a global
`Makefile` rule (for CMake artifacts) had been silently ignoring
all fixture Makefiles since the SPU fixtures landed. The fix
brought **20** previously-untracked SPU fixture Makefiles into
git (corrected from an earlier "17" miscount) + tracks the 4 new
lv2 ones.

**Audit (post-hoc, 2026-05-26):** all 20 SPU Makefiles were
verified as legitimate, hand-written build recipes (55-88 lines
each, one per fixture, each paired with its `main.c` — they
reproduce each oracle's `.self`). Tracking them was always the
intent: the comment at `.gitignore` line 58 explicitly states
"CC0 sources + Makefile + capture script are tracked". They are
NOT CMake-generated junk. The only incidental part is the
*bundling*: all 20 rode into the R10.1.d (`aec8a92b5`) sys_mutex
commit as a side-effect of the negation rule rather than getting
a standalone commit, and that commit's message mislabeled them as
"untracked-pending / a follow-up commit can bring them into git"
when they were in fact committed in the same change. No content
issue — the files are correct and useful; only the commit
hygiene + the message wording were imperfect.

## What's NOT done (deferred)

- **JSONL capture writer + byte-exact replay oracle promotion.**
  Needs C++ changes (capture writer extension to emit sync-
  primitive events), blocked by the R10 wave's no-C++ constraint.
  The smokes catch behaviour regressions but not byte-exact state.
- **event_flag / rwlock / lwcond / lwmutex fixtures.** Not
  exposed by PSL1GHT; would need hand-written syscall stubs
  rather than the libsysutil path. Low value vs the unit tests
  already covering them.
- **Contended / multi-thread paths.** Need PPU SMT or PPU↔SPU
  contention (R11+ scope per R10_LV2_SYNC_PLAN.md Phase F).

## Build recovery (Windows, after reboot)

```powershell
# Docker Desktop must be running (Start Menu → Docker Desktop).
subst R: "C:\Users\manod\Downloads\Emulador Ps2, ps1 e ps3 nativos\rpcs3-master\behavior-freeze\fixtures\lv2\sources"
docker run --rm -v "R:\<fixture>:/work" -w /work rpcs3-ps3dev-toolchain:local bash -lc "make"
```

Image uses `ENV` for PS3DEV/PSL1GHT (`/opt/ps3dev`); no
profile.d sourcing needed.
