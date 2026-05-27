# RSX GCM stream fixtures — License

The `.gcmhex` command-stream fixtures in this directory are original
work authored for the RPCS3 → Rust port project's behavior-freeze
fixture set.

**Dedication: CC0 1.0 Universal (Public Domain Dedication).**

To the extent possible under law, the author has dedicated all
copyright and related and neighboring rights to this software to the
public domain worldwide. This software is distributed without any
warranty. See <https://creativecommons.org/publicdomain/zero/1.0/>.

## Provenance

These streams are **authored from the NV4097 / NV406E method-encoding
spec**, mirroring what PSL1GHT's `libgcm` would emit — they are NOT
captured from real hardware or RPCS3. They serve as the R12.10a
golden test rail that freezes the pure RSX command decoder
(`rpcs3-rsx-fifo` + `rpcs3-rsx-state`). A future R12.10b/R12.11 will
add *real captured* streams (requiring an RSX capture writer or a
minimal cellGcm HLE) replayed through the same `replay_gcm` harness.
