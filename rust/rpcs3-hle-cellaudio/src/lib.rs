//! `rpcs3-hle-cellaudio` — audio-port HLE layer.
//!
//! Ports the game-facing entry points from
//! `rpcs3/Emu/Cell/Modules/cellAudio.cpp`. The actual audio backend
//! (XAudio2, PulseAudio, FAudio…) is out of scope. This crate owns
//! the port-state FSM (`Close` → `Ready` → `Run`), the 256-sample
//! block ring buffer, and returns `CELL_AUDIO_ERROR_*` codes byte-
//! exact vs the C++ header.
//!
//! ## Frozen constants (from `cellAudio.h`)
//!
//! * `BLOCK_SAMPLES = 256` — one ring-buffer slot.
//! * Block counts: 8 / 16 / 32.
//! * Channels: 2 or 8.
//! * Max 8 ports simultaneously.
//! * Status: `CLOSE = 0x1010`, `READY = 1`, `RUN = 2`.
//!
//! ## Syscalls covered
//!
//! | HLE function                      | Rust wrapper                  |
//! |-----------------------------------|-------------------------------|
//! | `cellAudioInit`                   | [`cell_audio_init`]           |
//! | `cellAudioQuit`                   | [`cell_audio_quit`]           |
//! | `cellAudioPortOpen`               | [`cell_audio_port_open`]      |
//! | `cellAudioPortClose`              | [`cell_audio_port_close`]     |
//! | `cellAudioPortStart`              | [`cell_audio_port_start`]     |
//! | `cellAudioPortStop`               | [`cell_audio_port_stop`]      |
//! | `cellAudioGetPortConfig`          | [`cell_audio_get_port_config`]|
//! | `cellAudioAddData`                | [`cell_audio_add_data`]       |

use rpcs3_emu_types::CellError;

// =====================================================================
// Frozen constants
// =====================================================================

pub mod errors {
    use rpcs3_emu_types::CellError;

    pub const ALREADY_INIT: CellError = CellError(0x8031_0701);
    pub const AUDIOSYSTEM: CellError = CellError(0x8031_0702);
    pub const NOT_INIT: CellError = CellError(0x8031_0703);
    pub const PARAM: CellError = CellError(0x8031_0704);
    pub const PORT_FULL: CellError = CellError(0x8031_0705);
    pub const PORT_ALREADY_RUN: CellError = CellError(0x8031_0706);
    pub const PORT_NOT_OPEN: CellError = CellError(0x8031_0707);
    pub const PORT_NOT_RUN: CellError = CellError(0x8031_0708);
    pub const TRANS_EVENT: CellError = CellError(0x8031_0709);
    pub const PORT_OPEN: CellError = CellError(0x8031_070A);
    pub const SHAREDMEMORY: CellError = CellError(0x8031_070B);
    pub const MUTEX: CellError = CellError(0x8031_070C);
    pub const EVENT_QUEUE: CellError = CellError(0x8031_070D);
    pub const AUDIOSYSTEM_NOT_FOUND: CellError = CellError(0x8031_070E);
    pub const TAG_NOT_FOUND: CellError = CellError(0x8031_070F);
}

pub const BLOCK_SAMPLES: usize = 256;
pub const BLOCKS_8: u32 = 8;
pub const BLOCKS_16: u32 = 16;
pub const BLOCKS_32: u32 = 32;

pub const PORT_2CH: u32 = 2;
pub const PORT_8CH: u32 = 8;

pub const STATUS_CLOSE: u32 = 0x1010;
pub const STATUS_READY: u32 = 1;
pub const STATUS_RUN: u32 = 2;

/// Maximum concurrent audio ports.
pub const MAX_AUDIO_PORTS: usize = 8;

// =====================================================================
// Data model
// =====================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PortParam {
    pub num_channels: u32,
    pub num_blocks: u32,
    pub attr: u64,
    /// Level multiplier in f32 bit pattern (passed through; games often
    /// leave it at 1.0 == 0x3F80_0000).
    pub level_bits: u32,
}

impl Default for PortParam {
    fn default() -> Self {
        Self { num_channels: PORT_2CH, num_blocks: BLOCKS_8, attr: 0, level_bits: 0x3F80_0000 }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PortStatus {
    Close,
    Ready,
    Run,
}

impl PortStatus {
    #[must_use]
    pub fn as_u32(self) -> u32 {
        match self {
            Self::Close => STATUS_CLOSE,
            Self::Ready => STATUS_READY,
            Self::Run => STATUS_RUN,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PortConfig {
    pub status: u32,
    pub num_channels: u32,
    pub num_blocks: u32,
    pub port_size: u32,
}

// =====================================================================
// Backend trait — sink side (speakers/headphones output)
// =====================================================================

/// The host-side audio sink. Real runtime implements this over
/// XAudio/PulseAudio; tests use [`TestAudioSink`].
pub trait AudioSink {
    /// Consume one 256-sample block of f32 interleaved samples.
    /// `num_channels` tells the backend how to unpack the stream.
    fn push_block(&mut self, num_channels: u32, samples: &[f32]);
}

// =====================================================================
// Manager state
// =====================================================================

#[derive(Debug)]
pub struct AudioManager {
    initialized: bool,
    ports: [Option<Port>; MAX_AUDIO_PORTS],
}

#[derive(Debug)]
struct Port {
    param: PortParam,
    status: PortStatus,
    /// Ring buffer of blocks — `param.num_blocks` entries allocated;
    /// each block is `BLOCK_SAMPLES * num_channels` f32 samples.
    blocks: Vec<Vec<f32>>,
    /// Next slot to write into (wraps).
    write_index: u32,
    /// How many blocks are currently queued (0..=num_blocks).
    blocks_pending: u32,
}

impl Default for AudioManager {
    fn default() -> Self {
        Self {
            initialized: false,
            ports: Default::default(),
        }
    }
}

// =====================================================================
// Validation helpers
// =====================================================================

fn ensure_init(m: &AudioManager) -> Result<(), CellError> {
    if m.initialized { Ok(()) } else { Err(errors::NOT_INIT) }
}

fn validate_port_param(p: &PortParam) -> Result<(), CellError> {
    match p.num_channels {
        PORT_2CH | PORT_8CH => {}
        _ => return Err(errors::PARAM),
    }
    match p.num_blocks {
        BLOCKS_8 | BLOCKS_16 | BLOCKS_32 => Ok(()),
        _ => Err(errors::PARAM),
    }
}

fn check_port_no(port_no: usize) -> Result<(), CellError> {
    if port_no < MAX_AUDIO_PORTS { Ok(()) } else { Err(errors::PARAM) }
}

// =====================================================================
// Syscalls
// =====================================================================

/// `cellAudioInit()` — must be the first call.
#[must_use]
pub fn cell_audio_init(m: &mut AudioManager) -> Result<(), CellError> {
    if m.initialized {
        return Err(errors::ALREADY_INIT);
    }
    m.initialized = true;
    Ok(())
}

/// `cellAudioQuit()` — tear everything down.
#[must_use]
pub fn cell_audio_quit(m: &mut AudioManager) -> Result<(), CellError> {
    ensure_init(m)?;
    *m = AudioManager::default();
    Ok(())
}

/// `cellAudioPortOpen(param, port_out)` — allocate the first free slot.
#[must_use]
pub fn cell_audio_port_open(
    m: &mut AudioManager,
    param: PortParam,
) -> Result<u32, CellError> {
    ensure_init(m)?;
    validate_port_param(&param)?;

    let free_slot = m
        .ports
        .iter()
        .position(|p| p.is_none())
        .ok_or(errors::PORT_FULL)?;

    let samples_per_block = BLOCK_SAMPLES * param.num_channels as usize;
    let blocks = (0..param.num_blocks)
        .map(|_| vec![0.0f32; samples_per_block])
        .collect();

    m.ports[free_slot] = Some(Port {
        param,
        status: PortStatus::Ready,
        blocks,
        write_index: 0,
        blocks_pending: 0,
    });
    Ok(free_slot as u32)
}

fn port_mut<'a>(m: &'a mut AudioManager, port_no: usize) -> Result<&'a mut Port, CellError> {
    check_port_no(port_no)?;
    m.ports[port_no].as_mut().ok_or(errors::PORT_NOT_OPEN)
}

fn port_ref<'a>(m: &'a AudioManager, port_no: usize) -> Result<&'a Port, CellError> {
    check_port_no(port_no)?;
    m.ports[port_no].as_ref().ok_or(errors::PORT_NOT_OPEN)
}

/// `cellAudioPortClose(port_no)`.
#[must_use]
pub fn cell_audio_port_close(m: &mut AudioManager, port_no: usize) -> Result<(), CellError> {
    ensure_init(m)?;
    check_port_no(port_no)?;
    if m.ports[port_no].is_none() {
        return Err(errors::PORT_NOT_OPEN);
    }
    m.ports[port_no] = None;
    Ok(())
}

/// `cellAudioPortStart(port_no)`.
#[must_use]
pub fn cell_audio_port_start(m: &mut AudioManager, port_no: usize) -> Result<(), CellError> {
    ensure_init(m)?;
    let p = port_mut(m, port_no)?;
    if p.status == PortStatus::Run {
        return Err(errors::PORT_ALREADY_RUN);
    }
    p.status = PortStatus::Run;
    Ok(())
}

/// `cellAudioPortStop(port_no)`.
#[must_use]
pub fn cell_audio_port_stop(m: &mut AudioManager, port_no: usize) -> Result<(), CellError> {
    ensure_init(m)?;
    let p = port_mut(m, port_no)?;
    if p.status != PortStatus::Run {
        return Err(errors::PORT_NOT_RUN);
    }
    p.status = PortStatus::Ready;
    Ok(())
}

/// `cellAudioGetPortConfig(port_no)`.
#[must_use]
pub fn cell_audio_get_port_config(m: &AudioManager, port_no: usize) -> Result<PortConfig, CellError> {
    ensure_init(m)?;
    let p = port_ref(m, port_no)?;
    let port_size = BLOCK_SAMPLES as u32 * p.param.num_channels * p.param.num_blocks * 4;
    Ok(PortConfig {
        status: p.status.as_u32(),
        num_channels: p.param.num_channels,
        num_blocks: p.param.num_blocks,
        port_size,
    })
}

/// `cellAudioAddData(port_no, src_samples)` — push exactly
/// `BLOCK_SAMPLES * num_channels` samples into the port's ring. Returns
/// `PORT_NOT_RUN` if the port isn't running; `PARAM` if the caller
/// passes a wrong-sized block.
#[must_use]
pub fn cell_audio_add_data<S: AudioSink + ?Sized>(
    m: &mut AudioManager,
    sink: &mut S,
    port_no: usize,
    samples: &[f32],
) -> Result<(), CellError> {
    ensure_init(m)?;
    let p = port_mut(m, port_no)?;
    if p.status != PortStatus::Run {
        return Err(errors::PORT_NOT_RUN);
    }
    let expected = BLOCK_SAMPLES * p.param.num_channels as usize;
    if samples.len() != expected {
        return Err(errors::PARAM);
    }

    let idx = p.write_index as usize;
    p.blocks[idx].copy_from_slice(samples);
    p.write_index = (p.write_index + 1) % p.param.num_blocks;
    if p.blocks_pending < p.param.num_blocks {
        p.blocks_pending += 1;
    }

    // Hand block to sink — real backend would pull asynchronously;
    // here we pass straight through so tests can verify.
    sink.push_block(p.param.num_channels, samples);
    Ok(())
}

// =====================================================================
// Reference sink — used by tests, captures every block
// =====================================================================

#[derive(Debug, Default)]
pub struct TestAudioSink {
    pub blocks: Vec<(u32, Vec<f32>)>,
}

impl AudioSink for TestAudioSink {
    fn push_block(&mut self, num_channels: u32, samples: &[f32]) {
        self.blocks.push((num_channels, samples.to_vec()));
    }
}

// =====================================================================
// Tests
// =====================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn init_mgr() -> AudioManager {
        let mut m = AudioManager::default();
        cell_audio_init(&mut m).unwrap();
        m
    }

    // --- constants ------------------------------------------------

    #[test]
    fn error_codes_byte_exact_vs_cellAudio_h() {
        assert_eq!(errors::ALREADY_INIT.0, 0x8031_0701);
        assert_eq!(errors::NOT_INIT.0, 0x8031_0703);
        assert_eq!(errors::PORT_FULL.0, 0x8031_0705);
        assert_eq!(errors::PORT_NOT_RUN.0, 0x8031_0708);
        assert_eq!(errors::TAG_NOT_FOUND.0, 0x8031_070F);
    }

    #[test]
    fn audio_layout_constants() {
        assert_eq!(BLOCK_SAMPLES, 256);
        assert_eq!(PORT_2CH, 2);
        assert_eq!(PORT_8CH, 8);
        assert_eq!(BLOCKS_8, 8);
        assert_eq!(BLOCKS_16, 16);
        assert_eq!(BLOCKS_32, 32);
        assert_eq!(STATUS_CLOSE, 0x1010);
        assert_eq!(STATUS_READY, 1);
        assert_eq!(STATUS_RUN, 2);
    }

    // --- init/quit ------------------------------------------------

    #[test]
    fn init_twice_is_already_init() {
        let mut m = AudioManager::default();
        cell_audio_init(&mut m).unwrap();
        assert_eq!(cell_audio_init(&mut m).unwrap_err(), errors::ALREADY_INIT);
    }

    #[test]
    fn quit_without_init_is_not_init() {
        let mut m = AudioManager::default();
        assert_eq!(cell_audio_quit(&mut m).unwrap_err(), errors::NOT_INIT);
    }

    #[test]
    fn port_open_without_init_is_not_init() {
        let mut m = AudioManager::default();
        assert_eq!(
            cell_audio_port_open(&mut m, PortParam::default()).unwrap_err(),
            errors::NOT_INIT,
        );
    }

    // --- open/close -----------------------------------------------

    #[test]
    fn port_open_allocates_first_free_slot() {
        let mut m = init_mgr();
        let id = cell_audio_port_open(&mut m, PortParam::default()).unwrap();
        assert_eq!(id, 0);
        let id2 = cell_audio_port_open(&mut m, PortParam::default()).unwrap();
        assert_eq!(id2, 1);
    }

    #[test]
    fn port_open_rejects_bad_channels() {
        let mut m = init_mgr();
        let bad = PortParam { num_channels: 3, ..PortParam::default() };
        assert_eq!(cell_audio_port_open(&mut m, bad).unwrap_err(), errors::PARAM);
    }

    #[test]
    fn port_open_rejects_bad_block_count() {
        let mut m = init_mgr();
        let bad = PortParam { num_blocks: 7, ..PortParam::default() };
        assert_eq!(cell_audio_port_open(&mut m, bad).unwrap_err(), errors::PARAM);
    }

    #[test]
    fn port_open_fills_all_slots_then_port_full() {
        let mut m = init_mgr();
        for _ in 0..MAX_AUDIO_PORTS {
            cell_audio_port_open(&mut m, PortParam::default()).unwrap();
        }
        assert_eq!(
            cell_audio_port_open(&mut m, PortParam::default()).unwrap_err(),
            errors::PORT_FULL,
        );
    }

    #[test]
    fn port_close_on_unopened_is_port_not_open() {
        let mut m = init_mgr();
        assert_eq!(cell_audio_port_close(&mut m, 3).unwrap_err(), errors::PORT_NOT_OPEN);
    }

    #[test]
    fn port_close_frees_slot_for_reuse() {
        let mut m = init_mgr();
        let id = cell_audio_port_open(&mut m, PortParam::default()).unwrap();
        cell_audio_port_close(&mut m, id as usize).unwrap();
        // New open returns the same slot.
        assert_eq!(cell_audio_port_open(&mut m, PortParam::default()).unwrap(), id);
    }

    // --- start/stop ------------------------------------------------

    #[test]
    fn start_moves_to_run() {
        let mut m = init_mgr();
        let id = cell_audio_port_open(&mut m, PortParam::default()).unwrap();
        cell_audio_port_start(&mut m, id as usize).unwrap();
        let cfg = cell_audio_get_port_config(&m, id as usize).unwrap();
        assert_eq!(cfg.status, STATUS_RUN);
    }

    #[test]
    fn start_when_already_running_is_error() {
        let mut m = init_mgr();
        let id = cell_audio_port_open(&mut m, PortParam::default()).unwrap();
        cell_audio_port_start(&mut m, id as usize).unwrap();
        assert_eq!(
            cell_audio_port_start(&mut m, id as usize).unwrap_err(),
            errors::PORT_ALREADY_RUN,
        );
    }

    #[test]
    fn stop_moves_back_to_ready() {
        let mut m = init_mgr();
        let id = cell_audio_port_open(&mut m, PortParam::default()).unwrap();
        cell_audio_port_start(&mut m, id as usize).unwrap();
        cell_audio_port_stop(&mut m, id as usize).unwrap();
        let cfg = cell_audio_get_port_config(&m, id as usize).unwrap();
        assert_eq!(cfg.status, STATUS_READY);
    }

    #[test]
    fn stop_when_not_running_is_port_not_run() {
        let mut m = init_mgr();
        let id = cell_audio_port_open(&mut m, PortParam::default()).unwrap();
        assert_eq!(
            cell_audio_port_stop(&mut m, id as usize).unwrap_err(),
            errors::PORT_NOT_RUN,
        );
    }

    // --- config ---------------------------------------------------

    #[test]
    fn port_size_is_block_samples_times_channels_blocks_float() {
        let mut m = init_mgr();
        let param = PortParam { num_channels: PORT_8CH, num_blocks: BLOCKS_16, ..PortParam::default() };
        let id = cell_audio_port_open(&mut m, param).unwrap();
        let cfg = cell_audio_get_port_config(&m, id as usize).unwrap();
        assert_eq!(cfg.port_size, 256 * 8 * 16 * 4);
    }

    // --- add_data -------------------------------------------------

    #[test]
    fn add_data_before_start_is_port_not_run() {
        let mut m = init_mgr();
        let id = cell_audio_port_open(&mut m, PortParam::default()).unwrap();
        let buf = vec![0f32; BLOCK_SAMPLES * PORT_2CH as usize];
        let mut sink = TestAudioSink::default();
        assert_eq!(
            cell_audio_add_data(&mut m, &mut sink, id as usize, &buf).unwrap_err(),
            errors::PORT_NOT_RUN,
        );
    }

    #[test]
    fn add_data_wrong_size_is_param() {
        let mut m = init_mgr();
        let id = cell_audio_port_open(&mut m, PortParam::default()).unwrap();
        cell_audio_port_start(&mut m, id as usize).unwrap();
        let short = vec![0f32; 10];
        let mut sink = TestAudioSink::default();
        assert_eq!(
            cell_audio_add_data(&mut m, &mut sink, id as usize, &short).unwrap_err(),
            errors::PARAM,
        );
    }

    #[test]
    fn add_data_happy_path_forwards_to_sink() {
        let mut m = init_mgr();
        let id = cell_audio_port_open(&mut m, PortParam::default()).unwrap();
        cell_audio_port_start(&mut m, id as usize).unwrap();
        let mut buf = vec![0f32; BLOCK_SAMPLES * PORT_2CH as usize];
        buf[0] = 0.5;
        buf[1] = -0.25;
        let mut sink = TestAudioSink::default();
        cell_audio_add_data(&mut m, &mut sink, id as usize, &buf).unwrap();
        assert_eq!(sink.blocks.len(), 1);
        assert_eq!(sink.blocks[0].0, PORT_2CH);
        assert_eq!(sink.blocks[0].1[0], 0.5);
        assert_eq!(sink.blocks[0].1[1], -0.25);
    }

    #[test]
    fn add_data_ring_buffer_wraps_around() {
        let mut m = init_mgr();
        let param = PortParam { num_blocks: BLOCKS_8, ..PortParam::default() };
        let id = cell_audio_port_open(&mut m, param).unwrap();
        cell_audio_port_start(&mut m, id as usize).unwrap();

        let buf = vec![0f32; BLOCK_SAMPLES * PORT_2CH as usize];
        let mut sink = TestAudioSink::default();
        for _ in 0..BLOCKS_8 * 2 {
            cell_audio_add_data(&mut m, &mut sink, id as usize, &buf).unwrap();
        }

        // All 16 blocks passed to the sink.
        assert_eq!(sink.blocks.len(), 16);

        // Ring pointer has wrapped back to start.
        let port = m.ports[id as usize].as_ref().unwrap();
        assert_eq!(port.write_index, 0);
        assert_eq!(port.blocks_pending, BLOCKS_8);
    }

    #[test]
    fn multi_port_isolation() {
        let mut m = init_mgr();
        let a = cell_audio_port_open(&mut m, PortParam::default()).unwrap();
        let b = cell_audio_port_open(&mut m, PortParam { num_channels: PORT_8CH, ..PortParam::default() }).unwrap();
        cell_audio_port_start(&mut m, a as usize).unwrap();
        cell_audio_port_start(&mut m, b as usize).unwrap();

        let mut sink = TestAudioSink::default();
        let buf_a = vec![1f32; BLOCK_SAMPLES * PORT_2CH as usize];
        let buf_b = vec![2f32; BLOCK_SAMPLES * PORT_8CH as usize];
        cell_audio_add_data(&mut m, &mut sink, a as usize, &buf_a).unwrap();
        cell_audio_add_data(&mut m, &mut sink, b as usize, &buf_b).unwrap();

        assert_eq!(sink.blocks.len(), 2);
        assert_eq!(sink.blocks[0].0, PORT_2CH);
        assert_eq!(sink.blocks[1].0, PORT_8CH);
        assert_eq!(sink.blocks[0].1.len(), BLOCK_SAMPLES * 2);
        assert_eq!(sink.blocks[1].1.len(), BLOCK_SAMPLES * 8);
    }

    #[test]
    fn port_config_on_unopened_is_port_not_open() {
        let m = init_mgr();
        assert_eq!(
            cell_audio_get_port_config(&m, 0).unwrap_err(),
            errors::PORT_NOT_OPEN,
        );
    }

    #[test]
    fn bad_port_no_is_param() {
        let mut m = init_mgr();
        assert_eq!(cell_audio_port_close(&mut m, 99).unwrap_err(), errors::PARAM);
    }

    #[test]
    fn quit_releases_all_ports() {
        let mut m = init_mgr();
        cell_audio_port_open(&mut m, PortParam::default()).unwrap();
        cell_audio_port_open(&mut m, PortParam::default()).unwrap();
        cell_audio_quit(&mut m).unwrap();
        // After quit, even port_close returns NOT_INIT.
        assert_eq!(cell_audio_port_close(&mut m, 0).unwrap_err(), errors::NOT_INIT);
    }
}
