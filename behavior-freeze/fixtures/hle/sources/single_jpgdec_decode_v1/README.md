# single_jpgdec_decode_v1 (HLE backlog — cellJpgDec pixel decode via stb_image)

Create->Open->ReadHeader->SetParameter(RGBA)->DecodeData on an embedded real 16x16
JPEG. emu-core decodes via the vendored stb_image (rpcs3-stb-image, feature
image-decode) — the SAME decoder RPCS3 links — so the RGBA pixels are byte-exact.
Checksum of the decoded buffer vs the stb_image golden -> 0xC0DE. Oracle test is
#[cfg(feature = "image-decode")]. CC0 1.0 — see LICENSE.md.
