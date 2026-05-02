# game_like_mailbox_signal_v1 — License

This fixture (PPU loader source `main.c`, SPU program source `spu/spu_game_like.c`, build script `Makefile`, accompanying docs) is original work authored for the RPCS3 → Rust port project's behavior-freeze fixture set.

**Dedication: CC0 1.0 Universal (Public Domain Dedication).**

To the extent possible under law, the author has dedicated all copyright and related and neighboring rights to this software to the public domain worldwide. This software is distributed without any warranty.

See <https://creativecommons.org/publicdomain/zero/1.0/> for the full text.

## Why CC0?

Same reason as the other R5.11 / R5.11b / R6.4b fixtures (`single_spu_mailbox_v1`, `single_spu_branch_loop_v1`, `single_spu_loadstore_v1`, `single_spu_signal_v1`, `single_spu_mailbox_multi_v1`): public-domain dedication maximises redistribution freedom for fixture sources, build artifacts, captured traces, and side-files.

## Build dependencies

The build requires the PSL1GHT toolchain (https://github.com/ps3dev/PSL1GHT, MIT-style licensed). The toolchain itself is NOT redistributed in this fixture — only the source code and resulting binary that exercises its API.

Build is reproducible via the `rpcs3-ps3dev-toolchain:local` Docker image scaffolded at `.claude/ps3toolchain-docker/Dockerfile` in this repo.

## Provenance

- Author: this project's contributors (via the autonomous R6.6 iteration).
- First commit: 2026-05-01.
- Sources: `main.c`, `spu/spu_game_like.c`, `Makefile`, this `LICENSE.md`, `README.md`.
- No third-party code is copied into this fixture; PSL1GHT is invoked as an external dependency at build time.
- Authoring intent: provide the first "game-like" fixture combining IN_MBOX + SNR1 + LS load/store + branch/loop + accumulated state in a single workload, exercising multiple bridge code paths simultaneously to surface any cross-path bugs in the C++↔Rust SPU bridge (R6.4b persistent-handle infrastructure).
