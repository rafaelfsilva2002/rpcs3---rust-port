# single_pngdec_decode_v1 (HLE backlog — cellPngDec pixel decode via stb_image)

Create->Open->ReadHeader->SetParameter(RGBA)->DecodeData on an embedded baseline
16x16 RGB PNG. RPCS3 uses libpng; emu-core uses the vendored stb_image
(rpcs3-stb-image, feature image-decode). For a baseline PNG (no gamma/sRGB/
interlace) both spec-decode to identical pixels -> byte-exact RGBA. Checksum vs the
stb_image golden -> 0xC0DE. Oracle test is #[cfg(feature = "image-decode")].
CC0 1.0 — see LICENSE.md.
