# single_spu_mailbox_v1

A minimal license-clean PPU+SPU homebrew that exercises a deterministic mailbox handshake with no DMA. The first replay-validated fixture target for the `behavior-freeze/fixtures/spu/traces/` directory.

## Behaviour

```
PPU side (main.c):
  1. Initialize SPU subsystem (sysSpuInitialize).
  2. Create exactly one SPU thread group with one cooperative SPU thread.
  3. Push command #1 (value 0x100) into the SPU's IN_MBOX.
  4. Drain SPU OUT_MBOX -> expect 0x129 (= 0x100 + 0x29).
  5. Push command #2 (value 0x200) into the SPU's IN_MBOX.
  6. Drain SPU OUT_MBOX -> expect 0x229 (= 0x200 + 0x29).
  7. Push sentinel command 0xFFFFFFFF -> SPU recognises "halt".
  8. Wait for SPU to stop (sys_spu_thread_join), then exit.

SPU side (spu/spu_mailbox.c -> spu_mailbox.elf, embedded in PPU executable):
  loop:
    rdch  r3, ch29 (SPU_RdInMbox)        ; blocking read of PPU command
    ceqi  r4, r3, -1                      ; is sentinel?
    brnz  r4, halt                        ; yes -> halt
    ai    r5, r3, 0x29                    ; compute reply
    wrch  r5, ch28 (SPU_WrOutMbox)        ; blocking write to PPU
    br    loop
  halt:
    stop  0xD5                            ; deterministic halt code
```

## Why this shape

- **Single SPU**: required by R5.9e schema; multi-SPU is deferred to R5.9f.
- **No DMA**: no `wrch ch21 (MFC_Cmd)`. Replay can run end-to-end without the EA-memory model.
- **Mailbox-only**: exercises the channels the R5.9c writer captures (`ch28` SPU_WrOutMbox + PPU push/pop hooks).
- **Sentinel-driven halt**: deterministic stop_code (0xD5) for the `final_state` assertion.
- **Computation `cmd + 0x29`**: arbitrary, distinguishable result; chosen so the test can assert `0x100 + 0x29 == 0x129` and `0x200 + 0x29 == 0x229` byte-exact.
- **No I/O / no FS / no GFX / no audio**: the homebrew is pure compute + IPC; nothing leaves the emulated SPU/PPU.

## Build

Requires PSL1GHT toolchain. After PS3DEV+PSL1GHT are installed and on PATH:

```bash
make
# Produces: single_spu_mailbox_v1.self
```

## Capture trace

```bash
# With the R5.9c+R5.9e.3-extended rpcs3.exe:
RPCS3_SPU_TRACE_JSONL=/tmp/single_spu_mailbox_v1.jsonl \
  /path/to/rpcs3.exe --headless single_spu_mailbox_v1.self

# Output:
#   /tmp/single_spu_mailbox_v1.jsonl                    (event timeline)
#   /tmp/single_spu_mailbox_v1.images/<sha>.spuimg      (LS image side-file)
```

## Acceptance for fixture commit

See [`docs/PROJECT_STATUS.md`](../../../../docs/PROJECT_STATUS.md) § "R5.9e.7 planning iteration" for the full 15-criterion acceptance contract. Summary:

- Trace has exactly 1 `target_spu`, 1 `spu_image` event, zero `spu_wrch ch21` events.
- Parser + transformer + builder all pass.
- Replay × Interpreter reaches `Finished{0xD5}` and final-state matches.
- Replay × Recompiler reaches `Finished{0xD5}` and final-state matches.
- `diff_snapshots(interp, recomp).is_identical()` holds.

## License

CC0 1.0 — see [`LICENSE.md`](LICENSE.md).
