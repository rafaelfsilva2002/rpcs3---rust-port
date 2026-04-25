//! `rpcs3-crypto` — Rust port of `rpcs3/Crypto/{aes,sha1}.h`.
//!
//! The C++ code embeds PolarSSL (a fork of mbedTLS). This crate
//! provides byte-identical outputs via the audited RustCrypto crates
//! (`aes`, `sha1`, `hmac`) — cryptographic primitives are deterministic
//! by specification, so there is no "flavor drift" to worry about.
//!
//! ## What is covered now
//!
//! | PolarSSL symbol                | Rust equivalent                       |
//! |--------------------------------|---------------------------------------|
//! | `aes_setkey_enc/dec`           | `AesKey::new_128/192/256`             |
//! | `aes_crypt_ecb(..., input[16])`| `AesKey::encrypt_block/decrypt_block` |
//! | `aes_crypt_cbc(...)`           | `aes_cbc_encrypt/decrypt_inplace`     |
//! | `sha1(input, len, out[20])`    | `sha1::digest`                        |
//! | `sha1_starts/update/finish`    | `Sha1Ctx` (streaming)                 |
//! | `sha1_hmac(key, ..., out[20])` | `hmac_sha1::digest`                   |
//!
//! ## Added in Wave 1.b (2026-04-21)
//!
//! * `aes_ctr_xor` — AES-CTR stream cipher, byte-level XOR; used by PKG
//!   segment decryption and EDAT block mode.
//! * `aes_cmac` — RFC 4493 CMAC-AES-128; used by NPDRM KLIC derivation.
//! * `md5_digest` — one-shot MD5 for legacy PS3 integrity checks.
//! * `sha256_digest` — one-shot SHA-256 for PKG digest validation.
//!
//! ## Adiados para Onda 2
//!
//! * `aes_crypt_cfb128` — CFB mode; rarely used by RPCS3 SELF paths.
//! * ECC para assinaturas (`ec.cpp`) — verificação opcional; atrás das
//!   crates `k256`/`p256` quando relevante.

use aes::cipher::{BlockDecrypt, BlockEncrypt, KeyInit};
use aes::{Aes128, Aes192, Aes256};
use cipher::{generic_array::GenericArray, StreamCipher};
use cmac::Cmac;
use hmac::{Hmac, Mac};
use md5::Md5;
use sha1::{Digest, Sha1};
use sha2::Sha256;

// =====================================================================
// Errors — modelled loosely after POLARSSL_ERR_AES_INVALID_*
// =====================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    /// Key length is not 128, 192 or 256 bits.
    InvalidKeyLength,
    /// CBC input length is not a multiple of the 16-byte block size.
    InvalidInputLength,
}

impl core::fmt::Display for Error {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Error::InvalidKeyLength => f.write_str("invalid AES key length (must be 16/24/32 bytes)"),
            Error::InvalidInputLength => f.write_str("CBC input length must be multiple of 16"),
        }
    }
}

impl std::error::Error for Error {}

// =====================================================================
// AES key schedule wrapper
// =====================================================================

/// Owns an AES key schedule. Matches PolarSSL's `aes_context` in role
/// but not in layout (we hide the round-key memory).
pub enum AesKey {
    Aes128(Aes128),
    Aes192(Aes192),
    Aes256(Aes256),
}

impl AesKey {
    pub fn new(key: &[u8]) -> Result<Self, Error> {
        match key.len() {
            16 => Ok(Self::Aes128(Aes128::new_from_slice(key).unwrap())),
            24 => Ok(Self::Aes192(Aes192::new_from_slice(key).unwrap())),
            32 => Ok(Self::Aes256(Aes256::new_from_slice(key).unwrap())),
            _ => Err(Error::InvalidKeyLength),
        }
    }

    #[inline]
    pub fn encrypt_block(&self, block: &mut [u8; 16]) {
        let ga = GenericArray::from_mut_slice(block);
        match self {
            Self::Aes128(c) => c.encrypt_block(ga),
            Self::Aes192(c) => c.encrypt_block(ga),
            Self::Aes256(c) => c.encrypt_block(ga),
        }
    }

    #[inline]
    pub fn decrypt_block(&self, block: &mut [u8; 16]) {
        let ga = GenericArray::from_mut_slice(block);
        match self {
            Self::Aes128(c) => c.decrypt_block(ga),
            Self::Aes192(c) => c.decrypt_block(ga),
            Self::Aes256(c) => c.decrypt_block(ga),
        }
    }
}

// =====================================================================
// AES-CBC — manual implementation to preserve PolarSSL IV-update
// =====================================================================

/// CBC decrypt in place. Mirrors
/// `aes_crypt_cbc(ctx, AES_DECRYPT, len, iv[16], in, out)` from
/// `rpcs3/Crypto/aes.h:113`:
///   * `data.len()` must be a multiple of 16.
///   * `iv` is **mutated** — on return it holds the last ciphertext
///     block, so chained calls produce the same output as PolarSSL.
pub fn aes_cbc_decrypt_inplace(
    key: &AesKey,
    iv: &mut [u8; 16],
    data: &mut [u8],
) -> Result<(), Error> {
    if !data.len().is_multiple_of(16) {
        return Err(Error::InvalidInputLength);
    }

    let mut prev = *iv;
    for chunk in data.chunks_exact_mut(16) {
        let mut block = [0u8; 16];
        block.copy_from_slice(chunk);
        let cipher_text = block;
        key.decrypt_block(&mut block);
        for i in 0..16 {
            chunk[i] = block[i] ^ prev[i];
        }
        prev = cipher_text;
    }

    *iv = prev;
    Ok(())
}

/// CBC encrypt in place. Mirrors
/// `aes_crypt_cbc(ctx, AES_ENCRYPT, ...)`: on return `iv` is the last
/// emitted ciphertext block.
pub fn aes_cbc_encrypt_inplace(
    key: &AesKey,
    iv: &mut [u8; 16],
    data: &mut [u8],
) -> Result<(), Error> {
    if !data.len().is_multiple_of(16) {
        return Err(Error::InvalidInputLength);
    }

    let mut prev = *iv;
    for chunk in data.chunks_exact_mut(16) {
        let mut block = [0u8; 16];
        for i in 0..16 {
            block[i] = chunk[i] ^ prev[i];
        }
        key.encrypt_block(&mut block);
        chunk.copy_from_slice(&block);
        prev = block;
    }

    *iv = prev;
    Ok(())
}

// =====================================================================
// SHA-1
// =====================================================================

/// Streaming SHA-1 context (mirrors `sha1_starts/update/finish`).
pub struct Sha1Ctx(Sha1);

impl Default for Sha1Ctx {
    fn default() -> Self {
        Self::new()
    }
}

impl Sha1Ctx {
    #[must_use]
    pub fn new() -> Self {
        Self(Sha1::new())
    }

    pub fn update(&mut self, data: &[u8]) {
        self.0.update(data);
    }

    #[must_use]
    pub fn finish(self) -> [u8; 20] {
        self.0.finalize().into()
    }
}

/// One-shot SHA-1, matching `sha1(input, ilen, output[20])`.
#[must_use]
pub fn sha1_digest(input: &[u8]) -> [u8; 20] {
    let mut ctx = Sha1Ctx::new();
    ctx.update(input);
    ctx.finish()
}

// =====================================================================
// HMAC-SHA-1
// =====================================================================

/// One-shot HMAC-SHA-1, matching `sha1_hmac(key, keylen, input, ilen, output[20])`.
#[must_use]
pub fn hmac_sha1(key: &[u8], input: &[u8]) -> [u8; 20] {
    type HmacSha1 = Hmac<Sha1>;
    // Disambiguate: both `cipher::KeyInit` and `hmac::Mac` provide
    // `new_from_slice`. We want the HMAC one (accepts any key length).
    let mut mac = <HmacSha1 as Mac>::new_from_slice(key)
        .expect("HMAC accepts any key length");
    mac.update(input);
    mac.finalize().into_bytes().into()
}

// =====================================================================
// AES-CTR stream cipher
// =====================================================================

/// AES-128 CTR stream cipher, in-place XOR.
///
/// Mirrors `aes_crypt_ctr(...)` from `rpcs3/Crypto/aes.h:167`:
///   * `counter` is the 128-bit nonce+counter block. It is **mutated**
///     so subsequent calls continue the counter stream, matching the
///     PolarSSL behaviour that RPCS3 depends on.
///   * `data` is XORed in place with the keystream.
///
/// AES-CTR is its own inverse: calling this twice with the same key and
/// initial counter restores the plaintext.
pub fn aes_ctr_xor(key: &[u8; 16], counter: &mut [u8; 16], data: &mut [u8]) {
    use ctr::cipher::KeyIvInit;
    type Aes128Ctr = ctr::Ctr128BE<Aes128>;
    let mut cipher = Aes128Ctr::new(key.into(), (&*counter).into());
    cipher.apply_keystream(data);

    // Advance the counter block by ceil(data.len() / 16) to mirror the
    // PolarSSL contract of updating the nonce+counter buffer in place.
    // We do big-endian increment on the last 8 bytes (the "counter"
    // portion of the 128-bit block).
    let full_blocks = (data.len() + 15) / 16;
    for _ in 0..full_blocks {
        for i in (8..16).rev() {
            let (next, wrap) = counter[i].overflowing_add(1);
            counter[i] = next;
            if !wrap {
                break;
            }
        }
    }
}

// =====================================================================
// AES-CMAC
// =====================================================================

/// AES-128 CMAC (RFC 4493).
///
/// Mirrors `aes_cmac(ctx, length, input, output)` from
/// `rpcs3/Crypto/aes.h:175`. Used by NPDRM KLIC derivation in
/// `unedat.cpp`.
#[must_use]
pub fn aes_cmac(key: &[u8; 16], input: &[u8]) -> [u8; 16] {
    let mut mac = <Cmac<Aes128> as Mac>::new_from_slice(key)
        .expect("AES-128 CMAC takes a 16-byte key");
    mac.update(input);
    mac.finalize().into_bytes().into()
}

// =====================================================================
// MD5 (RFC 1321)
// =====================================================================

/// One-shot MD5, matching `md5` in `rpcs3/Crypto/md5.cpp`.
#[must_use]
pub fn md5_digest(input: &[u8]) -> [u8; 16] {
    let mut h = Md5::new();
    h.update(input);
    h.finalize().into()
}

// =====================================================================
// SHA-256 (FIPS-180)
// =====================================================================

/// One-shot SHA-256, matching `sha256` in `rpcs3/Crypto/sha256.cpp`.
#[must_use]
pub fn sha256_digest(input: &[u8]) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update(input);
    h.finalize().into()
}

// =====================================================================
// Tests — Known-Answer Tests from FIPS/NIST/RFC
// =====================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn decode_hex(s: &str) -> Vec<u8> {
        hex::decode(s).expect("test vector hex")
    }

    // -- AES-128 ECB (FIPS-197 Appendix C.1) -------------------------

    #[test]
    fn aes128_ecb_fips197_c1_encrypt() {
        let key = decode_hex("000102030405060708090a0b0c0d0e0f");
        let mut block: [u8; 16] = decode_hex("00112233445566778899aabbccddeeff")
            .try_into()
            .unwrap();
        let k = AesKey::new(&key).unwrap();
        k.encrypt_block(&mut block);
        assert_eq!(hex::encode(block), "69c4e0d86a7b0430d8cdb78070b4c55a");
    }

    #[test]
    fn aes128_ecb_fips197_c1_decrypt_roundtrip() {
        let key = decode_hex("000102030405060708090a0b0c0d0e0f");
        let mut block: [u8; 16] = decode_hex("69c4e0d86a7b0430d8cdb78070b4c55a")
            .try_into()
            .unwrap();
        let k = AesKey::new(&key).unwrap();
        k.decrypt_block(&mut block);
        assert_eq!(hex::encode(block), "00112233445566778899aabbccddeeff");
    }

    // -- AES-192 ECB (FIPS-197 Appendix C.2) -------------------------

    #[test]
    fn aes192_ecb_fips197_c2_encrypt() {
        let key = decode_hex("000102030405060708090a0b0c0d0e0f1011121314151617");
        let mut block: [u8; 16] = decode_hex("00112233445566778899aabbccddeeff")
            .try_into()
            .unwrap();
        let k = AesKey::new(&key).unwrap();
        k.encrypt_block(&mut block);
        assert_eq!(hex::encode(block), "dda97ca4864cdfe06eaf70a0ec0d7191");
    }

    // -- AES-256 ECB (FIPS-197 Appendix C.3) -------------------------

    #[test]
    fn aes256_ecb_fips197_c3_encrypt() {
        let key = decode_hex("000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f");
        let mut block: [u8; 16] = decode_hex("00112233445566778899aabbccddeeff")
            .try_into()
            .unwrap();
        let k = AesKey::new(&key).unwrap();
        k.encrypt_block(&mut block);
        assert_eq!(hex::encode(block), "8ea2b7ca516745bfeafc49904b496089");
    }

    // -- AES-128 CBC (NIST SP 800-38A Appendix F.2.1/F.2.2) ----------

    #[test]
    fn aes128_cbc_encrypt_sp800_38a_f21() {
        let key = decode_hex("2b7e151628aed2a6abf7158809cf4f3c");
        let mut iv: [u8; 16] = decode_hex("000102030405060708090a0b0c0d0e0f")
            .try_into()
            .unwrap();
        let mut data = decode_hex(concat!(
            "6bc1bee22e409f96e93d7e117393172a",
            "ae2d8a571e03ac9c9eb76fac45af8e51",
            "30c81c46a35ce411e5fbc1191a0a52ef",
            "f69f2445df4f9b17ad2b417be66c3710",
        ));

        let k = AesKey::new(&key).unwrap();
        aes_cbc_encrypt_inplace(&k, &mut iv, &mut data).unwrap();

        assert_eq!(
            hex::encode(&data),
            concat!(
                "7649abac8119b246cee98e9b12e9197d",
                "5086cb9b507219ee95db113a917678b2",
                "73bed6b8e3c1743b7116e69e22229516",
                "3ff1caa1681fac09120eca307586e1a7",
            ),
        );
        // PolarSSL returns the last ciphertext block via iv.
        assert_eq!(hex::encode(iv), "3ff1caa1681fac09120eca307586e1a7");
    }

    #[test]
    fn aes128_cbc_decrypt_sp800_38a_f22() {
        let key = decode_hex("2b7e151628aed2a6abf7158809cf4f3c");
        let mut iv: [u8; 16] = decode_hex("000102030405060708090a0b0c0d0e0f")
            .try_into()
            .unwrap();
        let mut data = decode_hex(concat!(
            "7649abac8119b246cee98e9b12e9197d",
            "5086cb9b507219ee95db113a917678b2",
            "73bed6b8e3c1743b7116e69e22229516",
            "3ff1caa1681fac09120eca307586e1a7",
        ));

        let k = AesKey::new(&key).unwrap();
        aes_cbc_decrypt_inplace(&k, &mut iv, &mut data).unwrap();

        assert_eq!(
            hex::encode(&data),
            concat!(
                "6bc1bee22e409f96e93d7e117393172a",
                "ae2d8a571e03ac9c9eb76fac45af8e51",
                "30c81c46a35ce411e5fbc1191a0a52ef",
                "f69f2445df4f9b17ad2b417be66c3710",
            ),
        );
        assert_eq!(hex::encode(iv), "3ff1caa1681fac09120eca307586e1a7");
    }

    #[test]
    fn aes128_cbc_rejects_non_block_aligned_input() {
        let k = AesKey::new(&[0u8; 16]).unwrap();
        let mut iv = [0u8; 16];
        let mut data = vec![0u8; 15];
        assert_eq!(
            aes_cbc_decrypt_inplace(&k, &mut iv, &mut data),
            Err(Error::InvalidInputLength)
        );
    }

    #[test]
    fn aes128_cbc_empty_input_is_noop_and_keeps_iv() {
        let k = AesKey::new(&[0u8; 16]).unwrap();
        let mut iv = [0u8; 16];
        iv[0] = 0x42;
        let original_iv = iv;
        let mut data: [u8; 0] = [];
        aes_cbc_decrypt_inplace(&k, &mut iv, &mut data).unwrap();
        assert_eq!(iv, original_iv);
    }

    #[test]
    fn aes_new_rejects_bad_key_length() {
        assert_eq!(AesKey::new(&[0u8; 15]).err(), Some(Error::InvalidKeyLength));
        assert_eq!(AesKey::new(&[0u8; 20]).err(), Some(Error::InvalidKeyLength));
        assert_eq!(AesKey::new(&[0u8; 33]).err(), Some(Error::InvalidKeyLength));
    }

    // -- SHA-1 (FIPS-180 Appendix A) --------------------------------

    #[test]
    fn sha1_fips180_abc() {
        let digest = sha1_digest(b"abc");
        assert_eq!(hex::encode(digest), "a9993e364706816aba3e25717850c26c9cd0d89d");
    }

    #[test]
    fn sha1_fips180_long_alphabet() {
        let digest = sha1_digest(b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq");
        assert_eq!(hex::encode(digest), "84983e441c3bd26ebaae4aa1f95129e5e54670f1");
    }

    #[test]
    fn sha1_empty_input() {
        let digest = sha1_digest(b"");
        assert_eq!(hex::encode(digest), "da39a3ee5e6b4b0d3255bfef95601890afd80709");
    }

    #[test]
    fn sha1_streaming_matches_oneshot() {
        let data = b"the quick brown fox jumps over the lazy dog";
        let oneshot = sha1_digest(data);

        let mut ctx = Sha1Ctx::new();
        for chunk in data.chunks(7) {
            ctx.update(chunk);
        }
        let streamed = ctx.finish();

        assert_eq!(oneshot, streamed);
    }

    // -- HMAC-SHA-1 (RFC 2202) --------------------------------------

    #[test]
    fn hmac_sha1_rfc2202_case1() {
        let key = [0x0bu8; 20];
        let mac = hmac_sha1(&key, b"Hi There");
        assert_eq!(hex::encode(mac), "b617318655057264e28bc0b6fb378c8ef146be00");
    }

    #[test]
    fn hmac_sha1_rfc2202_case2() {
        let key = b"Jefe";
        let mac = hmac_sha1(key, b"what do ya want for nothing?");
        assert_eq!(hex::encode(mac), "effcdf6ae5eb2fa2d27416d5f184df9c259a7c79");
    }

    #[test]
    fn hmac_sha1_rfc2202_case4() {
        let key: Vec<u8> = (1u8..=25).collect();
        let data = [0xcdu8; 50];
        let mac = hmac_sha1(&key, &data);
        assert_eq!(hex::encode(mac), "4c9007f4026250c6bc8414f9bf50c86c2d7235da");
    }

    // -- AES-128 CTR (NIST SP 800-38A Appendix F.5.1) --------------

    #[test]
    fn aes128_ctr_sp800_38a_f51_encrypt() {
        let key: [u8; 16] = decode_hex("2b7e151628aed2a6abf7158809cf4f3c")
            .try_into()
            .unwrap();
        let mut counter: [u8; 16] = decode_hex("f0f1f2f3f4f5f6f7f8f9fafbfcfdfeff")
            .try_into()
            .unwrap();
        let mut data = decode_hex(concat!(
            "6bc1bee22e409f96e93d7e117393172a",
            "ae2d8a571e03ac9c9eb76fac45af8e51",
            "30c81c46a35ce411e5fbc1191a0a52ef",
            "f69f2445df4f9b17ad2b417be66c3710",
        ));

        aes_ctr_xor(&key, &mut counter, &mut data);

        assert_eq!(
            hex::encode(&data),
            concat!(
                "874d6191b620e3261bef6864990db6ce",
                "9806f66b7970fdff8617187bb9fffdff",
                "5ae4df3edbd5d35e5b4f09020db03eab",
                "1e031dda2fbe03d1792170a0f3009cee",
            ),
        );
    }

    #[test]
    fn aes128_ctr_is_self_inverse() {
        let key: [u8; 16] = [0x42; 16];
        let plain = b"Hello, PS3 world! This is a longer buffer for CTR streaming.".to_vec();

        let mut counter = [0x11u8; 16];
        let mut data = plain.clone();
        aes_ctr_xor(&key, &mut counter, &mut data);
        assert_ne!(data, plain, "ciphertext differs from plaintext");

        // Reset counter and decrypt
        let mut counter = [0x11u8; 16];
        aes_ctr_xor(&key, &mut counter, &mut data);
        assert_eq!(data, plain, "CTR is its own inverse");
    }

    #[test]
    fn aes128_ctr_counter_advances() {
        let key = [0u8; 16];
        let mut counter = [0u8; 16];
        let mut data = [0u8; 32]; // 2 blocks
        aes_ctr_xor(&key, &mut counter, &mut data);
        // Counter should have been incremented by 2 (big-endian on last 8 bytes)
        assert_eq!(counter[15], 2);
        assert_eq!(&counter[..15], &[0u8; 15]);
    }

    // -- AES-128 CMAC (RFC 4493 test vectors) -----------------------

    #[test]
    fn aes128_cmac_rfc4493_empty() {
        let key: [u8; 16] = decode_hex("2b7e151628aed2a6abf7158809cf4f3c")
            .try_into()
            .unwrap();
        let mac = aes_cmac(&key, b"");
        assert_eq!(hex::encode(mac), "bb1d6929e95937287fa37d129b756746");
    }

    #[test]
    fn aes128_cmac_rfc4493_16_bytes() {
        let key: [u8; 16] = decode_hex("2b7e151628aed2a6abf7158809cf4f3c")
            .try_into()
            .unwrap();
        let data = decode_hex("6bc1bee22e409f96e93d7e117393172a");
        let mac = aes_cmac(&key, &data);
        assert_eq!(hex::encode(mac), "070a16b46b4d4144f79bdd9dd04a287c");
    }

    #[test]
    fn aes128_cmac_rfc4493_40_bytes() {
        let key: [u8; 16] = decode_hex("2b7e151628aed2a6abf7158809cf4f3c")
            .try_into()
            .unwrap();
        // RFC 4493 test vector uses 40 bytes (not 48):
        //   block 1: 16 bytes
        //   block 2: 16 bytes
        //   block 3 (partial): 8 bytes
        let data = decode_hex(concat!(
            "6bc1bee22e409f96e93d7e117393172a",
            "ae2d8a571e03ac9c9eb76fac45af8e51",
            "30c81c46a35ce411",
        ));
        assert_eq!(data.len(), 40);
        let mac = aes_cmac(&key, &data);
        assert_eq!(hex::encode(mac), "dfa66747de9ae63030ca32611497c827");
    }

    #[test]
    fn aes128_cmac_rfc4493_64_bytes() {
        let key: [u8; 16] = decode_hex("2b7e151628aed2a6abf7158809cf4f3c")
            .try_into()
            .unwrap();
        let data = decode_hex(concat!(
            "6bc1bee22e409f96e93d7e117393172a",
            "ae2d8a571e03ac9c9eb76fac45af8e51",
            "30c81c46a35ce411e5fbc1191a0a52ef",
            "f69f2445df4f9b17ad2b417be66c3710",
        ));
        let mac = aes_cmac(&key, &data);
        assert_eq!(hex::encode(mac), "51f0bebf7e3b9d92fc49741779363cfe");
    }

    // -- MD5 (RFC 1321 test vectors) --------------------------------

    #[test]
    fn md5_empty_string() {
        assert_eq!(hex::encode(md5_digest(b"")), "d41d8cd98f00b204e9800998ecf8427e");
    }

    #[test]
    fn md5_abc() {
        assert_eq!(hex::encode(md5_digest(b"abc")), "900150983cd24fb0d6963f7d28e17f72");
    }

    #[test]
    fn md5_message_digest() {
        assert_eq!(
            hex::encode(md5_digest(b"message digest")),
            "f96b697d7cb7938d525a2f31aaf161d0"
        );
    }

    #[test]
    fn md5_alphabet() {
        assert_eq!(
            hex::encode(md5_digest(b"abcdefghijklmnopqrstuvwxyz")),
            "c3fcd3d76192e4007dfb496cca67e13b"
        );
    }

    // -- SHA-256 (FIPS-180 test vectors) ----------------------------

    #[test]
    fn sha256_empty_string() {
        assert_eq!(
            hex::encode(sha256_digest(b"")),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn sha256_abc() {
        assert_eq!(
            hex::encode(sha256_digest(b"abc")),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn sha256_long_alphabet() {
        assert_eq!(
            hex::encode(sha256_digest(b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq")),
            "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1"
        );
    }
}
