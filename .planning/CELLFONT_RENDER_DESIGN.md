# cellFont rendering — glyph-metrics slice (R18) design

Self-authored after the design workflow (wf_5071d5a7) failed on structured-output
enforcement; the core research was done directly and is captured here.

## Verdict

The smallest byte-exact rendering slice is **glyph METRICS** (no rasterization):
`cellFontOpenFontMemory` + `cellFontSetScalePixel` + `cellFontGetCharGlyphMetrics`.
Metrics are pure TrueType table reads + IEEE-deterministic f32 arithmetic, so they
can be reproduced bit-for-bit. Rasterization (`cellFontRenderCharGlyphImage`,
glyph bitmaps) is a SEPARATE later slice — it has the same byte-exact story (stb's
rasterizer is deterministic) but more surface (render surface, bitmap output).

Estimated 1 slice for metrics (R18); rasterization = R19+ (deferred decision).

## Byte-exactness strategy (the crux)

RPCS3 cellFont uses **stb_truetype.h** (confirmed: cellFont.cpp:5 `#include
<stb_truetype.h>`). The metrics math (cellFont.cpp:889-901):

```
scale       = stbtt_ScaleForPixelHeight(stbfont, font->scale_y)   // = scale_y / (hhea.ascent - hhea.descent)
GetCodepointBox(stbfont, code, &x0,&y0,&x1,&y1)                    // stored glyf bbox (4 int16)
GetCodepointHMetrics(stbfont, code, &advanceWidth,&leftSideBearing) // hmtx
metrics.width      = (x1-x0) * scale
metrics.height     = (y1-y0) * scale
metrics.h_bearingX = leftSideBearing * scale
metrics.h_bearingY = 0
metrics.h_advance  = advanceWidth * scale
metrics.v_*        = 0
```

All inputs are direct table reads (glyf stored bbox, hmtx, hhea); stbtt_GetGlyphBox
reads the STORED bbox (glyph header xMin/yMin/xMax/yMax), NOT a computed one. The
arithmetic is `(int) * f32` and `f32 / f32` — IEEE 754 deterministic.

**Engine choice: the `stb_truetype` Rust crate** (docs.rs/stb_truetype, a faithful
port of stb_truetype.h) — exposes `FontInfo::new`, `scale_for_pixel_height`,
`get_codepoint_box`, `get_codepoint_hmetrics`. Pure Rust (no C dep → keeps the
toaster/cross-compile story clean) and byte-exact because it is the same algorithm.

**De-risk / proof of parity (NOT circular):** compute golden metrics with the
ACTUAL C `stb_truetype.h` (compiled in the PSL1GHT Docker image — single header,
no deps) on the chosen font+glyph+scale. That is the RPCS3-equivalent output. Then:
- a calibration unit test in `rpcs3-hle-cellfont` asserts the Rust crate reproduces
  the C golden bit-for-bit (proves crate == RPCS3 stbtt);
- the homebrew hardcodes the same golden and compares emu-core's output → 0xC0DE.

If the crate ever diverges from the C golden, the calibration test fails loudly —
the byte-exact contract is enforced, not assumed.

## Struct layouts (BE; from cellFont.h)

- `CellFont` (28 B + ptr): scale_x@0, scale_y@4, slant@8, renderer_addr@12,
  fontdata_addr@16, origin@20, stbfont@24 (RPCS3 stores the stbtt_fontinfo in-guest
  at font+size — a hack we do NOT replicate; we key a host-side parsed font by the
  CellFont guest address instead).
- `CellFontGlyphMetrics` (32 B): width@0, height@4, h_bearingX@8, h_bearingY@12,
  h_advance@16, v_bearingX@20, v_bearingY@24, v_advance@28.
- `CellFontConfig` (known, R17): fc_buffer@0, fc_size@4, userFontEntryMax@8,
  userFontEntrys@12, flags@16.

## Minimal homebrew call sequence (PSL1GHT libfont)

```c
fontInit(&config_with_cache>=24)        // R17 — cellFontInitializeWithRevision
fontInitLibraryFreeType(&ftcfg, &lib)   // OR a library-NONE path; TBD at probe
fontOpenFontMemory(lib, fontAddr, fontSize, 0, 0, &font)  // cellFontOpenFontMemory
fontSetScalePixel(&font, w, h)          // cellFontSetScalePixel (w,h in FPRs f1,f2)
fontGetCharGlyphMetrics(&font, 'A', &m) // cellFontGetCharGlyphMetrics
// compare m.* against hardcoded golden f32 (bit-exact) -> 0xC0DE
```

NID arms to capture at runtime (never guessed): cellFontOpenFontMemory,
cellFontSetScalePixel, cellFontGetCharGlyphMetrics, and whatever
fontInitLibraryFreeType resolves to (may need a library handle). cellFontEnd +
cellFontInitializeWithRevision already wired (R17).

**Open question (probe resolves):** does getCharGlyphMetrics need a library/renderer
bound, or only an opened font? RPCS3's getCharGlyphMetrics only touches
font->stbfont + font->scale_y — no renderer. So open + setScale + metrics suffices.
fontInitLibraryFreeType may still be required by PSL1GHT to produce a `lib` handle
that fontOpenFontMemory wants non-null (cellFont.cpp:121 rejects !library).

## CC0 font

Generate a SYNTHETIC minimal TTF (Python fontTools `fontBuilder`) with a couple of
glyphs (e.g. 'A','B') — a fresh creation, CC0, no third-party font license. Embed
as a C byte array in the homebrew (a few hundred bytes). Because we design the
outline + set hhea ascent/descent, the table values are known by construction; the
C-stbtt golden cross-checks the exact f32 outputs.

## Build order (probe-then-implement, each step gated)

1. Generate `testfont.ttf` (fontTools) + dump its tables; compute C-stbtt golden
   metrics for ('A', scale_y=H) in Docker.
2. Add `stb_truetype` crate to `rpcs3-hle-cellfont`; calibration unit test
   (crate metrics == C golden, bit-exact). If the crate won't build / diverges,
   fall back to vendoring the ~200 LOC of stbtt metrics path.
3. Add a real `StbttFontBackend` to `rpcs3-hle-cellfont` (alongside StubFontBackend)
   implementing open-from-bytes + glyph-metrics via the crate, returning the exact
   CellFontGlyphMetrics math above.
4. Author the homebrew fixture (embed font + golden); build .self via Docker.
5. Probe NIDs at runtime (permissive) for OpenFontMemory / SetScalePixel /
   GetCharGlyphMetrics / FreeType-lib-init.
6. Wire emu-core arms: maintain a host-side `FontManager` field; OpenFontMemory
   parses guest font bytes → handle keyed by CellFont addr; SetScalePixel writes
   scale into the guest struct (faithful); GetCharGlyphMetrics looks up the font,
   reads scale_y, computes + writes CellFontGlyphMetrics.
7. Oracle → 0xC0DE; full gate; commit.

## Risks / open questions

- **fontInitLibraryFreeType**: may pull in FreeType-specific NIDs or need guest
  malloc callbacks. Mitigation: prefer the library-type-NONE path if PSL1GHT allows
  a non-null lib handle without FreeType; resolve at probe.
- **`stb_truetype` crate maintenance**: old (0.2/0.3). If it fails to build on
  current toolchain, vendor the metrics subset. (Calibration test guards parity.)
- **Synthetic font validity**: stbtt_InitFont must accept it (needs valid head/
  maxp/loca/glyf/cmap/hmtx/hhea). fontBuilder produces a valid TTF; verify with the
  C-stbtt golden step (if InitFont rejects it, the golden step fails first).
- **FPR arg read**: SetScalePixel passes w,h in f1,f2 (f32 promoted to f64 in FPR);
  read `fpr[1] as f32`.
