//! `rpcs3-hle-sys-prx-for-user` — user-mode runtime helper library.
//!
//! Ports the broadly-used subset of
//! `rpcs3/Emu/Cell/Modules/sysPrxForUser.cpp`. This PRX exposes
//! helpers that sit between pure syscalls and the full CRT: random
//! bytes, console I/O, exit-handler registration, console/media ID
//! getters.
//!
//! ## Entry points covered
//!
//! | Function                         | Rust wrapper                    |
//! |----------------------------------|---------------------------------|
//! | `sys_get_random_number`          | [`sys_get_random_number`]       |
//! | `sys_process_is_stack`           | [`sys_process_is_stack`]        |
//! | `_sys_process_atexitspawn`       | [`sys_process_atexitspawn`]     |
//! | `_sys_process_at_Exitspawn`      | [`sys_process_at_exitspawn`]    |
//! | `sys_process_get_paramsfo`       | [`sys_process_get_paramsfo`]    |
//! | `console_getc`                   | [`console_getc`]                |
//! | `console_putc`                   | [`console_putc`]                |
//! | `console_write`                  | [`console_write`]               |
//! | `sys_get_console_id`             | [`sys_get_console_id`]          |
//! | `sys_get_bd_media_id`            | [`sys_get_bd_media_id`]         |

use rpcs3_emu_types::CellError;

// =====================================================================
// Constants
// =====================================================================

/// Default stack range — anything outside this is `not-stack`.
/// PS3 user stacks are allocated in the [0x00000000, 0x10000000) window
/// with the high bit flipped (main stack near 0x10000000 top).
pub const STACK_BASE_HI: u32 = 0x1000_0000;

/// Length of `CellGameParamSFO`-style name buffer.
pub const PARAMSFO_BUF_LEN: usize = 20;

// =====================================================================
// Randomness
// =====================================================================

pub trait RandomSource {
    /// Fill `dst` with random bytes.
    fn fill(&mut self, dst: &mut [u8]);
}

/// Deterministic source built on linear-congruential generator —
/// exposed so tests can pin specific sequences.
#[derive(Debug, Clone)]
pub struct SeededRandom {
    pub state: u64,
}

impl SeededRandom {
    #[must_use]
    pub fn new(seed: u64) -> Self { Self { state: seed } }
}

impl RandomSource for SeededRandom {
    fn fill(&mut self, dst: &mut [u8]) {
        for chunk in dst.chunks_mut(8) {
            // xorshift64*.
            self.state = self.state.wrapping_add(0x9E37_79B9_7F4A_7C15);
            let mut z = self.state;
            z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
            z ^= z >> 31;
            let bytes = z.to_be_bytes();
            let take = chunk.len();
            chunk.copy_from_slice(&bytes[..take]);
        }
    }
}

/// `sys_get_random_number(buffer, size)` — fill `dst`. Caps at 0x1000
/// bytes per call (matches firmware).
#[must_use]
pub fn sys_get_random_number<R: RandomSource + ?Sized>(
    rng: &mut R,
    dst: &mut [u8],
) -> Result<(), CellError> {
    if dst.is_empty() || dst.len() > 0x1000 {
        return Err(CellError::EINVAL);
    }
    rng.fill(dst);
    Ok(())
}

// =====================================================================
// Process helpers
// =====================================================================

/// `sys_process_is_stack(p)` — returns 1 if the address is inside the
/// main stack window, 0 otherwise. Matches firmware heuristic.
#[must_use]
pub fn sys_process_is_stack(addr: u32) -> i32 {
    // C++ impl: any address strictly below STACK_BASE_HI is not stack.
    // Main stack lives at [0xD000_0000, 0xE000_0000) on PS3.
    if addr >= 0xD000_0000 && addr < 0xE000_0000 { 1 } else { 0 }
}

/// `_sys_process_atexitspawn(fn_addr)` — register post-exit callback.
/// Real impl appends to an internal list. Our port tracks the slot.
#[derive(Debug, Default)]
pub struct ProcessHooks {
    pub atexitspawn: Option<u32>,
    pub at_exitspawn: Option<u32>,
}

pub fn sys_process_atexitspawn(hooks: &mut ProcessHooks, func_addr: u32) {
    hooks.atexitspawn = Some(func_addr);
}
pub fn sys_process_at_exitspawn(hooks: &mut ProcessHooks, func_addr: u32) {
    hooks.at_exitspawn = Some(func_addr);
}

/// `sys_process_get_paramsfo(buffer)` — writes the 20-byte PARAM.SFO
/// title id into the provided buffer. Needs a backend.
pub trait ParamSfoSource {
    fn title_id(&self) -> &[u8; PARAMSFO_BUF_LEN];
}

#[must_use]
pub fn sys_process_get_paramsfo<P: ParamSfoSource + ?Sized>(
    src: &P,
    buf: &mut [u8; PARAMSFO_BUF_LEN],
) -> Result<(), CellError> {
    *buf = *src.title_id();
    Ok(())
}

// =====================================================================
// Console I/O
// =====================================================================

pub trait ConsoleIO {
    fn getc(&mut self) -> Option<u8>;
    fn putc(&mut self, byte: u8);
    fn write(&mut self, bytes: &[u8]);
}

/// In-memory console useful for tests.
#[derive(Debug, Default)]
pub struct TestConsole {
    pub input_buffer: Vec<u8>,
    pub output: Vec<u8>,
}

impl ConsoleIO for TestConsole {
    fn getc(&mut self) -> Option<u8> {
        if self.input_buffer.is_empty() {
            None
        } else {
            Some(self.input_buffer.remove(0))
        }
    }
    fn putc(&mut self, byte: u8) { self.output.push(byte); }
    fn write(&mut self, bytes: &[u8]) { self.output.extend_from_slice(bytes); }
}

#[must_use]
pub fn console_getc<C: ConsoleIO + ?Sized>(c: &mut C) -> Option<u8> {
    c.getc()
}

pub fn console_putc<C: ConsoleIO + ?Sized>(c: &mut C, byte: u8) {
    c.putc(byte)
}

/// `console_write(data, len)`.
#[must_use]
pub fn console_write<C: ConsoleIO + ?Sized>(
    c: &mut C,
    bytes: &[u8],
) -> Result<u32, CellError> {
    if bytes.len() > 0x1_0000 {
        return Err(CellError::EINVAL);
    }
    c.write(bytes);
    Ok(bytes.len() as u32)
}

// =====================================================================
// Console / media IDs
// =====================================================================

/// 16-byte console ID.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConsoleId(pub [u8; 16]);

/// 16-byte media ID (Blu-ray disc).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MediaId(pub [u8; 16]);

pub trait HardwareIds {
    fn console_id(&self) -> ConsoleId;
    fn bd_media_id(&self) -> Option<MediaId>;
}

pub struct StubHardwareIds;
impl HardwareIds for StubHardwareIds {
    fn console_id(&self) -> ConsoleId {
        let mut id = [0u8; 16];
        id[..4].copy_from_slice(&[0xDE, 0xAD, 0xBE, 0xEF]);
        ConsoleId(id)
    }
    fn bd_media_id(&self) -> Option<MediaId> { None }
}

#[must_use]
pub fn sys_get_console_id<H: HardwareIds + ?Sized>(h: &H) -> ConsoleId {
    h.console_id()
}

#[must_use]
pub fn sys_get_bd_media_id<H: HardwareIds + ?Sized>(h: &H) -> Result<MediaId, CellError> {
    h.bd_media_id().ok_or(CellError::ESRCH)
}

// =====================================================================
// Tests
// =====================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stack_addresses_return_1_in_window() {
        assert_eq!(sys_process_is_stack(0xD000_0000), 1);
        assert_eq!(sys_process_is_stack(0xD800_0000), 1);
        assert_eq!(sys_process_is_stack(0xDFFF_FFFF), 1);
    }

    #[test]
    fn non_stack_addresses_return_0() {
        assert_eq!(sys_process_is_stack(0x1000), 0);
        assert_eq!(sys_process_is_stack(0xC000_0000), 0);
        assert_eq!(sys_process_is_stack(0xE000_0000), 0);
        assert_eq!(sys_process_is_stack(0), 0);
    }

    // --- randomness ----------------------------------------------

    #[test]
    fn get_random_number_fills_buffer() {
        let mut rng = SeededRandom::new(0xCAFE_BABE);
        let mut buf = [0u8; 32];
        sys_get_random_number(&mut rng, &mut buf).unwrap();
        assert!(buf.iter().any(|&b| b != 0), "must not leave all zeros");
    }

    #[test]
    fn get_random_number_seeded_is_deterministic() {
        let mut buf_a = [0u8; 16];
        let mut buf_b = [0u8; 16];
        sys_get_random_number(&mut SeededRandom::new(42), &mut buf_a).unwrap();
        sys_get_random_number(&mut SeededRandom::new(42), &mut buf_b).unwrap();
        assert_eq!(buf_a, buf_b);
    }

    #[test]
    fn get_random_number_rejects_empty_buffer() {
        let mut rng = SeededRandom::new(0);
        let mut empty = [0u8; 0];
        assert_eq!(
            sys_get_random_number(&mut rng, &mut empty).unwrap_err(),
            CellError::EINVAL,
        );
    }

    #[test]
    fn get_random_number_rejects_over_4K_buffer() {
        let mut rng = SeededRandom::new(0);
        let mut big = vec![0u8; 0x1001];
        assert_eq!(
            sys_get_random_number(&mut rng, &mut big).unwrap_err(),
            CellError::EINVAL,
        );
    }

    // --- process hooks -------------------------------------------

    #[test]
    fn atexit_hooks_round_trip() {
        let mut h = ProcessHooks::default();
        sys_process_atexitspawn(&mut h, 0x10_0000);
        sys_process_at_exitspawn(&mut h, 0x20_0000);
        assert_eq!(h.atexitspawn, Some(0x10_0000));
        assert_eq!(h.at_exitspawn, Some(0x20_0000));
    }

    // --- param.sfo -----------------------------------------------

    struct StubSfo([u8; 20]);
    impl ParamSfoSource for StubSfo {
        fn title_id(&self) -> &[u8; 20] { &self.0 }
    }

    #[test]
    fn get_paramsfo_copies_title_id() {
        let mut src = [b'X'; 20];
        src[..9].copy_from_slice(b"BLUS12345");
        let sfo = StubSfo(src);
        let mut buf = [0u8; 20];
        sys_process_get_paramsfo(&sfo, &mut buf).unwrap();
        assert_eq!(&buf[..9], b"BLUS12345");
    }

    // --- console I/O ---------------------------------------------

    #[test]
    fn console_putc_writes_to_output() {
        let mut c = TestConsole::default();
        console_putc(&mut c, b'H');
        console_putc(&mut c, b'i');
        assert_eq!(c.output, b"Hi");
    }

    #[test]
    fn console_getc_drains_input_queue() {
        let mut c = TestConsole { input_buffer: b"AB".to_vec(), ..Default::default() };
        assert_eq!(console_getc(&mut c), Some(b'A'));
        assert_eq!(console_getc(&mut c), Some(b'B'));
        assert_eq!(console_getc(&mut c), None);
    }

    #[test]
    fn console_write_returns_bytes_written() {
        let mut c = TestConsole::default();
        let written = console_write(&mut c, b"hello").unwrap();
        assert_eq!(written, 5);
        assert_eq!(c.output, b"hello");
    }

    #[test]
    fn console_write_rejects_huge_buffer() {
        let mut c = TestConsole::default();
        let big = vec![0u8; 0x1_0001];
        assert_eq!(
            console_write(&mut c, &big).unwrap_err(),
            CellError::EINVAL,
        );
    }

    // --- hardware ids --------------------------------------------

    #[test]
    fn stub_console_id_returns_deadbeef_prefix() {
        let id = sys_get_console_id(&StubHardwareIds);
        assert_eq!(&id.0[..4], &[0xDE, 0xAD, 0xBE, 0xEF]);
    }

    #[test]
    fn stub_bd_media_id_returns_esrch_when_missing() {
        assert_eq!(
            sys_get_bd_media_id(&StubHardwareIds).unwrap_err(),
            CellError::ESRCH,
        );
    }

    struct DiscInSlot;
    impl HardwareIds for DiscInSlot {
        fn console_id(&self) -> ConsoleId { ConsoleId([0; 16]) }
        fn bd_media_id(&self) -> Option<MediaId> { Some(MediaId([0xAB; 16])) }
    }

    #[test]
    fn bd_media_id_when_present_returns_bytes() {
        let id = sys_get_bd_media_id(&DiscInSlot).unwrap();
        assert_eq!(id.0, [0xAB; 16]);
    }
}
