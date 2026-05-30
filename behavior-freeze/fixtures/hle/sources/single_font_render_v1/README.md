# single_font_render_v1 (R20 — cellFont byte-exact glyph rasterization)

The cellFont **rendering** path: rasterize a glyph to an 8-bit coverage surface.
RPCS3 calls `stbtt_GetCodepointBitmap` (cellFont.cpp:713); emu-core uses the
**vendored C `stb_truetype.h`** (cargo feature `cellfont-raster`, default OFF),
so the rendered surface is byte-exact — the Rust `stb_truetype` crate has no
rasterizer, so this is the one cellFont path that needs the real C stb.

## Behaviour

```c
fontInit -> fontInitLibraryFreeType -> fontOpenFontMemory -> fontSetScalePixel(32)
fontCreateRenderer -> fontBindRenderer            // renderer lifecycle
fontRenderSurfaceInit(&surface, g_surface, 64, 1, 64, 64)
fontRenderCharGlyphImage(&f, 'A', &surface, 0, 0, &m, &trans)  // the blit
sum(g_surface[0..64*64]) == 73114 ? 0xC0DE : 0xBAD7
```

The golden `73114` is the stb_truetype v2 rasterizer coverage sum for the
synthetic 'A' rect at scale_y=32 (13x23 bitmap, baseLineY=25), blitted at (0,0)
into the 64x64 surface (whole glyph fits → surface sum == bitmap sum). See
`rust/rpcs3-hle-cellfont/tests/raster_calibration.rs`.

## How it wires (emu-core arms, NIDs captured at runtime)

- `cellFontCreateRenderer` → validates args, sets `renderer->systemReserved[0x10]`
  non-null so bind passes (cellFont.cpp:351; RPCS3's stub omits this).
- `cellFontBindRenderer` → sets `font->renderer_addr` (cellFont.cpp:563).
- `cellFontRenderSurfaceInit` → fills the surface struct (cellFont.cpp:370) — if
  it is a NID rather than a PSL1GHT inline.
- `cellFontRenderCharGlyphImage` → `StbttFont::render_into` does the byte-exact
  blit (cellFont.cpp:726-740): coverage at row `y+ypos+yoff+baseLineY`, col
  `x+xpos`, surface->width-strided, u32-cast bounds checks.

## Result

`EmuCore::run_self` (with `--features cellfont-raster`) exit = **0xC0DE**.
Without the feature, the rasterizer is absent → render is a no-op → 0xBAD7, so
this oracle's test is `#[cfg(feature = "cellfont-raster")]`.

## Consumed by

`rust/rpcs3-emu-core/tests/hle_font_render.rs` (feature-gated). `.self`/`.elf`
built via Docker + gitignored; `testfont.ttf` + `font_data.h` committed (CC0).

CC0 1.0 (public domain) — see LICENSE.md.
