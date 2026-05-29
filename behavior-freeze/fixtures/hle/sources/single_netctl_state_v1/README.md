# single_netctl_state_v1 (HLE wave — cellNetCtl init + get-state)

Wires `cellNetCtl` into `EmuCore`: a **stateful** manager (`NetCtlManager` field,
gating `initialized`) plus a fixed **connected-network provider**
(`StubConnectedBackend`) so the reported state is non-zero.

## Behaviour

A PPU-only homebrew that calls `cellNetCtlInit()` then
`cellNetCtlGetState(&state)` (via PSL1GHT's libnetctl) into a sentinel-filled
`state`:

- `init != 0 || getstate != 0` → returns `0x0BAD`
- else → returns `state` (`0x55` pre-wire — OUT untouched; `3` =
  `CELL_NET_CTL_STATE_IPOBTAINED` post-wire, because emu-core stages a connected
  network backend)

No printf / SPU / RSX — pure cellNetCtl HLE calls.

## How it wires

1. Two NIDs fire (init + get-state), captured at runtime from the
   `[R9.1g.7] unimplemented import` log.
2. `rpcs3-emu-core` gains a `netctl: NetCtlManager` field (init in `new()`).
   `cellNetCtlInit` → `cell_net_ctl_init(&mut self.netctl)` (flips `initialized`);
   `cellNetCtlGetState` → `cell_net_ctl_get_state(&self.netctl,
   &StubConnectedBackend{..})` → writes the state (BE s32) to the OUT pointer.
3. The connected backend is a fixed config decision (emulated console reports an
   established network), mirroring how RPCS3 emulates an active connection.

## Result

`EmuCore::run_self` exit status = **3** (IPOBTAINED), vs `0x55` pre-wire — proving
the stateful init→query path runs end-to-end.

## Consumed by

`rust/rpcs3-emu-core/tests/hle_netctl_state.rs`. The `.self`/`.elf` are built
locally via the PSL1GHT Docker toolchain and gitignored.

CC0 1.0 (public domain) — see LICENSE.md.
