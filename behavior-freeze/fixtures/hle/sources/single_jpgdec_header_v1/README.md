# single_jpgdec_header_v1 (HLE backlog — cellJpgDec header parse)

cellJpgDec `Create -> Open(BUFFER) -> ReadHeader`. RPCS3's `cellJpgDecReadHeader`
(cellJpgDec.cpp:146-178) parses the JPEG header bytes MANUALLY — no stb_image,
no guest callbacks: it walks the JFIF segment chain to the `FF C0` SOF0 marker
and reads width/height from it. emu-core ports that parse byte-exact via
`rpcs3_hle_celljpgdec::JpgDec::parse_header` (faithful to the RPCS3 `*0xFF`
segment-length quirk; width/height use `*0x100`; numComponents=3, colorSpace=RGB).

## Behaviour

Embeds a minimal JFIF (SOI + APP0 len-16 + SOF0 width=320 height=240), opens it
as a BUFFER source, reads the header, and checks:
`width==320 && height==240 && num_comp==3 && color_space==JPGDEC_RGB(2)` -> 0xC0DE
(distinct 0xBADn per failing step).

## How it wires (emu-core arms, NIDs captured at runtime)

- `cellJpgDecCreate` -> `JpgDec::create` (SEQ gate), CELL_OK (cellJpgDec.cpp:36).
- `cellJpgDecOpen` -> reads `jpgDecSource`, `JpgDec::open` allocates a subHandle
  (id from 1), writes `*subHandle` + `openInfo.initSpaceAllocated` (cellJpgDec.cpp:54).
- `cellJpgDecReadHeader` -> `JpgDec::stream_window` finds the buffer, reads it from
  guest memory, `JpgDec::parse_header` extracts the dims, writes `jpgDecInfo`
  {width, height, num_comp, color_space} BE (cellJpgDec.cpp:112).

Pixel decode (`cellJpgDecSetParameter`/`DecodeData`, which use stb_image in RPCS3)
is a separate later slice.

## Consumed by

`rust/rpcs3-emu-core/tests/hle_jpgdec_header.rs`. `.self`/`.elf` built via Docker
+ gitignored.

CC0 1.0 (public domain) — see LICENSE.md.
