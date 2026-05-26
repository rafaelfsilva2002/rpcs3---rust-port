# single_sema_v1 — License

This fixture (PPU source `main.c`, build script `Makefile`,
accompanying docs) is original work authored for the RPCS3 → Rust
port project's behavior-freeze fixture set.

**Dedication: CC0 1.0 Universal (Public Domain Dedication).**

To the extent possible under law, the author has dedicated all
copyright and related and neighboring rights to this software to the
public domain worldwide. This software is distributed without any
warranty.

See <https://creativecommons.org/publicdomain/zero/1.0/> for the full
text.

## Why CC0?

This fixture is the second PPU-only entry in the behavior-freeze
fixture set (after `single_mutex_v1`). Its purpose is to exercise
the LV2 kernel `sys_semaphore_*` syscall path end-to-end through
`rpcs3-emu-core::run_self`, validating the R10.4 trait impl and
its dispatcher arms (#90, #91, #92, #93, #94, #114) against a
real PSL1GHT-compiled binary. Public-domain dedication maximises
redistribution freedom for the fixture source and its derived
`.self` binary.

## Build dependencies

The build requires the PSL1GHT toolchain
(<https://github.com/ps3dev/PSL1GHT>, MIT-style licensed). The
toolchain itself is NOT redistributed in this fixture — only the
source code and resulting binary that exercises its API.

## Provenance

- Author: this project's contributors (via the autonomous R10.1.e
  iteration).
- First commit: 2026-05-26.
- Sources: `main.c`, `Makefile`, this `LICENSE.md`, `README.md`.
- No third-party code is copied into this fixture; PSL1GHT is invoked
  as an external dependency at build time.
