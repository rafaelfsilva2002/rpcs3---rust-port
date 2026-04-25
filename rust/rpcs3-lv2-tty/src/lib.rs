//! Rust port of `rpcs3/Emu/Cell/lv2/sys_tty.cpp` — PS3 LV2 TTY syscalls
//! (2 entries: read/write, 16 channels, 205 lines C++).
//!
//! Channels 0..=15. SYS_TTYP_USER1=3 → channels < 3 are system channels
//! (logged with warning). Channels 16+ → CELL_EINVAL.
//!
//! `read`: requires `debug_console_mode` enabled (else CELL_EIO). Pulls
//! from per-channel input queue, splitting at `\n` or `len` boundary.
//! `write`: appends to TTY output, returns bytes written via `pwritelen`.
//!
//! `no_std` + `alloc`. Single dep: `rpcs3-emu-types`.

#![no_std]

extern crate alloc;

use alloc::collections::VecDeque;
use alloc::string::String;
use alloc::vec::Vec;
use rpcs3_emu_types::CellError;

pub const MODULE_NAME: &str = "sys_tty";

pub const REGISTERED_ENTRY_POINTS: &[&str] = &["sys_tty_read", "sys_tty_write"];

/// `SYS_TTYP_USER1=3` — channels below this are system channels (cpp:32-35).
pub const SYS_TTYP_USER1: i32 = 3;
/// Total channels (0..=15 = 16 channels).
pub const NUM_CHANNELS: usize = 16;

pub const CELL_EINVAL: CellError = CellError(0x8001_0002);
pub const CELL_EIO: CellError = CellError(0x8001_000C);
pub const CELL_EFAULT: CellError = CellError(0x8001_000D);

#[derive(Debug)]
pub struct SysTty {
    /// Per-channel input queue (FIFO of strings).
    pub input_queues: Vec<VecDeque<String>>,
    /// Captured stdout/stderr per channel.
    pub captured_output: Vec<String>,
    pub debug_console_mode: bool,
    pub total_bytes_written: u64,
    pub system_channel_warnings: u64,
    pub read_calls: u64,
    pub write_calls: u64,
}

impl Default for SysTty {
    fn default() -> Self {
        let mut input_queues = Vec::with_capacity(NUM_CHANNELS);
        let mut captured_output = Vec::with_capacity(NUM_CHANNELS);
        for _ in 0..NUM_CHANNELS {
            input_queues.push(VecDeque::new());
            captured_output.push(String::new());
        }
        Self {
            input_queues,
            captured_output,
            debug_console_mode: false,
            total_bytes_written: 0,
            system_channel_warnings: 0,
            read_calls: 0,
            write_calls: 0,
        }
    }
}

impl SysTty {
    pub fn new() -> Self {
        Self::default()
    }

    /// Inject input for a channel (test/scaffold helper).
    pub fn push_input(&mut self, ch: usize, data: &str) {
        if ch < NUM_CHANNELS {
            self.input_queues[ch].push_back(data.into());
        }
    }

    /// `sys_tty_read(ch, buf, len, preadlen)` cpp:18-89.
    /// Validation order: debug mode → ch range/null buf → preadlen non-null →
    /// reads up to first `\n` or `len` chars from front of channel queue.
    pub fn read(
        &mut self,
        ch: i32,
        buf_present: bool,
        len: u32,
        preadlen_out: Option<&mut u32>,
    ) -> Result<String, CellError> {
        self.read_calls = self.read_calls.saturating_add(1);
        if !self.debug_console_mode {
            return Err(CELL_EIO);
        }
        if !(0..=15).contains(&ch) || !buf_present {
            return Err(CELL_EINVAL);
        }
        if ch < SYS_TTYP_USER1 {
            self.system_channel_warnings = self.system_channel_warnings.saturating_add(1);
        }
        let mut chars_to_read = 0usize;
        let mut read_str = String::new();
        if len > 0 {
            let queue = &mut self.input_queues[ch as usize];
            if let Some(input) = queue.front_mut() {
                let nl_pos = input.find('\n');
                let limit = match nl_pos {
                    Some(p) => p.min(len as usize),
                    None => input.len().min(len as usize),
                };
                chars_to_read = limit;
                read_str = input[..limit].into();
                // cpp:65 — drop consumed prefix.
                let rest: String = input[limit..].into();
                *input = rest;
                if input.is_empty() {
                    queue.pop_front();
                }
            }
        }
        let plen = preadlen_out.ok_or(CELL_EFAULT)?;
        *plen = chars_to_read as u32;
        Ok(read_str)
    }

    /// `sys_tty_write(ch, buf, len, pwritelen)` cpp:93-205. Simplified to
    /// append the payload to the per-channel captured output.
    pub fn write(
        &mut self,
        ch: i32,
        payload: Option<&str>,
        len: u32,
        pwritelen_out: Option<&mut u32>,
    ) -> Result<(), CellError> {
        self.write_calls = self.write_calls.saturating_add(1);
        if !(0..=15).contains(&ch) {
            return Err(CELL_EINVAL);
        }
        let payload = payload.ok_or(CELL_EFAULT)?;
        let plen = pwritelen_out.ok_or(CELL_EFAULT)?;
        let actual = (len as usize).min(payload.len());
        let slice = &payload[..actual];
        self.captured_output[ch as usize].push_str(slice);
        self.total_bytes_written = self.total_bytes_written.saturating_add(actual as u64);
        *plen = actual as u32;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn module_and_entries() {
        assert_eq!(MODULE_NAME, "sys_tty");
        assert_eq!(REGISTERED_ENTRY_POINTS.len(), 2);
    }

    #[test]
    fn constants_byte_exact() {
        assert_eq!(SYS_TTYP_USER1, 3);
        assert_eq!(NUM_CHANNELS, 16);
    }

    #[test]
    fn read_without_debug_console_eio() {
        let mut t = SysTty::new();
        let mut p = 0u32;
        assert_eq!(t.read(3, true, 16, Some(&mut p)), Err(CELL_EIO));
    }

    #[test]
    fn read_invalid_channel_einval() {
        let mut t = SysTty::new();
        t.debug_console_mode = true;
        let mut p = 0u32;
        assert_eq!(t.read(-1, true, 16, Some(&mut p)), Err(CELL_EINVAL));
        assert_eq!(t.read(16, true, 16, Some(&mut p)), Err(CELL_EINVAL));
    }

    #[test]
    fn read_null_buf_einval() {
        let mut t = SysTty::new();
        t.debug_console_mode = true;
        let mut p = 0u32;
        assert_eq!(t.read(3, false, 16, Some(&mut p)), Err(CELL_EINVAL));
    }

    #[test]
    fn read_null_preadlen_efault() {
        let mut t = SysTty::new();
        t.debug_console_mode = true;
        assert_eq!(t.read(3, true, 16, None), Err(CELL_EFAULT));
    }

    #[test]
    fn read_pulls_from_queue_until_newline() {
        let mut t = SysTty::new();
        t.debug_console_mode = true;
        t.push_input(3, "hello\nworld");
        let mut p = 0u32;
        let s = t.read(3, true, 100, Some(&mut p)).unwrap();
        assert_eq!(s, "hello");
        assert_eq!(p, 5);
    }

    #[test]
    fn read_truncates_to_len() {
        let mut t = SysTty::new();
        t.debug_console_mode = true;
        t.push_input(3, "abcdefgh");
        let mut p = 0u32;
        let s = t.read(3, true, 3, Some(&mut p)).unwrap();
        assert_eq!(s, "abc");
        assert_eq!(p, 3);
    }

    #[test]
    fn read_pops_when_consumed() {
        let mut t = SysTty::new();
        t.debug_console_mode = true;
        t.push_input(5, "xy");
        let mut p = 0u32;
        let s = t.read(5, true, 100, Some(&mut p)).unwrap();
        assert_eq!(s, "xy");
        assert!(t.input_queues[5].is_empty());
    }

    #[test]
    fn read_system_channel_warns() {
        let mut t = SysTty::new();
        t.debug_console_mode = true;
        t.push_input(0, "sys");
        let mut p = 0u32;
        let _ = t.read(0, true, 16, Some(&mut p));
        assert_eq!(t.system_channel_warnings, 1);
        // ch=3 = USER1, no warning.
        t.push_input(3, "user");
        let _ = t.read(3, true, 16, Some(&mut p));
        assert_eq!(t.system_channel_warnings, 1); // unchanged
    }

    #[test]
    fn write_basic() {
        let mut t = SysTty::new();
        let mut p = 0u32;
        t.write(3, Some("hello"), 5, Some(&mut p)).unwrap();
        assert_eq!(p, 5);
        assert_eq!(t.captured_output[3], "hello");
    }

    #[test]
    fn write_truncates_to_len() {
        let mut t = SysTty::new();
        let mut p = 0u32;
        t.write(3, Some("hello world"), 5, Some(&mut p)).unwrap();
        assert_eq!(p, 5);
        assert_eq!(t.captured_output[3], "hello");
    }

    #[test]
    fn write_invalid_channel() {
        let mut t = SysTty::new();
        let mut p = 0u32;
        assert_eq!(t.write(99, Some("x"), 1, Some(&mut p)), Err(CELL_EINVAL));
    }

    #[test]
    fn write_accumulates_bytes() {
        let mut t = SysTty::new();
        let mut p = 0u32;
        t.write(3, Some("abc"), 3, Some(&mut p)).unwrap();
        t.write(3, Some("def"), 3, Some(&mut p)).unwrap();
        assert_eq!(t.captured_output[3], "abcdef");
        assert_eq!(t.total_bytes_written, 6);
    }

    #[test]
    fn full_tty_lifecycle_smoke() {
        let mut t = SysTty::new();
        t.debug_console_mode = true;

        // User input arrives — note: cpp:65 `input.substr(chars_to_read, ...)`
        // does NOT consume the trailing newline, so consumers must push lines
        // separately for a clean read sequence (preserves upstream behaviour).
        t.push_input(3, "user typed this");
        t.push_input(3, "more");

        // Read first entry.
        let mut p = 0u32;
        let s = t.read(3, true, 100, Some(&mut p)).unwrap();
        assert_eq!(s, "user typed this");

        // Second read consumes "more".
        let s2 = t.read(3, true, 100, Some(&mut p)).unwrap();
        assert_eq!(s2, "more");

        // Queue exhausted now.
        let s3 = t.read(3, true, 100, Some(&mut p)).unwrap();
        assert_eq!(s3, "");

        // Game writes to TTY.
        t.write(3, Some("response"), 8, Some(&mut p)).unwrap();
        assert_eq!(t.captured_output[3], "response");
    }

    #[test]
    fn read_leaves_trailing_newline_in_queue() {
        // Preserves cpp:65 quirk: `input.substr(chars_to_read, ...)` keeps the
        // newline at the front of the remaining input. Next read finds \n at
        // position 0 and reads 0 chars.
        let mut t = SysTty::new();
        t.debug_console_mode = true;
        t.push_input(3, "abc\nxyz");
        let mut p = 0u32;
        let s = t.read(3, true, 100, Some(&mut p)).unwrap();
        assert_eq!(s, "abc");
        // \nxyz still in queue — next read finds \n at pos 0 → 0 chars.
        let s2 = t.read(3, true, 100, Some(&mut p)).unwrap();
        assert_eq!(s2, "");
        assert_eq!(p, 0);
    }
}
