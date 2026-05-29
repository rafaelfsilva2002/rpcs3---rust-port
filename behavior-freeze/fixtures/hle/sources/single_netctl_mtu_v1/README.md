# single_netctl_mtu_v1 (HLE wave — cellNetCtl get-info MTU)

Second cellNetCtl function, reusing the `NetCtlManager` field + connected backend
from `single_netctl_state_v1` — only the `cellNetCtlGetInfo` NID is new (one more
match arm, no new dep/field).

## Behaviour

A PPU-only homebrew that calls `cellNetCtlInit()` then
`cellNetCtlGetInfo(NET_CTL_INFO_MTU, &info)` (via PSL1GHT's libnetctl) into a
zero-initialised `union net_ctl_info`:

- `init != 0 || getinfo != 0` → returns `0x0BAD`
- else → returns `info.mtu` (`0` pre-wire — union untouched; `1500` post-wire =
  the default MTU)

`mtu` is the union's u32 member at offset 0. No printf / SPU / RSX.

## How it wires

1. The `cellNetCtlGetInfo` NID fires (captured at runtime; r3=code, r4=&info).
2. The dispatcher routes it to `cell_net_ctl_get_info(&self.netctl,
   &StubConnectedBackend{..}, code)`; for `INFO_MTU` it returns `NetInfo::Mtu(1500)`,
   which the arm writes as BE u32 to the OUT union. (Only the MTU path is
   fixture-validated; other info codes are wired best-effort.)

## Result

`EmuCore::run_self` exit status = **1500** (`0x5DC`), vs `0` pre-wire — proving
the second cellNetCtl function runs end-to-end off the shared field.

## Consumed by

`rust/rpcs3-emu-core/tests/hle_netctl_mtu.rs`. The `.self`/`.elf` are built
locally via the PSL1GHT Docker toolchain and gitignored.

CC0 1.0 (public domain) — see LICENSE.md.
