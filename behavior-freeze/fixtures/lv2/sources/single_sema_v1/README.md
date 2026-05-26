# single_sema_v1

Second PPU-only fixture in the behavior-freeze set (after
`single_mutex_v1`). Exercises the LV2 kernel `sys_semaphore_*`
syscall family end-to-end via PSL1GHT's `<sys/sem.h>`:

```
sysSemCreate(initial=1, max=10)
  → Wait → Post(1) → TryWait → GetValue(==0)
  → Post(2) → GetValue(==2)
  → Destroy
```

The binary returns `0xC0DE` on full round-trip success;
step-specific failure codes 1..=10 preserve which syscall (or
value check) faulted.

## Syscalls covered

| PSL1GHT call | LV2 syscall | Rust handler |
|---|---|---|
| `sysSemCreate(*sem, *attr, initial, max)` | `#90`  | `EmuCore::dispatch_syscall` (R10.1.e) → `Lv2SyncState::sema_create` (R10.4 `SyncTable`) |
| `sysSemDestroy(sem)`                      | `#91`  | R10.1.e → `sema_destroy` |
| `sysSemWait(sem, timeout)`                | `#92`  | R10.1.e → `sema_wait` (single-PPU; MustBlock surfaces as `ETIMEDOUT`) |
| `sysSemTryWait(sem)`                      | `#93`  | R10.1.e → `sema_trywait` |
| `sysSemPost(sem, count)`                  | `#94`  | R10.1.e → `sema_post` |
| `sysSemGetValue(sem, *count)`             | `#114` | R10.1.e → `sema_get_value` + write s32 BE to `*count` |

Note: `sysSemGetValue` lives at syscall **#114**, NOT in the 90-95
band — PSL1GHT puts it after the rwlock family. The arm picks the
syscall up by number alone; the dispatcher doesn't care about
adjacency.

`sys_sem_attr_t` (32 bytes BE: protocol/pshared + key + flags +
pad + name[8]) is parsed by the dispatcher arm and mapped to
`rpcs3-lv2-sync::SemaAttr`.

## Build

```
# from any PowerShell session
subst R: "C:\Users\manod\Downloads\Emulador Ps2, ps1 e ps3 nativos\rpcs3-master\behavior-freeze\fixtures\lv2\sources"

docker run --rm -v "R:\single_sema_v1:/work" -w /work \
    rpcs3-ps3dev-toolchain:local \
    bash -lc "make"
```

Output: `single_sema_v1.self` in this directory (gitignored —
`.self` extension blocked by the path-lock hook).

## Validation

The Rust smoke test
[`rust/rpcs3-emu-core/tests/run_self_sema_smoke.rs`](../../../../../rust/rpcs3-emu-core/tests/run_self_sema_smoke.rs)
loads the `.self` (when present) and asserts the exit status
equals `0xC0DE`. Skips gracefully when the fixture binary is
absent.

## CC0

CC0 1.0 Universal — see `LICENSE.md`.
