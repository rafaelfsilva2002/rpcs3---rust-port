# single_cond_v1

Fourth PPU-only fixture in the behavior-freeze set (after mutex,
sema, event_queue). Exercises the single-PPU-reachable subset of
the LV2 `sys_cond_*` syscall family via PSL1GHT's `<sys/cond.h>`:

```
mutexCreate → condCreate(cond, mutex) → condSignal (empty)
  → condBroadcast (empty) → condDestroy → mutexDestroy
```

Returns `0xC0DE` on success; step codes 1..=6 on failure.

## Why no sys_cond_wait

`sysCondWait` (#107) atomically releases the mutex and parks the
caller until another thread signals. On the single-PPU EmuCore
there is no second thread, so a wait would block forever. This
fixture validates the create / signal / broadcast / destroy arms
only; the wait/reacquire handshake is unit-tested in
`rpcs3-lv2-sync::state::tests` (R10.3) instead.

## Syscalls covered

| PSL1GHT call | LV2 syscall | Rust handler |
|---|---|---|
| `sysMutexCreate` / `sysMutexDestroy` | `#100` / `#101` | reused from R10.1.d |
| `sysCondCreate(*cond, mutex, *attr)` | `#105` | `EmuCore::dispatch_syscall` (R10.1.g) → `Lv2SyncState::cond_create` (R10.3 `CondRegistry`) |
| `sysCondDestroy(cond)`               | `#106` | R10.1.g → `cond_destroy` |
| `sysCondWait(cond, timeout)`         | `#107` | wired (MustBlock → ETIMEDOUT) but not exercised by this fixture |
| `sysCondSignal(cond)`                | `#108` | R10.1.g → `cond_signal` (None on empty → CELL_OK) |
| `sysCondBroadcast(cond)`             | `#109` | R10.1.g → `cond_signal_all` |

`sys_cond_attr_t` (24 bytes BE: pshared + flags + key + name[8]) is
parsed by the create arm; protocol comes from the bound mutex, not
the cond attr.

## Build

```
subst R: "C:\Users\manod\Downloads\Emulador Ps2, ps1 e ps3 nativos\rpcs3-master\behavior-freeze\fixtures\lv2\sources"

docker run --rm -v "R:\single_cond_v1:/work" -w /work \
    rpcs3-ps3dev-toolchain:local \
    bash -lc "make"
```

Output: `single_cond_v1.self` (gitignored).

## Validation

[`rust/rpcs3-emu-core/tests/run_self_cond_smoke.rs`](../../../../../rust/rpcs3-emu-core/tests/run_self_cond_smoke.rs)
asserts exit status `0xC0DE`; skips when `.self` absent.

## CC0

CC0 1.0 Universal — see `LICENSE.md`.
