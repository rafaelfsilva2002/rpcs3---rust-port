# single_spu_mailbox_v1 — capture provenance (TEMPLATE)

> **Template only**. After R5.9e.7 capture lands, fill in the placeholders
> below and copy the result to
> `behavior-freeze/fixtures/spu/traces/single_spu_mailbox_v1.notes.md`.

## Origin

- **Source code**: [`behavior-freeze/fixtures/spu/sources/single_spu_mailbox_v1/`](../sources/single_spu_mailbox_v1/) (CC0 1.0; original work).
- **License**: CC0 1.0 (public-domain dedication; see [`LICENSE.md`](../sources/single_spu_mailbox_v1/LICENSE.md)).
- **Author**: this project's contributors (autonomous R5.9e.7 iteration, 2026-04-30).

## Build

- **Toolchain**: PSL1GHT, built from source via [ps3toolchain](https://github.com/ps3dev/ps3toolchain) (commit `<TODO: ps3toolchain HEAD>` at `<TODO: build start ISO date>`) inside Docker container `ps3-build` (debian:bookworm-slim base).
- **Build host**: Windows 11 + Docker Desktop 29.3.1 (linux engine), ps3toolchain build inside Debian 12 container.
- **Build command**: `make` (in the source dir, with `PS3DEV=/opt/ps3dev`, `PSL1GHT=/opt/ps3dev/psl1ght` from the toolchain output).
- **Output**: `single_spu_mailbox_v1.self` (sha256: `<TODO>`, size: `<TODO>` bytes).

## Capture

- **RPCS3 binary**: `R:\bin\rpcs3.exe` built `<TODO: build date>` from RPCS3 commit `<TODO: git HEAD at capture time>`.
- **Patches applied at capture time**:
  - `docs/patches/spu_trace_jsonl_scaffolding.patch` sha256 `d65aec91b6b2439b4befeaf6d51d64ddb98b9425726fc17abbc3d434ae1aba1c`
  - `docs/patches/spu_trace_jsonl_runtime_hooks.patch` sha256 `8f253d7d207793266eb3a81e809c73731a8e565757a9d2c40fa944a88266663a`
- **Capture command**:
  ```powershell
  $env:RPCS3_SPU_TRACE_JSONL = "$env:TEMP\single_spu_mailbox_v1.jsonl"
  R:\bin\rpcs3.exe --headless `
    "C:\Users\manod\Downloads\Emulador Ps2, ps1 e ps3 nativos\rpcs3-master\behavior-freeze\fixtures\spu\sources\single_spu_mailbox_v1\single_spu_mailbox_v1.self"
  ```
- **Capture date** (ISO): `<TODO>`.

## Trace shape

- **Lines**: `<TODO>` JSONL events.
- **`target_spu` distinct ids**: 1 (criterion 2 of R5.9e.7 spec).
- **`spu_image` events**: 1 (criterion 3).
- **`spu_wrch` ch21 (MFC_Cmd) events**: 0 (criterion 4 — non-DMA confirmed).
- **`spu_wrch` ch28 (OUT_MBOX) events**: ≥2 (criterion 5).
- **`final_state` events**: 1.
- **`spu_stop` events**: 1, with `code=0xD5` (criterion 6).

## `.spuimg` side-file

- **Filename**: `behavior-freeze/fixtures/spu/images/<TODO: sha256>.spuimg`.
- **Size**: `<TODO>` bytes (typically 262144 = 256 KiB local store dump).
- **SHA-256**: `<TODO>` (matches the trace's `spu_image.image_sha256`).

## Replay results

| Backend | stop_code | total_steps | final_snapshot |
|---|---:|---:|---|
| InterpreterExecutor | `0xD5` | `<TODO>` | `<TODO>` |
| RecompilerExecutor | `0xD5` | `<TODO>` | `<TODO>` |
| `diff_snapshots(interp, recomp).is_identical()` | | | `true` |

## Determinism

- The mailbox protocol is fully deterministic: PPU pushes 0x100 → SPU computes 0x129 → PPU pushes 0x200 → SPU computes 0x229 → PPU pushes 0xFFFFFFFF (sentinel) → SPU stops 0xD5.
- No FP, no DMA, no atomics, no time-dependent ops.
- No environmental dependencies: no FS access, no GFX, no audio, no input.

## Re-capture procedure

If the trace needs to be re-captured (e.g. after a writer-extension iteration):
1. Rebuild `.self` from this fixture's source dir using the same PSL1GHT toolchain.
2. Run the capture command above.
3. Run the integration test `cargo test -p rpcs3-spu-recompiler --release --test single_spu_mailbox_v1_replay` — it must pass.
4. Update this `.notes.md` with the new sha256s and dates.

## Acceptance gate

This fixture's commit caused `behavior-freeze/harness/check_trace_fixtures.py` to flip its `REPLAY_VALIDATED_TRACE_EXISTS` constant from `False` to `True`. The change in that file was reviewed alongside the trace + `.spuimg` + this notes file as a single atomic commit.
