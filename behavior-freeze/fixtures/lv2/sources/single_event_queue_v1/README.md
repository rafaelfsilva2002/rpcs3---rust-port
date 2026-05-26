# single_event_queue_v1

Third PPU-only fixture in the behavior-freeze set (after
`single_mutex_v1`, `single_sema_v1`). Exercises the LV2
`sys_event_queue_*` + `sys_event_port_*` syscall family
end-to-end via PSL1GHT's `<sys/event_queue.h>`:

```
queueCreate → portCreate → portConnectLocal
  → portSend(0xAA, 0xBB, 0xCC)
  → queueReceive (event already queued → returns immediately)
  → verify data_1/2/3
  → portDisconnect → portDestroy → queueDestroy
```

Returns `0xC0DE` on full success; step codes 1..=11 on failure.

## Single-thread receive

`sysEventQueueReceive` (#130) is normally a blocking call. This
fixture sends the event via `sysEventPortSend` BEFORE receiving,
so the queue is non-empty and receive returns the event without
parking — safe on the single-PPU EmuCore (which can't suspend a
thread).

## Syscalls covered

| PSL1GHT call | LV2 syscall | Rust handler |
|---|---|---|
| `sysEventQueueCreate(*q, *attr, key, size)` | `#128` | `EmuCore::dispatch_syscall` (R10.1.f) → `Lv2SyncState::queue_create` (R10.6 `EventRegistry`) |
| `sysEventQueueDestroy(q, mode)`             | `#129` | R10.1.f → `queue_destroy` |
| `sysEventQueueReceive(q, *event, timeout)`  | `#130` | R10.1.f → `queue_receive` (writes 32-byte sys_event_t BE; MustBlock → ETIMEDOUT single-PPU) |
| `sysEventPortCreate(*port, type, name)`     | `#134` | R10.1.f → `port_create` |
| `sysEventPortDestroy(port)`                 | `#135` | R10.1.f → `port_destroy` |
| `sysEventPortConnectLocal(port, q)`         | `#136` | R10.1.f → `port_connect_local` |
| `sysEventPortDisconnect(port)`              | `#137` | R10.1.f → `port_disconnect` |
| `sysEventPortSend(port, d0, d1, d2)`        | `#138` | R10.1.f → `port_send` |

`sys_event_queue_attr_t` (16 bytes BE: protocol + type + name[8])
is parsed by the create arm and mapped to
`rpcs3-lv2-event::QueueAttr`.

Note: PSL1GHT does NOT expose `sysEventQueueTryReceive` — only the
blocking `Receive`. The `queue_tryreceive` / `queue_drain`
EventRegistry methods stay unit-test-only.

## Build

```
subst R: "C:\Users\manod\Downloads\Emulador Ps2, ps1 e ps3 nativos\rpcs3-master\behavior-freeze\fixtures\lv2\sources"

docker run --rm -v "R:\single_event_queue_v1:/work" -w /work \
    rpcs3-ps3dev-toolchain:local \
    bash -lc "make"
```

Output: `single_event_queue_v1.self` (gitignored).

## Validation

[`rust/rpcs3-emu-core/tests/run_self_event_queue_smoke.rs`](../../../../../rust/rpcs3-emu-core/tests/run_self_event_queue_smoke.rs)
loads the `.self` (when present) and asserts exit status `0xC0DE`.
Skips when absent.

## CC0

CC0 1.0 Universal — see `LICENSE.md`.
