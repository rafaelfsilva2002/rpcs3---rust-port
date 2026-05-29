# single_net_inet_addr_v1 (HLE wave — sys_net inet_addr)

Wires the `sys_net` module's `inet_addr` into `EmuCore` — a stateless function
returned directly in `r3`.

## Behaviour

A PPU-only homebrew that calls `inet_addr("1.2.3.4")` (via PSL1GHT's libnet,
`arpa/inet.h`). On real PS3 firmware `sys_net_inet_addr` is a STUB that
unconditionally returns `INET_ADDR_NONE` (`0xFFFFFFFF`);
`rpcs3-hle-sys-net-user::inet_addr_stub` mirrors this byte-exact.

- `inet_addr == 0xFFFFFFFF` → returns `1` (post-wire firmware-stub behaviour)
- else → returns `0` (pre-wire: the return-0 import stub)

No printf / SPU / RSX — a clean PPU + sys_net HLE call.

## How it wires

1. The guest call fires the sys_net inet_addr NID (captured at runtime from the
   `[R9.1g.7] unimplemented import` log).
2. The dispatcher routes it to
   `rpcs3_hle_sys_net_user::inet_addr_stub(true)` and returns `0xFFFFFFFF` in r3.

## Result

`EmuCore::run_self` exit status = **1**, vs `0` pre-wire — proving the call runs
end-to-end.

## Consumed by

`rust/rpcs3-emu-core/tests/hle_net_inet_addr.rs`. The `.self`/`.elf` are built
locally via the PSL1GHT Docker toolchain and gitignored.

CC0 1.0 (public domain) — see LICENSE.md.
