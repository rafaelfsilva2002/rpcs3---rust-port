# single_pngdec_header_v1 (HLE backlog — cellPngDec header, CALLBACK-driven)

cellPngDec `Create -> Open(BUFFER) -> ReadHeader`. UNLIKE cellJpgDec, cellPngDec
is callback-driven: Create + Open invoke the guest `cbCtrlMalloc` callback to
allocate the handle/stream in guest memory (cellPngDec.cpp:344/390). emu-core
drives those via `EmuCore::call_guest_function` (R14). ReadHeader's width/height/
numComponents/colorSpace/bitDepth come from the PNG IHDR chunk (RPCS3 uses libpng;
those values ARE the IHDR fields), parsed byte-exact by
`rpcs3_hle_cellpngdec::PngDec::parse_header`.

Embeds a minimal RGB PNG (320x240, depth 8; png_data.h via gen_png.py). Checks
width/height/num_comp/color_space/bit_depth -> 0xC0DE. Pixel decode
(SetParameter/DecodeData, which uses libpng in RPCS3) is a separate later slice.
Consumed by hle_pngdec_header.rs. CC0 1.0 — see LICENSE.md.
