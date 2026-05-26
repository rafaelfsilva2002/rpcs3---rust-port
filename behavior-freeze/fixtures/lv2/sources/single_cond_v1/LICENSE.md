# single_cond_v1 — License

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

Fourth PPU-only entry in the behavior-freeze fixture set (after
mutex, sema, event_queue). Exercises the single-PPU-reachable
subset of the LV2 `sys_cond_*` syscall path
(create / signal-empty / broadcast-empty / destroy) end-to-end
through `rpcs3-emu-core::run_self`, validating the R10.3
`CondRegistry` trait impl and its dispatcher arms (#105, #106,
#108, #109). The blocking `sys_cond_wait` (#107) is not
exercisable on a single PPU and is excluded.

## Build dependencies

The build requires the PSL1GHT toolchain
(<https://github.com/ps3dev/PSL1GHT>, MIT-style licensed), invoked
as an external dependency at build time.

## Provenance

- Author: this project's contributors (via the autonomous R10.1.g
  iteration).
- First commit: 2026-05-26.
- Sources: `main.c`, `Makefile`, this `LICENSE.md`, `README.md`.
