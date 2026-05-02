# single_spu_mailbox_multi_v1 — License

This fixture (PPU loader source `main.c`, SPU program source `spu/spu_mailbox_multi.c`, build script `Makefile`, accompanying docs) is original work authored for the RPCS3 → Rust port project's behavior-freeze fixture set.

**Dedication: CC0 1.0 Universal (Public Domain Dedication).**

To the extent possible under law, the author has dedicated all copyright and related and neighboring rights to this software to the public domain worldwide. This software is distributed without any warranty.

See <https://creativecommons.org/publicdomain/zero/1.0/> for the full text.

## Why CC0?

Same reason as the other R5.11 / R5.11b fixtures (`single_spu_mailbox_v1`, `single_spu_branch_loop_v1`, `single_spu_loadstore_v1`, `single_spu_signal_v1`): public-domain dedication maximises redistribution freedom for fixture sources, build artifacts, captured traces, and side-files.

## Build dependencies

The build requires the PSL1GHT toolchain (https://github.com/ps3dev/PSL1GHT, MIT-style licensed). The toolchain itself is NOT redistributed in this fixture — only the source code and resulting binary that exercises its API.

## Provenance

- Author: this project's contributors (via the autonomous R6.4b-pre iteration).
- First commit: 2026-05-01.
- Sources: `main.c`, `spu/spu_mailbox_multi.c`, `Makefile`, this `LICENSE.md`, `README.md`.
- No third-party code is copied into this fixture; PSL1GHT is invoked as an external dependency at build time.
- Authoring intent: provide the first oracle fixture that REQUIRES persistent-handle re-entry in the C++ ↔ Rust SPU bridge (R6.4b scope). Until the `.self` is built, an equivalent FFI-level acceptance gate lives at `rust/rpcs3-spu-ffi/src/tests.rs::rust_spu_mailbox_multi_round_via_ffi`.
