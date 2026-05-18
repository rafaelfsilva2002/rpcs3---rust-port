//! R6.7 A.3 — DMA `.dmachunk` side-file loader.
//!
//! Wire-format reference: `docs/SPU_DMA_MFC_R6_7_DESIGN.md` § 5
//! ("EA-memory side-file design"). The R6.7 A.1 writer extension
//! emits content-addressed `.dmachunk` files alongside the JSONL
//! trace; the R6.7 A.2 parser validates the metadata reference
//! (`SpuMfcCmdEvent::ea_chunk_sha256`); this module — A.3 — resolves
//! the file on disk, reads the bytes, and hash-verifies them against
//! the SHA-256 declared in the metadata.
//!
//! **Scope of this module (A.3):**
//! - Path resolution (per-trace `<trace>.dma/` first, then the canonical
//!   shared CC0 store at `behavior-freeze/fixtures/spu/dma/`).
//! - Bytes-on-disk read.
//! - Size validation (> 0, ≤ 0x4000, optional `expected_size` match).
//! - SHA-256 verification (bytes hash to the declared sha).
//! - Defensive re-check that the sha string is 64 lowercase hex.
//!
//! **NOT in scope (A.4 / later):**
//! - Copying loaded bytes into SPU local store.
//! - Driving an `MfcReplayState`.
//! - Mutating the transformer policy — the transformer still hard-
//!   rejects MFC traces with `TraceTransformError::UnsupportedDmaInTrace`
//!   (verified by the `loader_does_not_change_transformer_policy`
//!   test below).
//!
//! The loader is intentionally STRICT — every failure mode is a
//! distinct error variant so the eventual A.4 caller can surface a
//! precise diagnostic and the operator can act (re-capture, fix
//! filename casing, regenerate side-file, etc.). Mirrors the
//! `spu_image_loader` style for `.spuimg` side-files.

use std::fs;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

/// MFC transfer hard cap (16 KiB). Larger sizes need DMA list cmds
/// which are out of R6.7 scope. Mirrors the `MFC_DMA_SIZE_MAX`
/// constant in `trace_fmt.rs`; duplicated here so this module stays
/// self-contained and importable without depending on the parser.
pub const DMA_CHUNK_SIZE_MAX: usize = 0x4000;

/// `.dmachunk` filename extension. Used both for path construction
/// (loader) and the optional defensive check that a caller-supplied
/// path actually points at a chunk file.
pub const DMA_CHUNK_EXTENSION: &str = "dmachunk";

/// Errors from [`resolve_dma_chunk_side_file`] and the lower-level
/// helpers it composes. Every variant is structured so the caller
/// can present a precise diagnostic — none of these failure modes
/// are recoverable in-place; the caller (eventually `MfcReplayState`,
/// A.4) should surface them to the user.
#[derive(Debug, Clone, PartialEq)]
pub enum DmaChunkLoadError {
    /// The supplied `sha256` string is not a valid 64-char lowercase-
    /// hex digest. The R6.7 A.2 parser already enforces this at JSONL
    /// parse time (see `TraceParseError::BadDmaSha`), but this loader
    /// re-validates defensively so it can be invoked with hand-built
    /// inputs (tests, ad-hoc tools) without trusting the caller.
    BadDmaSha {
        sha: String,
        reason: &'static str,
    },
    /// Neither the per-trace path nor the canonical path resolved to
    /// a regular file. The error carries both attempted paths so the
    /// operator can fix the more likely cause (often: capture moved
    /// without its sibling `.dma/` directory).
    MissingDmaChunk {
        sha: String,
        per_trace_path: PathBuf,
        canonical_path: PathBuf,
    },
    /// The `.dmachunk` file existed but I/O failed during read. The
    /// `message` carries the `std::io::Error::to_string()` so callers
    /// can distinguish permission errors, EOF mid-read, etc.
    DmaChunkReadFailed {
        path: PathBuf,
        message: String,
    },
    /// The on-disk byte length disagrees with the caller-supplied
    /// `expected_size` (typically the `SpuMfcCmdEvent.size` field).
    /// Refuse rather than zero-pad / truncate.
    DmaChunkSizeMismatch {
        path: PathBuf,
        expected: usize,
        actual: usize,
    },
    /// The bytes on disk hashed to a different SHA-256 than the
    /// declared `sha256`. Either the side-file was corrupted/edited
    /// after capture, OR the JSONL was tampered. Either case violates
    /// the content-addressed contract — refuse to surface the bytes.
    DmaChunkShaMismatch {
        path: PathBuf,
        expected: String,
        actual: String,
    },
    /// The `.dmachunk` file's size exceeds the MFC simple-cmd cap
    /// (`> 0x4000`). Should have been caught by the R6.7 A.2 parser
    /// (`BadDmaSize`); kept here as defense-in-depth in case the
    /// loader is invoked with a hand-fabricated event.
    DmaChunkTooLarge {
        path: PathBuf,
        size: usize,
    },
    /// The `.dmachunk` file is empty (0 bytes). Same defensive check
    /// as the parser's `BadDmaSize { size: 0, .. }` path.
    DmaChunkEmpty {
        path: PathBuf,
    },
}

impl std::fmt::Display for DmaChunkLoadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::BadDmaSha { sha, reason } => {
                write!(f, "dma chunk load error: bad sha256 '{sha}' ({reason})")
            }
            Self::MissingDmaChunk { sha, per_trace_path, canonical_path } => {
                write!(
                    f,
                    "dma chunk load error: side-file for sha256 '{sha}' not found in either \
                     per-trace path '{}' or canonical path '{}'",
                    per_trace_path.display(),
                    canonical_path.display()
                )
            }
            Self::DmaChunkReadFailed { path, message } => {
                write!(
                    f,
                    "dma chunk load error: I/O reading '{}': {message}",
                    path.display()
                )
            }
            Self::DmaChunkSizeMismatch { path, expected, actual } => {
                write!(
                    f,
                    "dma chunk load error: size mismatch for '{}' — expected {expected}, on-disk {actual}",
                    path.display()
                )
            }
            Self::DmaChunkShaMismatch { path, expected, actual } => {
                write!(
                    f,
                    "dma chunk load error: SHA-256 mismatch for '{}' — declared {expected}, on-disk {actual}",
                    path.display()
                )
            }
            Self::DmaChunkTooLarge { path, size } => {
                write!(
                    f,
                    "dma chunk load error: '{}' size {size} exceeds 0x4000 (16 KiB R6.7 cap)",
                    path.display()
                )
            }
            Self::DmaChunkEmpty { path } => {
                write!(
                    f,
                    "dma chunk load error: '{}' is empty (0 bytes)",
                    path.display()
                )
            }
        }
    }
}

impl std::error::Error for DmaChunkLoadError {}

/// Resolve and load the `.dmachunk` side-file for a `spu_mfc_cmd`
/// event. Searches two locations in priority order:
///
/// 1. **Per-trace:** `<trace_path>.dma/<sha>.dmachunk` — what the
///    R6.7 A.1 writer emits on capture. Matches the `.spuimg` /
///    `<trace>.images/` convention.
/// 2. **Canonical:** `<canonical_dma_dir>/<sha>.dmachunk` — the
///    CC0 shared store under `behavior-freeze/fixtures/spu/dma/`.
///    Populated by a separate post-capture workflow step that
///    deduplicates `.dmachunk` files across traces.
///
/// Per-trace takes precedence: a fresh capture's `.dma/` directory is
/// the source of truth for that trace, and the canonical store is
/// only consulted as a fallback for committed CC0 fixtures whose
/// per-trace directory was pruned.
///
/// Validation order (cheapest first; fails fast):
/// 1. `sha256` string shape (defensive re-check).
/// 2. Path resolution (per-trace, then canonical).
/// 3. Read bytes from disk.
/// 4. Empty-file check.
/// 5. Hard-cap check (`size <= 0x4000`).
/// 6. `expected_size` match (if `Some`).
/// 7. SHA-256 of bytes matches `sha256`.
///
/// Returns the loaded bytes verbatim — the caller is responsible for
/// copying them into SPU local store or wherever else (A.4 scope).
pub fn resolve_dma_chunk_side_file(
    trace_path: &Path,
    canonical_dma_dir: &Path,
    sha256: &str,
    expected_size: Option<usize>,
) -> Result<Vec<u8>, DmaChunkLoadError> {
    // 1. Defensive sha shape check — even though the parser already
    // ran. The loader is callable with hand-built inputs.
    validate_sha_string(sha256)?;

    // 2. Path resolution: per-trace first, canonical fallback.
    let per_trace_path = per_trace_dma_chunk_path(trace_path, sha256);
    let canonical_path = canonical_dma_chunk_path(canonical_dma_dir, sha256);
    let resolved = if per_trace_path.is_file() {
        per_trace_path.clone()
    } else if canonical_path.is_file() {
        canonical_path.clone()
    } else {
        return Err(DmaChunkLoadError::MissingDmaChunk {
            sha: sha256.to_owned(),
            per_trace_path,
            canonical_path,
        });
    };

    // 3. I/O read.
    let bytes = fs::read(&resolved).map_err(|e| DmaChunkLoadError::DmaChunkReadFailed {
        path: resolved.clone(),
        message: e.to_string(),
    })?;

    // 4. Empty-file check (the writer never emits 0-byte files; this
    // is defense-in-depth against a stale or corrupted side-file).
    if bytes.is_empty() {
        return Err(DmaChunkLoadError::DmaChunkEmpty { path: resolved });
    }

    // 5. Hard cap.
    if bytes.len() > DMA_CHUNK_SIZE_MAX {
        return Err(DmaChunkLoadError::DmaChunkTooLarge {
            path: resolved,
            size: bytes.len(),
        });
    }

    // 6. Caller-declared size match.
    if let Some(n) = expected_size {
        if bytes.len() != n {
            return Err(DmaChunkLoadError::DmaChunkSizeMismatch {
                path: resolved,
                expected: n,
                actual: bytes.len(),
            });
        }
    }

    // 7. Content-addressed SHA-256 verification. The declared sha
    // is the lookup key AND the integrity check; if the bytes don't
    // hash to it, the contract is broken — refuse.
    let actual_hex = sha256_hex_of(&bytes);
    if actual_hex != sha256 {
        return Err(DmaChunkLoadError::DmaChunkShaMismatch {
            path: resolved,
            expected: sha256.to_owned(),
            actual: actual_hex,
        });
    }

    Ok(bytes)
}

/// Compute `<trace_path>.dma/<sha>.dmachunk`. Public so callers can
/// pre-compute the expected path for diagnostic / pre-flight checks
/// without invoking the full loader. Does NOT verify the path exists.
#[must_use]
pub fn per_trace_dma_chunk_path(trace_path: &Path, sha256: &str) -> PathBuf {
    // `concat` instead of `with_extension` — the writer concatenates
    // ".dma" onto the FULL trace path (including any `.jsonl` suffix),
    // so e.g. `trace.jsonl` → `trace.jsonl.dma/`.
    let mut dir = trace_path.as_os_str().to_owned();
    dir.push(".dma");
    let mut p = PathBuf::from(dir);
    p.push(format!("{sha256}.{DMA_CHUNK_EXTENSION}"));
    p
}

/// Compute `<canonical_dma_dir>/<sha>.dmachunk`. Public for the same
/// reason as [`per_trace_dma_chunk_path`].
#[must_use]
pub fn canonical_dma_chunk_path(canonical_dma_dir: &Path, sha256: &str) -> PathBuf {
    canonical_dma_dir.join(format!("{sha256}.{DMA_CHUNK_EXTENSION}"))
}

/// Defensive SHA-256-string shape check. Returns Ok iff `sha` is
/// exactly 64 ASCII lowercase-hex chars (`[0-9a-f]{64}`). Anything
/// else (length, casing, non-hex) → `BadDmaSha`.
fn validate_sha_string(sha: &str) -> Result<(), DmaChunkLoadError> {
    if sha.len() != 64 {
        return Err(DmaChunkLoadError::BadDmaSha {
            sha: sha.to_owned(),
            reason: "sha256 must be exactly 64 hex chars",
        });
    }
    if !sha.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')) {
        return Err(DmaChunkLoadError::BadDmaSha {
            sha: sha.to_owned(),
            reason: "sha256 must be lowercase [0-9a-f] only (no uppercase, no non-hex)",
        });
    }
    Ok(())
}

/// SHA-256 of `bytes`, lowercase-hex encoded. Mirrors the format the
/// C++ writer emits and the parser expects.
fn sha256_hex_of(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let digest = hasher.finalize();
    digest.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use tempfile::TempDir;

    use super::*;

    /// Synthetic chunk bytes: a deterministic counting pattern so
    /// tests can compute the SHA-256 once and reuse it. Returns
    /// (bytes, lowercase-hex sha256).
    fn synthetic_chunk(size: usize) -> (Vec<u8>, String) {
        let bytes: Vec<u8> = (0..size).map(|i| (i & 0xFF) as u8).collect();
        let hex = sha256_hex_of(&bytes);
        (bytes, hex)
    }

    /// Write `bytes` to `<dir>/<sha>.dmachunk` and return the path.
    fn write_chunk(dir: &Path, sha: &str, bytes: &[u8]) -> PathBuf {
        std::fs::create_dir_all(dir).expect("create chunk dir");
        let path = dir.join(format!("{sha}.dmachunk"));
        let mut f = std::fs::File::create(&path).expect("create temp .dmachunk");
        f.write_all(bytes).expect("write temp .dmachunk");
        f.flush().expect("flush temp .dmachunk");
        path
    }

    /// Build the per-trace `.dma/` directory layout for `<trace_path>`.
    /// Returns the directory path for further file writes.
    fn ensure_per_trace_dma_dir(trace_path: &Path) -> PathBuf {
        let mut dir = trace_path.as_os_str().to_owned();
        dir.push(".dma");
        let dir = PathBuf::from(dir);
        std::fs::create_dir_all(&dir).expect("create per-trace .dma dir");
        dir
    }

    #[test]
    fn load_dma_chunk_from_per_trace_dir() {
        let tmp = TempDir::new().expect("temp dir");
        let trace_path = tmp.path().join("capture.jsonl");
        let canonical = tmp.path().join("canonical");
        std::fs::create_dir_all(&canonical).unwrap();

        let (bytes, sha) = synthetic_chunk(128);
        let dma_dir = ensure_per_trace_dma_dir(&trace_path);
        write_chunk(&dma_dir, &sha, &bytes);

        let loaded = resolve_dma_chunk_side_file(&trace_path, &canonical, &sha, Some(128))
            .expect("per-trace lookup must succeed");
        assert_eq!(loaded, bytes);
    }

    #[test]
    fn load_dma_chunk_from_canonical_dir() {
        let tmp = TempDir::new().expect("temp dir");
        let trace_path = tmp.path().join("capture.jsonl");
        let canonical = tmp.path().join("canonical");
        std::fs::create_dir_all(&canonical).unwrap();

        // Per-trace dir intentionally absent — fallback to canonical.
        let (bytes, sha) = synthetic_chunk(256);
        write_chunk(&canonical, &sha, &bytes);

        let loaded = resolve_dma_chunk_side_file(&trace_path, &canonical, &sha, Some(256))
            .expect("canonical fallback must succeed");
        assert_eq!(loaded, bytes);
    }

    #[test]
    fn per_trace_takes_precedence_over_canonical() {
        let tmp = TempDir::new().expect("temp dir");
        let trace_path = tmp.path().join("capture.jsonl");
        let canonical = tmp.path().join("canonical");
        std::fs::create_dir_all(&canonical).unwrap();

        // Per-trace has the correct bytes; canonical has DIFFERENT
        // bytes whose sha would hash to a totally different value
        // (so if the loader incorrectly picked canonical, the SHA
        // verification would fire — even though the filename is the
        // same as the per-trace one).
        let (per_trace_bytes, sha) = synthetic_chunk(128);
        let dma_dir = ensure_per_trace_dma_dir(&trace_path);
        write_chunk(&dma_dir, &sha, &per_trace_bytes);

        // Canonical file with the same sha-name but tampered contents.
        // The loader MUST pick per-trace and ignore canonical here.
        let tampered = vec![0xAA; 128];
        write_chunk(&canonical, &sha, &tampered);

        let loaded = resolve_dma_chunk_side_file(&trace_path, &canonical, &sha, Some(128))
            .expect("per-trace must take precedence");
        assert_eq!(loaded, per_trace_bytes,
            "per-trace bytes must win — canonical was tampered with same filename");
    }

    #[test]
    fn reject_missing_dma_chunk() {
        let tmp = TempDir::new().expect("temp dir");
        let trace_path = tmp.path().join("capture.jsonl");
        let canonical = tmp.path().join("canonical");
        std::fs::create_dir_all(&canonical).unwrap();

        let sha = "0".repeat(64);
        let err = resolve_dma_chunk_side_file(&trace_path, &canonical, &sha, None)
            .expect_err("no chunk file → MissingDmaChunk");
        match err {
            DmaChunkLoadError::MissingDmaChunk { sha: got_sha, per_trace_path, canonical_path } => {
                assert_eq!(got_sha, sha);
                assert!(per_trace_path.to_string_lossy().contains("capture.jsonl.dma"));
                assert!(canonical_path.starts_with(&canonical));
            }
            other => panic!("expected MissingDmaChunk, got {other:?}"),
        }
    }

    #[test]
    fn reject_bad_sha_string() {
        let tmp = TempDir::new().expect("temp dir");
        let trace_path = tmp.path().join("capture.jsonl");
        let canonical = tmp.path().join("canonical");
        std::fs::create_dir_all(&canonical).unwrap();

        // Too short.
        let err = resolve_dma_chunk_side_file(&trace_path, &canonical, "abc", None)
            .expect_err("short sha → BadDmaSha");
        assert!(matches!(err, DmaChunkLoadError::BadDmaSha { .. }), "got {err:?}");

        // Uppercase.
        let upper = "ABCDEF0123456789".repeat(4);
        let err = resolve_dma_chunk_side_file(&trace_path, &canonical, &upper, None)
            .expect_err("uppercase sha → BadDmaSha");
        assert!(matches!(err, DmaChunkLoadError::BadDmaSha { .. }), "got {err:?}");

        // Non-hex.
        let mut bad = "0123456789abcdef".repeat(4);
        bad.replace_range(0..1, "z");
        let err = resolve_dma_chunk_side_file(&trace_path, &canonical, &bad, None)
            .expect_err("non-hex sha → BadDmaSha");
        assert!(matches!(err, DmaChunkLoadError::BadDmaSha { .. }), "got {err:?}");
    }

    #[test]
    fn reject_empty_dma_chunk() {
        let tmp = TempDir::new().expect("temp dir");
        let trace_path = tmp.path().join("capture.jsonl");
        let canonical = tmp.path().join("canonical");
        std::fs::create_dir_all(&canonical).unwrap();

        // Hand-built empty file under per-trace path. The sha must be
        // valid hex shape so we get past the defensive check and reach
        // the empty-file check. Use the SHA of zero-bytes (well-known
        // value) so the writer would naturally never emit it (the
        // writer drops 0-byte snapshots), proving this is a defensive
        // check against external tampering.
        let empty_sha = sha256_hex_of(&[]);
        let dma_dir = ensure_per_trace_dma_dir(&trace_path);
        let path = dma_dir.join(format!("{empty_sha}.dmachunk"));
        std::fs::File::create(&path).expect("create empty file");

        let err = resolve_dma_chunk_side_file(&trace_path, &canonical, &empty_sha, None)
            .expect_err("empty file → DmaChunkEmpty");
        match err {
            DmaChunkLoadError::DmaChunkEmpty { path: p } => assert_eq!(p, path),
            other => panic!("expected DmaChunkEmpty, got {other:?}"),
        }
    }

    #[test]
    fn reject_too_large_dma_chunk() {
        let tmp = TempDir::new().expect("temp dir");
        let trace_path = tmp.path().join("capture.jsonl");
        let canonical = tmp.path().join("canonical");
        std::fs::create_dir_all(&canonical).unwrap();

        // 0x4001 bytes = 16385 — just above the cap. The bytes hash
        // to a real SHA which we use as the lookup key. Note: the
        // declared sha is the real one; if the loader instead checked
        // size FIRST we'd see DmaChunkTooLarge regardless of the sha.
        let oversized = DMA_CHUNK_SIZE_MAX + 1;
        let (bytes, sha) = synthetic_chunk(oversized);
        let dma_dir = ensure_per_trace_dma_dir(&trace_path);
        write_chunk(&dma_dir, &sha, &bytes);

        let err = resolve_dma_chunk_side_file(&trace_path, &canonical, &sha, None)
            .expect_err("oversized chunk → DmaChunkTooLarge");
        match err {
            DmaChunkLoadError::DmaChunkTooLarge { size, .. } => {
                assert_eq!(size, oversized);
            }
            other => panic!("expected DmaChunkTooLarge, got {other:?}"),
        }
    }

    #[test]
    fn reject_expected_size_mismatch() {
        let tmp = TempDir::new().expect("temp dir");
        let trace_path = tmp.path().join("capture.jsonl");
        let canonical = tmp.path().join("canonical");
        std::fs::create_dir_all(&canonical).unwrap();

        let (bytes, sha) = synthetic_chunk(128);
        let dma_dir = ensure_per_trace_dma_dir(&trace_path);
        write_chunk(&dma_dir, &sha, &bytes);

        // Caller declares 256 but the file is 128.
        let err = resolve_dma_chunk_side_file(&trace_path, &canonical, &sha, Some(256))
            .expect_err("size mismatch → DmaChunkSizeMismatch");
        match err {
            DmaChunkLoadError::DmaChunkSizeMismatch { expected, actual, .. } => {
                assert_eq!(expected, 256);
                assert_eq!(actual, 128);
            }
            other => panic!("expected DmaChunkSizeMismatch, got {other:?}"),
        }
    }

    #[test]
    fn reject_content_sha_mismatch() {
        let tmp = TempDir::new().expect("temp dir");
        let trace_path = tmp.path().join("capture.jsonl");
        let canonical = tmp.path().join("canonical");
        std::fs::create_dir_all(&canonical).unwrap();

        // Write bytes whose real sha is `real_sha`, but file the
        // request under a different (well-formed) sha. The loader
        // must read the bytes, hash them, and reject because the
        // hash doesn't match the declared lookup key.
        let (bytes, real_sha) = synthetic_chunk(128);
        let bogus_sha = "f".repeat(64);
        let dma_dir = ensure_per_trace_dma_dir(&trace_path);
        // Write the file under the BOGUS name — this is the tampered
        // case: filename claims one sha, contents hash to another.
        let path = dma_dir.join(format!("{bogus_sha}.dmachunk"));
        std::fs::File::create(&path).unwrap().write_all(&bytes).unwrap();

        let err = resolve_dma_chunk_side_file(&trace_path, &canonical, &bogus_sha, None)
            .expect_err("content sha mismatch → DmaChunkShaMismatch");
        match err {
            DmaChunkLoadError::DmaChunkShaMismatch { expected, actual, .. } => {
                assert_eq!(expected, bogus_sha);
                assert_eq!(actual, real_sha);
            }
            other => panic!("expected DmaChunkShaMismatch, got {other:?}"),
        }
    }

    /// R6.7 C.5 — after Phase C wired MFC channels into the executor
    /// + the transformer started accepting MFC traces as context, the
    /// loader's relationship to the transformer policy is the
    /// inverse of the A.3 invariant: this test now confirms the
    /// transformer ACCEPTS the same trace the A.3 loader would
    /// resolve `.dmachunk` for. The loader itself is still
    /// transformer-orthogonal — its job is bytes-on-disk, not trace
    /// transformation.
    #[test]
    fn loader_orthogonal_to_transformer_policy_post_phase_c() {
        use crate::trace_fmt::{captured_events_to_traces_per_spu, parse_jsonl_trace};

        let valid_sha = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
        let jsonl = format!(
            r#"
{{"seq":0,"side":"spu","kind":"spu_wrch","pc":256,"channel":21,"value":64,"would_stall":false,"target_spu":1}}
{{"seq":1,"side":"spu","kind":"spu_mfc_cmd","target_spu":1,"pc":256,"cmd":64,"tag":3,"size":128,"lsa":0,"eah":0,"eal":4096,"ea_chunk_sha256":"{valid_sha}"}}
{{"seq":2,"side":"spu","kind":"mfc_dma_complete","target_spu":1,"tag":3,"transferred_bytes":128}}
{{"seq":3,"side":"spu","kind":"spu_stop","pc":260,"stop_code":1,"target_spu":1}}
{{"seq":4,"side":"spu","kind":"final_state","gpr_lane_zero":[],"channels":{{"in_mbox":null,"out_mbox":null,"out_intr_mbox":null,"snr1":0,"snr2":0}},"target_spu":1}}
"#
        );
        let events = parse_jsonl_trace(&jsonl).expect("parser still accepts valid MFC sequence");
        // The transformer now succeeds (post-C.5). The loader is
        // not consulted by the transformer at any point — the
        // pre-replay layer (`mfc_replay::apply_mfc_dma_pre_replay`)
        // is where the loader is invoked.
        let _per_spu = captured_events_to_traces_per_spu(&events)
            .expect("transformer accepts valid MFC trace post Phase C");
    }

    /// Sanity: the public path-builder helpers produce the exact paths
    /// the loader uses internally. Used by callers who want to pre-flight
    /// a path before invoking the loader, or to surface "expected file"
    /// in their own error messages.
    #[test]
    fn path_builders_match_loader_lookup_paths() {
        let trace = Path::new("/tmp/some/capture.jsonl");
        let canonical = Path::new("/repo/behavior-freeze/fixtures/spu/dma");
        let sha = "0123456789abcdef".repeat(4);

        let pt = per_trace_dma_chunk_path(trace, &sha);
        assert_eq!(
            pt,
            PathBuf::from("/tmp/some/capture.jsonl.dma").join(format!("{sha}.dmachunk"))
        );

        let cn = canonical_dma_chunk_path(canonical, &sha);
        assert_eq!(
            cn,
            PathBuf::from("/repo/behavior-freeze/fixtures/spu/dma").join(format!("{sha}.dmachunk"))
        );
    }
}
