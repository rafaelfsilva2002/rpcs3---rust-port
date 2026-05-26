# single_mutex_v1

First PPU-only fixture in the behavior-freeze set. Exercises the
LV2 **kernel sys_mutex** syscall family end-to-end via the
PSL1GHT user library:

```
sysMutexCreate → sysMutexLock → sysMutexUnlock →
sysMutexTryLock → sysMutexUnlock → sysMutexDestroy
```

The binary returns `0xC0DE` on full round-trip success;
step-specific failure codes (1..=6) preserve which syscall faulted.

## Why kernel mutex (not lwmutex)?

PSL1GHT's `<sys/lwmutex.h>` is NOT exposed to homebrew authors —
lwmutex is internal to PSL1GHT's `crt0` and `libsysutil`. Every
PSL1GHT binary's startup already exercises lwmutex via crt0
(including R9's `single_spu_mailbox_v1.self`), so the R10.1.b
lwmutex wiring is implicitly validated by the existing R9 smoke.
This fixture targets the next layer — kernel `sys_mutex_*` —
which IS exposed to homebrew via `<sys/mutex.h>`.

## Syscalls covered

| PSL1GHT call | LV2 syscall | Rust handler |
|---|---|---|
| `sysMutexCreate(*mutex, *attr)` | `#100` | `EmuCore::dispatch_syscall` (R10.1.d) → `Lv2SyncState::mutex_create` (R10.2 `SyncTable` impl) |
| `sysMutexDestroy(mutex)` | `#101` | R10.1.d → `mutex_destroy` |
| `sysMutexLock(mutex, timeout)` | `#102` | R10.1.d → `mutex_lock` (single-PPU model; contention surfaces honestly) |
| `sysMutexTryLock(mutex)` | `#103` | R10.1.d → `mutex_trylock` |
| `sysMutexUnlock(mutex)` | `#104` | R10.1.d → `mutex_unlock` |

`sys_mutex_attr_t` (40 bytes BE: protocol/recursive/pshared/adaptive
+ key + flags + pad + name[8]) is parsed by the dispatcher arm and
mapped to `rpcs3-lv2-sync::MutexAttr`.

## Build

The PSL1GHT toolchain ships in the project's Docker container
(`.claude/ps3toolchain-docker/`, image
`rpcs3-ps3dev-toolchain:local`, 2.43 GB). Per `CLAUDE.md`, use
`subst R:` on Windows to shorten the path before invoking docker.

```
# from any PowerShell session after reboot
subst R: "C:\Users\manod\Downloads\Emulador Ps2, ps1 e ps3 nativos\rpcs3-master\behavior-freeze\fixtures\lv2\sources"

docker run --rm -v "R:\single_mutex_v1:/work" -w /work \
    rpcs3-ps3dev-toolchain:local \
    bash -lc "make"
```

Output: `single_mutex_v1.self` in this directory (gitignored —
`.self` extension blocked by the path-lock hook).

## Validation

The Rust smoke test
[`rust/rpcs3-emu-core/tests/run_self_mutex_smoke.rs`](../../../../../rust/rpcs3-emu-core/tests/run_self_mutex_smoke.rs)
loads the `.self` (when present) and asserts the exit status
equals `0xC0DE`. Skips gracefully when the fixture binary is
absent so CI on machines without the Docker toolchain still
passes.

## Why no JSONL trace

The capture-writer extension to emit `sys_mutex_*` events is
out of R10 scope (no C++ changes per the wave's authorship
constraints). This fixture is a **smoke test against the R10.2
SyncTable impl + R10.1.d syscall arms**, not a byte-exact
replay oracle in the R5-R8 sense. A future slice can add the
sync-primitive event class to the capture schema and promote
this fixture to a full replay-validated oracle.

## CC0

CC0 1.0 Universal — see `LICENSE.md`.
