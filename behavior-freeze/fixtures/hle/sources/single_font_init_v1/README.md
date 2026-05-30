# single_font_init_v1 (R17 — cellFont init lifecycle)

The cellFont **entry path**, behavior-frozen without any glyph rendering. PSL1GHT's
inline `fontInit` calls the local `fontGetStubRevisionFlags` then the real
`cellFontInitializeWithRevision` NID, which validates the file-cache size
(`config->fc_size < 24 → CELL_FONT_ERROR_INVALID_PARAMETER`, cellFont.cpp:54).
`fontEnd` is `cellFontEnd`.

## Behaviour

Self-contained oracle — exercises the `fc_size >= 24` invariant both ways:

```c
fontConfig config; fontConfig_initialize(&config);
config.fileCache.buffer = g_filecache;
config.fileCache.size = 0;                 // < 24
if (fontInit(&config) == 0) return 0xBAD2; // must be REJECTED
config.fileCache.size = sizeof(g_filecache);
if (fontInit(&config) != 0) return 0xBAD0; // must SUCCEED
if (fontEnd()        != 0) return 0xBAD1;
return 0xC0DE;
```

No SPU / RSX / FreeType — pure cellFont init NIDs.

## How it wires

- `cellFontInitializeWithRevision` (NID captured at runtime) → emu-core arm:
  reads `config->fc_size` (offset 4, BE); `< 24` → `0x80540002`
  (CELL_FONT_ERROR_INVALID_PARAMETER), else `CELL_OK`. Mirrors cellFont.cpp:50.
- `cellFontEnd` (NID) → `CELL_OK` (cellFont.cpp:79).

## Result

`EmuCore::run_self` exit status = **0xC0DE** (invariant enforced both ways), vs
**0xBAD2** (a too-small cache wrongly accepted) / **0xBAD0** (a valid cache
wrongly rejected) / **0xBAD1** (end failed).

## Deferred

The cellFont *rendering* path — `cellFontOpenFontFile`/`cellFontGetCharGlyphMetrics`/
`cellFontRenderCharGlyphImage` — needs real TrueType parsing + rasterization
(RPCS3 uses stb_truetype). That is a giant tail and a separate design call; it is
NOT part of this init slice.

## Consumed by

`rust/rpcs3-emu-core/tests/hle_font_init.rs`. The `.self`/`.elf` are built
locally via the PSL1GHT Docker toolchain and gitignored.

CC0 1.0 (public domain) — see LICENSE.md.
