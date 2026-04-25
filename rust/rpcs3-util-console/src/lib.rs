//! `rpcs3-util-console` — Rust port of `rpcs3/util/console.cpp` + `.h`.
//!
//! Windows-specific console attachment bits plus a cross-platform
//! `output_stderr(str, with_endline)` helper. We freeze the bit flags
//! and the wants-newline decision — the actual Win32 AttachConsole/
//! AllocConsole side is platform-specific and lives in the frontend.
//!
//! Frozen:
//!
//! - `ConsoleStream` bit flags: `std_out=1, std_err=2, std_in=4`.
//! - `attach_console_early_exit(stream)` — returns true when `stream == 0`
//!   (cpp:15..18 early return).
//! - `wants_stream(bits, which)` helper.

/// `console_stream` enum from `console.h:7..11`. Bit flags that select
/// which standard streams to (re)attach.
pub const CONSOLE_STREAM_STD_OUT: u32 = 0x01;
pub const CONSOLE_STREAM_STD_ERR: u32 = 0x02;
pub const CONSOLE_STREAM_STD_IN: u32 = 0x04;

/// Returns `true` when `attach_console(stream, ...)` should early-return
/// immediately (cpp:15..18 — `stream == 0` means "no streams requested").
#[must_use]
pub const fn attach_console_early_exit(stream: u32) -> bool {
    stream == 0
}

/// Whether `stream_bits` requests `which` stream to be reattached.
#[must_use]
pub const fn wants_stream(stream_bits: u32, which: u32) -> bool {
    (stream_bits & which) != 0
}

/// Compose a stream bitfield from individual flags — useful for tests
/// and frontends that build it programmatically.
#[must_use]
pub const fn stream_bits(out: bool, err: bool, r#in: bool) -> u32 {
    let mut bits = 0;
    if out {
        bits |= CONSOLE_STREAM_STD_OUT;
    }
    if err {
        bits |= CONSOLE_STREAM_STD_ERR;
    }
    if r#in {
        bits |= CONSOLE_STREAM_STD_IN;
    }
    bits
}

/// Format a string for `output_stderr(str, with_endline=true)`. The cpp
/// version emits the string then appends `"\n"`; both halves go to the
/// same stream. We return the composed output so the caller can write
/// it in one go.
#[must_use]
pub fn format_output_stderr(msg: &str, with_endline: bool) -> String {
    if with_endline {
        format!("{msg}\n")
    } else {
        msg.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stream_bits_are_powers_of_two() {
        assert_eq!(CONSOLE_STREAM_STD_OUT, 1);
        assert_eq!(CONSOLE_STREAM_STD_ERR, 2);
        assert_eq!(CONSOLE_STREAM_STD_IN, 4);
    }

    #[test]
    fn early_exit_when_stream_zero() {
        assert!(attach_console_early_exit(0));
        assert!(!attach_console_early_exit(1));
        assert!(!attach_console_early_exit(7));
    }

    #[test]
    fn wants_stream_individual_bits() {
        let all = CONSOLE_STREAM_STD_OUT | CONSOLE_STREAM_STD_ERR | CONSOLE_STREAM_STD_IN;
        assert!(wants_stream(all, CONSOLE_STREAM_STD_OUT));
        assert!(wants_stream(all, CONSOLE_STREAM_STD_ERR));
        assert!(wants_stream(all, CONSOLE_STREAM_STD_IN));

        assert!(wants_stream(CONSOLE_STREAM_STD_OUT, CONSOLE_STREAM_STD_OUT));
        assert!(!wants_stream(CONSOLE_STREAM_STD_OUT, CONSOLE_STREAM_STD_ERR));
        assert!(!wants_stream(0, CONSOLE_STREAM_STD_OUT));
    }

    #[test]
    fn compose_stream_bits() {
        assert_eq!(stream_bits(true, false, false), CONSOLE_STREAM_STD_OUT);
        assert_eq!(stream_bits(false, true, false), CONSOLE_STREAM_STD_ERR);
        assert_eq!(stream_bits(false, false, true), CONSOLE_STREAM_STD_IN);
        assert_eq!(stream_bits(true, true, true), 7);
        assert_eq!(stream_bits(false, false, false), 0);
    }

    #[test]
    fn format_output_stderr_with_endline() {
        assert_eq!(format_output_stderr("hello", true), "hello\n");
        assert_eq!(format_output_stderr("hello", false), "hello");
        assert_eq!(format_output_stderr("", true), "\n");
        assert_eq!(format_output_stderr("", false), "");
    }
}
