# single_lwmutex_v1

First PPU-only fixture in the behavior-freeze set. Exercises the LV2
lwmutex syscall family end-to-end via the PSL1GHT user library:

```
sysLwMutexCreate → sysLwMutexLock → sysLwMutexUnlock → sysLwMutexDestroy
```

The binary returns `0xC0DE` on full round-trip success; step-specific
failure codes (1..=4) preserve which syscall faulted.

## NIDs covered

| PSL1GHT call | NID | Rust handler |
|---|---|---|
| `sysLwMutexCreate` | `0x2f85c0ef` | `rpcs3-emu-core::EmuCore` R10.1.b — `_sys_lwmutex_create` (Lv2SyncState → LwMutex entry + 32-byte control struct round-trip) |
| `sysLwMutexLock` | `0x1573dc3f` | R10.1.b — `_sys_lwmutex_lock` (single-PPU model; MustBlock surfaces as EBUSY honestly) |
| `sysLwMutexUnlock` | `0x1bc200f4` | R10.1.b — `_sys_lwmutex_unlock` |
| `sysLwMutexDestroy` | `0xc3476d0c` | permissive catch-all (Lv2SyncState entry retained until process exit; safe for this single-shot fixture) |

## Build

The PSL1GHT toolchain ships in the project's Docker container
(`.claude/ps3toolchain-docker/`, image `rpcs3-ps3dev-toolchain:local`,
2.43 GB). Per `CLAUDE.md`, use `subst R:` on Windows to shorten the
path before invoking docker.

```
# from rpcs3-master/ root
subst R: "%CD%\behavior-freeze\fixtures\lv2\sources"
docker run --rm -v R:\single_lwmutex_v1:/work -w /work \
    rpcs3-ps3dev-toolchain:local \
    bash -lc "source /etc/profile.d/ps3dev.sh && make"
```

Output: `single_lwmutex_v1.self` in this directory (gitignored —
`.self` extension blocked by the path-lock hook).

## Validation

The Rust smoke test
[`rust/rpcs3-emu-core/tests/run_self_lwmutex_smoke.rs`](../../../../../rust/rpcs3-emu-core/tests/run_self_lwmutex_smoke.rs)
loads the `.self` (when present) and asserts the exit status equals
`0xC0DE`. Skips gracefully when the fixture binary is absent so CI on
machines without the Docker toolchain still passes.

## Why no JSONL trace

The capture-writer extension to emit lwmutex events is out of R10
scope (no C++ changes per the wave's authorship constraints). This
fixture is a **smoke test against R10.1.b's NID wiring**, not a
byte-exact replay oracle in the R5-R8 sense. A future slice can add
the lwmutex-event class to the capture schema and promote this
fixture to a full replay-validated oracle.

## CC0

CC0 1.0 Universal — see `LICENSE.md`.
