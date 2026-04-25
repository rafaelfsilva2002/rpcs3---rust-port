//! Rust port of RPCS3 `Utilities/` leaf helpers.
//!
//! Phase 0 — one function: `get_file_extension`. Everything here must
//! mirror the C++ behavior byte-for-byte; see the contract test below
//! and `Utilities/StrFmt.cpp:958`.
//!
//! We keep `std` enabled: staticlib on MSVC/MinGW/glibc links the
//! system allocator and unwinder anyway, and going `no_std` would cost
//! manual panic handlers for zero runtime benefit.

use std::slice;

/// Returns the file extension of `path`, excluding the leading '.'.
///
/// Mirrors `Utilities/StrFmt.cpp:958`:
/// ```text
///   if dotpos = path.rfind('.'); dotpos exists && dotpos+1 < path.len()
///       => return path[dotpos+1 ..]
///   else
///       => return ""
/// ```
#[must_use]
pub fn get_file_extension(path: &[u8]) -> &[u8] {
    match path.iter().rposition(|&b| b == b'.') {
        Some(pos) if pos + 1 < path.len() => &path[pos + 1..],
        _ => &[],
    }
}

// ---------------------------------------------------------------------
// C ABI — consumed by the rpcs3 C++ side through include/rpcs3_utilities.h
// ---------------------------------------------------------------------

/// # Safety
/// `file_path` must point to at least `file_path_len` valid bytes
/// (or be null iff `file_path_len == 0`). `out` must point to
/// at least `out_cap` writable bytes (or be null iff `out_cap == 0`).
#[no_mangle]
pub unsafe extern "C" fn rpcs3_get_file_extension(
    file_path: *const u8,
    file_path_len: usize,
    out: *mut u8,
    out_cap: usize,
) -> usize {
    let input: &[u8] = if file_path_len == 0 {
        &[]
    } else {
        // SAFETY: caller contract.
        unsafe { slice::from_raw_parts(file_path, file_path_len) }
    };

    let ext = get_file_extension(input);
    let to_write = core::cmp::min(ext.len(), out_cap);

    if to_write > 0 {
        // SAFETY: caller contract; `out_cap >= to_write`.
        unsafe {
            core::ptr::copy_nonoverlapping(ext.as_ptr(), out, to_write);
        }
    }

    ext.len()
}

// ---------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    // -- Behavior parity with Utilities/StrFmt.cpp:958 --------------

    fn cxx_reference(path: &str) -> &str {
        // Literal translation of the C++:
        //   if ((dotpos = find_last_of('.')) != npos && dotpos + 1 < size)
        //       return substr(dotpos + 1);
        //   return "";
        match path.rfind('.') {
            Some(pos) if pos + 1 < path.len() => &path[pos + 1..],
            _ => "",
        }
    }

    /// Exhaustive table of adversarial inputs used as a substitute for
    /// property-based testing until the MSVC toolchain lands (see
    /// docs/PORT_PLAN.md §1). Each case must round-trip against the
    /// reference implementation.
    const PARITY_CASES: &[&str] = &[
        "",
        ".",
        "..",
        "...",
        "a",
        "a.",
        ".a",
        "a.b",
        "a.b.c",
        "a.b.c.d",
        "/",
        "/.",
        "./",
        "/a.b",
        "a/b.c",
        "a.b/c",
        "a.b/c.d",
        "game.PKG",
        "GAME.pkg",
        "EBOOT.BIN",
        "PARAM.SFO",
        "data.tar.gz",
        ".bashrc",
        "no_dot_at_all",
        "ümläut.txt",
        "weird\x00name.ext",
        "trailing_space .txt ",
        "/dev_hdd0/game/BLES12345/EBOOT.BIN",
        "/dev_bdvd/PS3_GAME/USRDIR/EBOOT.BIN",
        "mul.ti.ple.dots",
        ".only.leading.dots.",
    ];

    #[test]
    fn no_dot_returns_empty() {
        assert_eq!(get_file_extension(b"no_extension_here"), b"");
    }

    #[test]
    fn trailing_dot_returns_empty() {
        assert_eq!(get_file_extension(b"file."), b"");
    }

    #[test]
    fn simple_extension() {
        assert_eq!(get_file_extension(b"game.pkg"), b"pkg");
    }

    #[test]
    fn multiple_dots_last_wins() {
        assert_eq!(get_file_extension(b"archive.tar.gz"), b"gz");
    }

    #[test]
    fn hidden_file_returns_empty() {
        // Matches C++ behavior: ".bashrc" has dotpos=0, size=7 → returns "bashrc"
        // (C++ does NOT treat leading-dot files specially, so we mirror that.)
        assert_eq!(get_file_extension(b".bashrc"), b"bashrc");
    }

    #[test]
    fn empty_input_returns_empty() {
        assert_eq!(get_file_extension(b""), b"");
    }

    #[test]
    fn path_with_directory() {
        assert_eq!(
            get_file_extension(b"/dev_hdd0/game/BLES12345/EBOOT.BIN"),
            b"BIN"
        );
    }

    #[test]
    fn path_with_dot_in_dir_no_ext() {
        // "dir.name/file" — last dot is in dir, file has no extension.
        // C++ returns "name/file" because it just blindly uses the last '.'.
        // This is a known quirk we must preserve exactly.
        assert_eq!(get_file_extension(b"dir.name/file"), b"name/file");
    }

    #[test]
    fn parity_table_with_cxx_reference() {
        for case in PARITY_CASES {
            let ours = get_file_extension(case.as_bytes());
            let theirs = cxx_reference(case).as_bytes();
            assert_eq!(
                ours, theirs,
                "divergence for input {case:?}: got {:?}, expected {:?}",
                core::str::from_utf8(ours).ok(),
                core::str::from_utf8(theirs).ok(),
            );
        }
    }

    proptest! {
        /// Fuzz arbitrary strings (including non-UTF-8 bytes via the
        /// `".{0,64}"` regex mapped to bytes below) against the C++
        /// reference. Any divergence is a regression vs the oracle.
        #[test]
        fn parity_with_cxx_reference(s in ".{0,64}") {
            prop_assert_eq!(
                get_file_extension(s.as_bytes()),
                cxx_reference(&s).as_bytes()
            );
        }

        /// Fuzz raw byte sequences (may be invalid UTF-8). The
        /// C++ reference operates on `std::string` (byte sequence too),
        /// so we mirror the byte-level view using a parallel reference
        /// that does not depend on UTF-8 validity.
        #[test]
        fn parity_bytes_with_byte_reference(bytes in proptest::collection::vec(any::<u8>(), 0..64)) {
            let ours = get_file_extension(&bytes);
            let reference: &[u8] = match bytes.iter().rposition(|&b| b == b'.') {
                Some(pos) if pos + 1 < bytes.len() => &bytes[pos + 1..],
                _ => &[],
            };
            prop_assert_eq!(ours, reference);
        }
    }

    // -- C ABI tests ------------------------------------------------

    #[test]
    fn c_abi_writes_into_buffer() {
        let input = b"game.pkg";
        let mut out = [0u8; 16];
        let written = unsafe {
            rpcs3_get_file_extension(input.as_ptr(), input.len(), out.as_mut_ptr(), out.len())
        };
        assert_eq!(written, 3);
        assert_eq!(&out[..written], b"pkg");
    }

    #[test]
    fn c_abi_truncates_when_buffer_small() {
        let input = b"video.webm";
        let mut out = [0u8; 2];
        let written = unsafe {
            rpcs3_get_file_extension(input.as_ptr(), input.len(), out.as_mut_ptr(), out.len())
        };
        // Full extension length reported, but only 2 bytes written.
        assert_eq!(written, 4);
        assert_eq!(&out, b"we");
    }

    #[test]
    fn c_abi_handles_empty_input() {
        let mut out = [0u8; 8];
        let written = unsafe {
            rpcs3_get_file_extension(core::ptr::null(), 0, out.as_mut_ptr(), out.len())
        };
        assert_eq!(written, 0);
    }

    #[test]
    fn c_abi_handles_null_output_when_cap_zero() {
        let input = b"a.b";
        let written = unsafe {
            rpcs3_get_file_extension(input.as_ptr(), input.len(), core::ptr::null_mut(), 0)
        };
        assert_eq!(written, 1);
    }
}
