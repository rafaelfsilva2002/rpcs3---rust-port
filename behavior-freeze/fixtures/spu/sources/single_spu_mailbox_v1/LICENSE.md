# single_spu_mailbox_v1 — License

This fixture (PPU loader source `main.c`, SPU program source `spu/spu_mailbox.c`, build script `Makefile`, accompanying docs) is original work authored for the RPCS3 → Rust port project's behavior-freeze fixture set.

**Dedication: CC0 1.0 Universal (Public Domain Dedication).**

To the extent possible under law, the author has dedicated all copyright and related and neighboring rights to this software to the public domain worldwide. This software is distributed without any warranty.

See <https://creativecommons.org/publicdomain/zero/1.0/> for the full text.

## Why CC0?

This fixture's purpose is to be the load-bearing oracle for replay-validated SPU traces in the project's `behavior-freeze/fixtures/spu/traces/` directory. Public-domain dedication maximises redistribution freedom — the fixture and its derived `.self` binary, `.jsonl` trace, and `.spuimg` side-file can all be committed to the repo without license-creep concerns.

## Build dependencies

The build requires the PSL1GHT toolchain (https://github.com/ps3dev/PSL1GHT, MIT-style licensed). The toolchain itself is NOT redistributed in this fixture — only the source code and resulting binary that exercises its API.

## Provenance

- Author: this project's contributors (via the autonomous R5.9e.7 iteration).
- First commit: 2026-04-30.
- Sources: `main.c`, `spu/spu_mailbox.c`, `Makefile`, this `LICENSE.md`, `README.md`.
- No third-party code is copied into this fixture; PSL1GHT is invoked as an external dependency at build time.
