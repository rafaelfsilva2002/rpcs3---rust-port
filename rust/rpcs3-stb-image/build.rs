// Compiles the stb_image C shim ONLY when the `decode` feature is on. Default
// builds stay pure-Rust (no C toolchain) — image decode is opt-in because it is
// the one path that needs the actual C stb_image for byte-exact parity with
// RPCS3's cellJpgDec (which links stb_image).

fn main() {
    #[cfg(feature = "decode")]
    {
        cc::Build::new()
            .file("cbits/stbi_shim.c")
            .include("cbits")
            .warnings(false) // stb_image.h is warning-noisy under -Wall
            .compile("stbi_shim");
        println!("cargo:rerun-if-changed=cbits/stbi_shim.c");
        println!("cargo:rerun-if-changed=cbits/stb_image.h");
    }
}
