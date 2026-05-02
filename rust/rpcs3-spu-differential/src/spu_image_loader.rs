//! R5.9e.4 — Build [`SpuProgram`] from a captured `.spuimg` side-file +
//! its companion `spu_image` JSONL event.
//!
//! Wire-format reference: `docs/SPU_TRACE_CAPTURE.md` §
//! "R5.9e.1 — SPU image metadata + side-file layout".
//!
//! The builder is intentionally STRICT — every metadata field is
//! validated, the side-file's bytes are SHA-256-hashed and compared
//! against the event's declared `image_sha256`, and any mismatch is a
//! hard failure. The replay engine (R5.9e.5+, deferred) will consume
//! the resulting `SpuProgram` directly; if this builder accepts it,
//! the bytes-on-disk are exactly the bytes the SPU loaded at
//! thread-creation time.
//!
//! Side-files are NEVER loaded by the parser — only this builder
//! touches them. The parser/transformer pipeline (R5.9a/R5.9b/R5.9e.2)
//! validates JSONL metadata only.

use std::fs;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use crate::trace_fmt::SpuImageEvent;
use crate::SpuProgram;

/// SPU local-store size in bytes (`0x40000` = 256 KiB). Mirror of
/// `SPU_LS_SIZE` in `rpcs3/Emu/Cell/SPUThread.h`.
const SPU_LS_SIZE_U32: u32 = 0x40000;

/// Errors from [`build_spu_program_from_captured_image`].
///
/// Every variant is structured so the caller can present a precise
/// diagnostic — none of these failure modes are recoverable in-place;
/// the caller should surface them to the user.
#[derive(Debug, Clone, PartialEq)]
pub enum SpuProgramBuildError {
    /// The `.spuimg` side-file did not exist (or was not a regular
    /// file) at the requested path. Most common cause: the JSONL
    /// trace was moved without its sibling `.images/` directory.
    ImageFileMissing { path: PathBuf },
    /// The `.spuimg` file existed but I/O failed during read. The
    /// `message` carries the `std::io::Error::to_string()` so callers
    /// can distinguish permission errors, EOF mid-read, etc.
    ImageIo { path: PathBuf, message: String },
    /// The bytes on disk hashed to a different SHA-256 than the JSONL
    /// `image_sha256` field claimed. Either the side-file was
    /// corrupted/edited after capture, OR the JSONL was tampered.
    /// Either case violates the content-addressed contract — refuse
    /// to build the program.
    ImageHashMismatch { expected: String, actual: String },
    /// The `.spuimg` file's byte length differs from the JSONL
    /// `size` field. Signal of partial write OR JSONL declared the
    /// wrong size. Refuse rather than zero-pad / truncate.
    ImageSizeMismatch { expected: u32, actual: usize },
    /// The image's declared size exceeds the SPU local store
    /// (`> 0x40000`). Should have been caught by the R5.9e.2 parser
    /// (`BadImageSize`); kept here as defense-in-depth in case the
    /// builder is invoked with a hand-fabricated `SpuImageEvent`.
    ImageTooLarge { size: usize },
    /// `size`, `load_addr`, or `entry_pc` is not 4-byte aligned. SPU
    /// instructions are 4-byte aligned; segments / entry points must
    /// match. The `field` discriminator says which.
    BadImageAlignment { field: &'static str, value: u32 },
    /// `load_addr + size` overflows `u32` OR exceeds the SPU local
    /// store. Defensive against hand-fabricated events whose load
    /// region would extend past LS — same condition the R5.9e.2
    /// parser already catches via `BadImageLoadAddr`, re-checked
    /// here so the builder is self-defending.
    BadImageBounds { load_addr: u32, size: u32 },
    /// `entry_pc` is unaligned OR `>= 0x40000`. Same condition the
    /// parser's `BadImageEntryPc` catches; defense-in-depth.
    BadEntryPc { entry_pc: u32 },
}

impl std::fmt::Display for SpuProgramBuildError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ImageFileMissing { path } => {
                write!(f, "spu_program build error: side-file missing at '{}'", path.display())
            }
            Self::ImageIo { path, message } => {
                write!(f, "spu_program build error: I/O reading '{}': {message}", path.display())
            }
            Self::ImageHashMismatch { expected, actual } => {
                write!(
                    f,
                    "spu_program build error: SHA-256 mismatch — JSONL image_sha256={expected}, on-disk={actual}"
                )
            }
            Self::ImageSizeMismatch { expected, actual } => {
                write!(
                    f,
                    "spu_program build error: size mismatch — JSONL size={expected}, on-disk={actual}"
                )
            }
            Self::ImageTooLarge { size } => {
                write!(f, "spu_program build error: image size {size} exceeds 256 KiB SPU LS")
            }
            Self::BadImageAlignment { field, value } => {
                write!(f, "spu_program build error: {field}=0x{value:x} is not 4-byte aligned")
            }
            Self::BadImageBounds { load_addr, size } => {
                write!(
                    f,
                    "spu_program build error: load_addr=0x{load_addr:x} + size=0x{size:x} overflows or exceeds 256 KiB LS"
                )
            }
            Self::BadEntryPc { entry_pc } => {
                write!(
                    f,
                    "spu_program build error: entry_pc=0x{entry_pc:x} is unaligned or out of LS range"
                )
            }
        }
    }
}

impl std::error::Error for SpuProgramBuildError {}

/// Build a [`SpuProgram`] from a captured `.spuimg` side-file plus its
/// companion `spu_image` event. Validates every metadata field, the
/// side-file's existence and size, and verifies the SHA-256 hash
/// matches `image.image_sha256`. Returns a `SpuProgram` ready for
/// replay (R5.9e.5+ scope, not yet implemented).
///
/// Validation order (cheapest first; fails fast):
/// 1. Metadata: `size`, `load_addr`, `entry_pc` (alignment + bounds).
/// 2. Side-file existence + I/O read.
/// 3. Byte length matches `size`.
/// 4. SHA-256 of bytes matches `image_sha256`.
/// 5. Build `SpuProgram` (single segment at `load_addr`).
///
/// `max_steps` is the replay budget; the builder doesn't use it
/// directly but propagates it into the resulting `SpuProgram`.
pub fn build_spu_program_from_captured_image(
    image_path: impl AsRef<Path>,
    image: &SpuImageEvent,
    max_steps: u64,
) -> Result<SpuProgram, SpuProgramBuildError> {
    // 1. Metadata validation (cheap — no I/O).

    // size: > 0 enforced by parser, but 0-byte image still defensive.
    if image.size == 0 || image.size > SPU_LS_SIZE_U32 {
        return Err(SpuProgramBuildError::ImageTooLarge {
            size: image.size as usize,
        });
    }
    if image.size & 0x3 != 0 {
        return Err(SpuProgramBuildError::BadImageAlignment {
            field: "size",
            value: image.size,
        });
    }
    if image.load_addr & 0x3 != 0 {
        return Err(SpuProgramBuildError::BadImageAlignment {
            field: "load_addr",
            value: image.load_addr,
        });
    }
    let load_end = image
        .load_addr
        .checked_add(image.size)
        .ok_or(SpuProgramBuildError::BadImageBounds {
            load_addr: image.load_addr,
            size: image.size,
        })?;
    if load_end > SPU_LS_SIZE_U32 {
        return Err(SpuProgramBuildError::BadImageBounds {
            load_addr: image.load_addr,
            size: image.size,
        });
    }
    if image.entry_pc & 0x3 != 0 || image.entry_pc >= SPU_LS_SIZE_U32 {
        return Err(SpuProgramBuildError::BadEntryPc {
            entry_pc: image.entry_pc,
        });
    }

    // 2. Side-file existence + I/O.
    let path = image_path.as_ref();
    if !path.is_file() {
        return Err(SpuProgramBuildError::ImageFileMissing {
            path: path.to_path_buf(),
        });
    }
    let bytes = fs::read(path).map_err(|e| SpuProgramBuildError::ImageIo {
        path: path.to_path_buf(),
        message: e.to_string(),
    })?;

    // 3. Length check.
    if bytes.len() != image.size as usize {
        return Err(SpuProgramBuildError::ImageSizeMismatch {
            expected: image.size,
            actual: bytes.len(),
        });
    }

    // 4. SHA-256 verification. The JSONL `image_sha256` is the ground
    // truth; if the on-disk bytes don't hash to the same value, the
    // content-addressed contract is broken — refuse.
    let mut hasher = Sha256::new();
    hasher.update(&bytes);
    let digest = hasher.finalize();
    let actual_hex: String = digest.iter().map(|b| format!("{b:02x}")).collect();
    if actual_hex != image.image_sha256 {
        return Err(SpuProgramBuildError::ImageHashMismatch {
            expected: image.image_sha256.clone(),
            actual: actual_hex,
        });
    }

    // 5. Construct SpuProgram. Single segment at `load_addr`; entry_pc
    // and max_steps from the event + caller. The replay engine (when
    // it lands) will call `program.validate()` itself — we don't
    // duplicate that here.
    //
    // Set PS3 lv2 SPU thread initial GPR state so the replay matches
    // captured behaviour. The lv2 kernel `sys_spu_thread_group_start`
    // path sets gpr[1] preferred slot (top 32 bits of u128) to
    // 0x3FFF0 (top of 256 KiB LS minus a 16-byte stack reserve);
    // r3..r6 are populated from sysSpuThreadArgument but our
    // synthetic / homebrew fixtures call sysSpuThreadArgumentInitialize
    // (which zeros them), so the default zero from
    // SpuThread::new is fine for those.
    const PS3_INITIAL_SP: u128 = 0x0003_FFF0_u128 << 96;
    Ok(SpuProgram::new(image.entry_pc, max_steps)
        .with_segment(image.load_addr, bytes)
        .with_initial_gpr(1, PS3_INITIAL_SP))
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use sha2::{Digest, Sha256};
    use tempfile::TempDir;

    use crate::trace_fmt::{CapturedSide, SpuImageEvent};

    use super::{build_spu_program_from_captured_image, SpuProgramBuildError};

    /// Build a synthetic 4 KiB SPU image: 1024 instructions of `nop`
    /// (encoded as `0x40_20_00_00` big-endian, the SPU lnop). Returns
    /// the bytes + their SHA-256 (lowercase hex).
    fn synthetic_image_bytes(size: u32) -> (Vec<u8>, String) {
        // Use a deterministic byte pattern (NOT real SPU code; the
        // builder doesn't disassemble — it only validates length,
        // alignment, and hash). Pattern: counting bytes mod 256.
        let bytes: Vec<u8> = (0..size as usize).map(|i| (i & 0xFF) as u8).collect();
        let mut h = Sha256::new();
        h.update(&bytes);
        let digest = h.finalize();
        let hex: String = digest.iter().map(|b| format!("{b:02x}")).collect();
        (bytes, hex)
    }

    /// Write `bytes` to `dir/<sha>.spuimg` and return the path.
    fn write_image(dir: &TempDir, sha: &str, bytes: &[u8]) -> std::path::PathBuf {
        let path = dir.path().join(format!("{sha}.spuimg"));
        let mut f = std::fs::File::create(&path).expect("create temp .spuimg");
        f.write_all(bytes).expect("write temp .spuimg");
        f.flush().expect("flush temp .spuimg");
        path
    }

    fn make_event(target_spu: u32, sha: &str, size: u32, load_addr: u32, entry_pc: u32) -> SpuImageEvent {
        SpuImageEvent {
            seq: 0,
            side: CapturedSide::Spu,
            target_spu,
            image_sha256: sha.to_owned(),
            load_addr,
            size,
            entry_pc,
        }
    }

    #[test]
    fn builder_accepts_valid_synthetic_image() {
        let dir = TempDir::new().expect("temp dir");
        let (bytes, sha) = synthetic_image_bytes(4096);
        let path = write_image(&dir, &sha, &bytes);
        let event = make_event(1, &sha, 4096, 0, 0x100);

        let prog = build_spu_program_from_captured_image(&path, &event, 1_000_000)
            .expect("valid image must build");

        assert_eq!(prog.entry_pc, 0x100);
        assert_eq!(prog.max_steps, 1_000_000);
        assert_eq!(prog.segments.len(), 1);
        assert_eq!(prog.segments[0].lsa, 0);
        assert_eq!(prog.segments[0].data.len(), 4096);
    }

    #[test]
    fn builder_rejects_missing_file() {
        let dir = TempDir::new().expect("temp dir");
        let (_, sha) = synthetic_image_bytes(4096);
        let missing_path = dir.path().join(format!("{sha}.spuimg"));
        let event = make_event(1, &sha, 4096, 0, 0);

        let err = build_spu_program_from_captured_image(&missing_path, &event, 1)
            .expect_err("missing file must error");
        match err {
            SpuProgramBuildError::ImageFileMissing { path } => {
                assert_eq!(path, missing_path);
            }
            other => panic!("expected ImageFileMissing, got {other:?}"),
        }
    }

    #[test]
    fn builder_rejects_hash_mismatch() {
        let dir = TempDir::new().expect("temp dir");
        let (bytes, real_sha) = synthetic_image_bytes(4096);
        let path = write_image(&dir, &real_sha, &bytes);

        // Event claims a DIFFERENT sha than the one the bytes actually
        // hash to. Hand-fabricated lying-event scenario.
        let bogus_sha = "0".repeat(64);
        let event = make_event(1, &bogus_sha, 4096, 0, 0);

        let err = build_spu_program_from_captured_image(&path, &event, 1)
            .expect_err("hash mismatch must error");
        match err {
            SpuProgramBuildError::ImageHashMismatch { expected, actual } => {
                assert_eq!(expected, bogus_sha);
                assert_eq!(actual, real_sha);
            }
            other => panic!("expected ImageHashMismatch, got {other:?}"),
        }
    }

    #[test]
    fn builder_rejects_size_mismatch() {
        let dir = TempDir::new().expect("temp dir");
        // Write 4096 bytes; event claims 8192.
        let (bytes, sha) = synthetic_image_bytes(4096);
        let path = write_image(&dir, &sha, &bytes);
        let event = make_event(1, &sha, 8192, 0, 0);

        let err = build_spu_program_from_captured_image(&path, &event, 1)
            .expect_err("size mismatch must error");
        match err {
            SpuProgramBuildError::ImageSizeMismatch { expected, actual } => {
                assert_eq!(expected, 8192);
                assert_eq!(actual, 4096);
            }
            other => panic!("expected ImageSizeMismatch, got {other:?}"),
        }
    }

    #[test]
    fn builder_rejects_bad_load_addr_alignment() {
        let dir = TempDir::new().expect("temp dir");
        let (bytes, sha) = synthetic_image_bytes(4096);
        let path = write_image(&dir, &sha, &bytes);
        // load_addr=1 — not 4-byte aligned.
        let event = make_event(1, &sha, 4096, 1, 0);

        let err = build_spu_program_from_captured_image(&path, &event, 1)
            .expect_err("unaligned load_addr must error");
        match err {
            SpuProgramBuildError::BadImageAlignment { field, value } => {
                assert_eq!(field, "load_addr");
                assert_eq!(value, 1);
            }
            other => panic!("expected BadImageAlignment(load_addr), got {other:?}"),
        }
    }

    #[test]
    fn builder_rejects_bad_bounds() {
        let dir = TempDir::new().expect("temp dir");
        let (bytes, sha) = synthetic_image_bytes(4096);
        let path = write_image(&dir, &sha, &bytes);
        // load_addr near end + size pushes past LS.
        // 0x3FFFC + 0x10 = 0x4000C > 0x40000.
        let event = make_event(1, &sha, 16, 0x3FFFC, 0);
        // Note: bytes are 4096, event size is 16 — but bounds check
        // fires FIRST (before file read).

        let err = build_spu_program_from_captured_image(&path, &event, 1)
            .expect_err("out-of-bounds load_addr+size must error");
        match err {
            SpuProgramBuildError::BadImageBounds { load_addr, size } => {
                assert_eq!(load_addr, 0x3FFFC);
                assert_eq!(size, 16);
            }
            other => panic!("expected BadImageBounds, got {other:?}"),
        }
    }

    #[test]
    fn builder_rejects_bad_entry_pc() {
        let dir = TempDir::new().expect("temp dir");
        let (bytes, sha) = synthetic_image_bytes(4096);
        let path = write_image(&dir, &sha, &bytes);

        // entry_pc unaligned.
        let event_unaligned = make_event(1, &sha, 4096, 0, 1);
        let err1 = build_spu_program_from_captured_image(&path, &event_unaligned, 1)
            .expect_err("unaligned entry_pc must error");
        match err1 {
            SpuProgramBuildError::BadEntryPc { entry_pc } => assert_eq!(entry_pc, 1),
            other => panic!("expected BadEntryPc, got {other:?}"),
        }

        // entry_pc out of LS range.
        let event_oor = make_event(1, &sha, 4096, 0, 0x40000);
        let err2 = build_spu_program_from_captured_image(&path, &event_oor, 1)
            .expect_err("out-of-range entry_pc must error");
        match err2 {
            SpuProgramBuildError::BadEntryPc { entry_pc } => assert_eq!(entry_pc, 0x40000),
            other => panic!("expected BadEntryPc, got {other:?}"),
        }
    }

    #[test]
    fn builder_places_image_at_load_addr() {
        let dir = TempDir::new().expect("temp dir");
        let (bytes, sha) = synthetic_image_bytes(0x1000);
        let path = write_image(&dir, &sha, &bytes);
        // Place image at LS offset 0x1000 (mid-LS).
        let event = make_event(1, &sha, 0x1000, 0x1000, 0x1000);

        let prog = build_spu_program_from_captured_image(&path, &event, 5)
            .expect("valid mid-LS image must build");

        assert_eq!(prog.segments.len(), 1, "exactly one segment");
        assert_eq!(prog.segments[0].lsa, 0x1000, "segment lsa equals load_addr");
        assert_eq!(prog.segments[0].data, bytes, "segment data equals captured bytes");
        assert_eq!(prog.entry_pc, 0x1000);
        assert_eq!(prog.max_steps, 5);

        // Sanity: the produced program validates against the existing
        // SPU LS-bounds check.
        prog.validate().expect("built program must validate");
    }
}
