// Compiles the stb_truetype C shim ONLY when the `cellfont-raster` feature is
// on. Default builds stay pure-Rust (no C toolchain needed) — the rasterizer is
// opt-in because it is the one cellFont path that needs the actual C
// stb_truetype rasterizer for byte-exact parity with RPCS3.

fn main() {
    #[cfg(feature = "cellfont-raster")]
    {
        cc::Build::new()
            .file("cbits/stbtt_shim.c")
            .include("cbits")
            // stb_truetype.h is warning-noisy under -Wall; keep the build quiet.
            .warnings(false)
            .compile("stbtt_shim");
        println!("cargo:rerun-if-changed=cbits/stbtt_shim.c");
        println!("cargo:rerun-if-changed=cbits/stb_truetype.h");
    }
}
