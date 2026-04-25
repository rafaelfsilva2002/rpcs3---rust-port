//! `rpcs3-hle-cellmic` — microphone / MicIn HLE.
//!
//! Ports `rpcs3/Emu/Cell/Modules/cellMic.cpp`. Games use cellMic to read
//! raw PCM from USB / Bluetooth mics, EyeToy / PlayStation Eye audio, or
//! SingStar microphones. The API:
//!
//! 1. `Init`/`Init2` — allocate the mic subsystem.
//! 2. `Open(devId, format, type)` — reserve a mic slot.
//! 3. `Start(flag)` — begin streaming into the ring buffer.
//! 4. `Read(devId, buf)` / `ReadAux` — pull PCM frames.
//! 5. `Stop`/`Close`/`End`.
//!
//! ## Entry points covered
//!
//! | HLE function                      | Rust wrapper                  |
//! |-----------------------------------|-------------------------------|
//! | `cellMicInit` / `cellMicInit2`    | [`MicManager::init`]          |
//! | `cellMicEnd`                      | [`MicManager::end`]           |
//! | `cellMicOpen`                     | [`MicManager::open`]          |
//! | `cellMicClose`                    | [`MicManager::close`]         |
//! | `cellMicStart`                    | [`MicManager::start`]         |
//! | `cellMicStop`                     | [`MicManager::stop`]          |
//! | `cellMicRead` / `cellMicReadAux`  | [`MicManager::read`]          |
//! | `cellMicGetStatus`                | [`MicManager::status`]        |
//! | `cellMicGetDeviceAttr`            | [`MicManager::device_attr`]   |
//! | `cellMicSetDeviceAttr`            | [`MicManager::set_device_attr`]|

use rpcs3_emu_types::CellError;

// =====================================================================
// Error codes — byte-exact with cellMic.h:9-30
// =====================================================================

pub mod errors {
    use rpcs3_emu_types::CellError;

    pub const ALREADY_INIT: CellError = CellError(0x8014_0101);
    pub const DEVICE: CellError = CellError(0x8014_0102);
    pub const NOT_INIT: CellError = CellError(0x8014_0103);
    pub const PARAM: CellError = CellError(0x8014_0104);
    pub const PORT_FULL: CellError = CellError(0x8014_0105);
    pub const ALREADY_OPEN: CellError = CellError(0x8014_0106);
    pub const NOT_OPEN: CellError = CellError(0x8014_0107);
    pub const NOT_RUN: CellError = CellError(0x8014_0108);
    pub const TRANS_EVENT: CellError = CellError(0x8014_0109);
    pub const OPEN: CellError = CellError(0x8014_010a);
    pub const SHAREDMEMORY: CellError = CellError(0x8014_010b);
    pub const MUTEX: CellError = CellError(0x8014_010c);
    pub const EVENT_QUEUE: CellError = CellError(0x8014_010d);
    pub const DEVICE_NOT_FOUND: CellError = CellError(0x8014_010e);
    pub const FATAL: CellError = CellError(0x8014_010f);
    pub const DEVICE_NOT_SUPPORT: CellError = CellError(0x8014_0110);
}

// DSP subsystem errors (cellMic.h:32-49) — separate facility 0x80140200
pub mod dsp_errors {
    use rpcs3_emu_types::CellError;

    pub const DSP: CellError = CellError(0x8014_0200);
    pub const DSP_ASSERT: CellError = CellError(0x8014_0201);
    pub const DSP_PATH: CellError = CellError(0x8014_0202);
    pub const DSP_FILE: CellError = CellError(0x8014_0203);
    pub const DSP_PARAM: CellError = CellError(0x8014_0204);
    pub const DSP_MEMALLOC: CellError = CellError(0x8014_0205);
    pub const DSP_POINTER: CellError = CellError(0x8014_0206);
    pub const DSP_FUNC: CellError = CellError(0x8014_0207);
    pub const DSP_MEM: CellError = CellError(0x8014_0208);
    pub const DSP_ALIGN16: CellError = CellError(0x8014_0209);
    pub const DSP_ALIGN128: CellError = CellError(0x8014_020a);
    pub const DSP_EAALIGN128: CellError = CellError(0x8014_020b);
    pub const DSP_LIB_HANDLER: CellError = CellError(0x8014_0216);
    pub const DSP_LIB_INPARAM: CellError = CellError(0x8014_0217);
    pub const DSP_LIB_NOSPU: CellError = CellError(0x8014_0218);
    pub const DSP_LIB_SAMPRATE: CellError = CellError(0x8014_0219);
}

// =====================================================================
// Constants (cellMic.h:104-140)
// =====================================================================

// Signal types (bitmask — a mic stream can expose DSP + AUX + RAW at once).
pub const SIGTYPE_NULL: u8 = 0;
pub const SIGTYPE_DSP: u8 = 1;
pub const SIGTYPE_AUX: u8 = 2;
pub const SIGTYPE_RAW: u8 = 4;
pub const SIGTYPE_ALL_MASK: u8 = SIGTYPE_DSP | SIGTYPE_AUX | SIGTYPE_RAW;

// Device types.
pub const MIC_TYPE_UNDEF: i32 = -1;
pub const MIC_TYPE_UNKNOWN: i32 = 0;
pub const MIC_TYPE_EYETOY1: i32 = 1;
pub const MIC_TYPE_EYETOY2: i32 = 2;
pub const MIC_TYPE_USBAUDIO: i32 = 3;
pub const MIC_TYPE_BLUETOOTH: i32 = 4;
pub const MIC_TYPE_A2DP: i32 = 5;

#[must_use]
pub fn is_known_mic_type(t: i32) -> bool {
    (MIC_TYPE_UNKNOWN..=MIC_TYPE_A2DP).contains(&t)
}

// Signal / device attributes.
pub const DEVATTR_LED: u32 = 9;
pub const DEVATTR_GAIN: u32 = 10;
pub const DEVATTR_VOLUME: u32 = 201;
pub const DEVATTR_AGC: u32 = 202;
pub const DEVATTR_CHANVOL: u32 = 301;
pub const DEVATTR_DSPTYPE: u32 = 302;

pub const SIGATTR_BKNGAIN: u32 = 0;
pub const SIGATTR_REVERB: u32 = 9;
pub const SIGATTR_AGCLEVEL: u32 = 26;
pub const SIGATTR_VOLUME: u32 = 301;
pub const SIGATTR_PITCHSHIFT: u32 = 331;

// Signal states (returned by cellMicGetSignalState).
pub const SIGSTATE_LOCTALK: u32 = 0;
pub const SIGSTATE_FARTALK: u32 = 1;
pub const SIGSTATE_NSR: u32 = 3;
pub const SIGSTATE_AGC: u32 = 4;
pub const SIGSTATE_MICENG: u32 = 5;
pub const SIGSTATE_SPKENG: u32 = 6;

// Start flags (latency knob).
pub const STARTFLAG_LATENCY_4: u32 = 0x01;
pub const STARTFLAG_LATENCY_2: u32 = 0x02;
pub const STARTFLAG_LATENCY_1: u32 = 0x03;

#[must_use]
pub fn is_known_start_flag(f: u32) -> bool {
    matches!(f, STARTFLAG_LATENCY_4 | STARTFLAG_LATENCY_2 | STARTFLAG_LATENCY_1)
}

pub const MAX_MICS: usize = 8;
pub const MAX_MICS_PERMISSABLE: usize = 4;
pub const NULL_DEVICE_ID: i32 = -1;

// =====================================================================
// Types
// =====================================================================

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MicFormat {
    pub channel_num: u8,
    pub subframe_size: u8,
    pub bit_resolution: u8,
    pub data_type: u8,
    pub sample_rate: u32,
}

impl MicFormat {
    #[must_use]
    pub const fn mono_16khz() -> Self {
        Self { channel_num: 1, subframe_size: 2, bit_resolution: 16, data_type: 0, sample_rate: 16_000 }
    }

    fn validate(&self) -> Result<(), CellError> {
        if !(1..=2).contains(&self.channel_num) {
            return Err(errors::PARAM);
        }
        if ![8, 16, 24, 32].contains(&self.bit_resolution) {
            return Err(errors::PARAM);
        }
        if self.sample_rate == 0 {
            return Err(errors::PARAM);
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MicStatus {
    pub raw_samprate: i32,
    pub dsp_samprate: i32,
    pub dsp_volume: i32,
    pub is_start: i32,
    pub is_open: i32,
    pub local_voice: i32,
    pub remote_voice: i32,
    pub mic_energy_bits: u32, // f32::to_bits preserved (struct impls Eq)
    pub spk_energy_bits: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum SlotState {
    Closed,
    Opened,
    Running,
}

#[derive(Clone, Debug)]
struct MicSlot {
    device_id: i32,
    mic_type: i32,
    #[allow(dead_code)] // bitmask stored for future GetSignalState queries
    signal_types: u8,
    format: MicFormat,
    state: SlotState,
    pending_pcm: std::collections::VecDeque<u8>,
    gain: i32,
    volume: i32,
    agc: i32,
    led: i32,
}

#[derive(Clone, Debug)]
pub struct MicManager {
    initialized: bool,
    slots: Vec<MicSlot>,
}

impl MicManager {
    #[must_use]
    pub fn new() -> Self {
        Self { initialized: false, slots: Vec::new() }
    }

    #[must_use]
    pub fn is_initialized(&self) -> bool {
        self.initialized
    }

    #[must_use]
    pub fn open_count(&self) -> usize {
        self.slots.len()
    }

    // ----------------- Lifecycle -----------------

    /// `cellMicInit` / `cellMicInit2` share this entry. Caller ensures
    /// they only call once per session.
    pub fn init(&mut self) -> Result<(), CellError> {
        if self.initialized {
            return Err(errors::ALREADY_INIT);
        }
        self.initialized = true;
        self.slots.clear();
        Ok(())
    }

    pub fn end(&mut self) -> Result<(), CellError> {
        if !self.initialized {
            return Err(errors::NOT_INIT);
        }
        self.initialized = false;
        self.slots.clear();
        Ok(())
    }

    // ----------------- Open / Close -----------------

    /// `cellMicOpen(dev_num, sample_rate)` or `cellMicOpenEx` — unified
    /// entry. `dev_num` identifies the hardware device id (>=0); mic_type
    /// classifies it (EyeToy / USB / BT / A2DP).
    pub fn open(
        &mut self,
        device_id: i32,
        mic_type: i32,
        signal_types: u8,
        format: MicFormat,
    ) -> Result<(), CellError> {
        self.require_init()?;
        if device_id < 0 {
            return Err(errors::PARAM);
        }
        if !is_known_mic_type(mic_type) {
            return Err(errors::DEVICE_NOT_SUPPORT);
        }
        if signal_types == 0 || (signal_types & !SIGTYPE_ALL_MASK) != 0 {
            return Err(errors::PARAM);
        }
        format.validate()?;
        if self.slots.iter().any(|s| s.device_id == device_id) {
            return Err(errors::ALREADY_OPEN);
        }
        if self.slots.len() >= MAX_MICS_PERMISSABLE {
            return Err(errors::PORT_FULL);
        }
        self.slots.push(MicSlot {
            device_id,
            mic_type,
            signal_types,
            format,
            state: SlotState::Opened,
            pending_pcm: std::collections::VecDeque::new(),
            gain: 0,
            volume: 100,
            agc: 0,
            led: 0,
        });
        Ok(())
    }

    pub fn close(&mut self, device_id: i32) -> Result<(), CellError> {
        self.require_init()?;
        let idx = self.slot_idx(device_id)?;
        self.slots.remove(idx);
        Ok(())
    }

    // ----------------- Run control -----------------

    pub fn start(&mut self, device_id: i32, start_flag: u32) -> Result<(), CellError> {
        self.require_init()?;
        if !is_known_start_flag(start_flag) {
            return Err(errors::PARAM);
        }
        let idx = self.slot_idx(device_id)?;
        if self.slots[idx].state == SlotState::Running {
            return Err(errors::PARAM);
        }
        self.slots[idx].state = SlotState::Running;
        Ok(())
    }

    pub fn stop(&mut self, device_id: i32) -> Result<(), CellError> {
        self.require_init()?;
        let idx = self.slot_idx(device_id)?;
        if self.slots[idx].state != SlotState::Running {
            return Err(errors::NOT_RUN);
        }
        self.slots[idx].state = SlotState::Opened;
        self.slots[idx].pending_pcm.clear();
        Ok(())
    }

    // ----------------- Read -----------------

    /// Test hook — injects PCM bytes captured from the backend into the
    /// mic's ring buffer. `cellMicStart` in the real lib arms async IO.
    pub fn inject_pcm(&mut self, device_id: i32, bytes: &[u8]) -> Result<(), CellError> {
        self.require_init()?;
        let idx = self.slot_idx(device_id)?;
        if self.slots[idx].state != SlotState::Running {
            return Err(errors::NOT_RUN);
        }
        self.slots[idx].pending_pcm.extend(bytes);
        Ok(())
    }

    pub fn read(&mut self, device_id: i32, out: &mut [u8]) -> Result<usize, CellError> {
        self.require_init()?;
        let idx = self.slot_idx(device_id)?;
        if self.slots[idx].state != SlotState::Running {
            return Err(errors::NOT_RUN);
        }
        let available = self.slots[idx].pending_pcm.len();
        let take = available.min(out.len());
        for b in out.iter_mut().take(take) {
            *b = self.slots[idx].pending_pcm.pop_front().unwrap_or(0);
        }
        Ok(take)
    }

    // ----------------- Status / attrs -----------------

    pub fn status(&self, device_id: i32) -> Result<MicStatus, CellError> {
        self.require_init()?;
        let idx = self.slot_idx(device_id)?;
        let s = &self.slots[idx];
        Ok(MicStatus {
            raw_samprate: s.format.sample_rate as i32,
            dsp_samprate: s.format.sample_rate as i32,
            dsp_volume: s.volume,
            is_start: i32::from(s.state == SlotState::Running),
            is_open: i32::from(s.state != SlotState::Closed),
            local_voice: 0,
            remote_voice: 0,
            mic_energy_bits: 0.0f32.to_bits(),
            spk_energy_bits: 0.0f32.to_bits(),
        })
    }

    pub fn device_attr(&self, device_id: i32, attr: u32) -> Result<i32, CellError> {
        self.require_init()?;
        let idx = self.slot_idx(device_id)?;
        let s = &self.slots[idx];
        Ok(match attr {
            DEVATTR_LED => s.led,
            DEVATTR_GAIN => s.gain,
            DEVATTR_VOLUME => s.volume,
            DEVATTR_AGC => s.agc,
            DEVATTR_CHANVOL => s.volume,
            DEVATTR_DSPTYPE => i32::from(s.mic_type),
            _ => return Err(errors::PARAM),
        })
    }

    pub fn set_device_attr(&mut self, device_id: i32, attr: u32, value: i32) -> Result<(), CellError> {
        self.require_init()?;
        let idx = self.slot_idx(device_id)?;
        let s = &mut self.slots[idx];
        match attr {
            DEVATTR_LED => s.led = value,
            DEVATTR_GAIN => s.gain = value.clamp(0, 127),
            DEVATTR_VOLUME | DEVATTR_CHANVOL => s.volume = value.clamp(0, 127),
            DEVATTR_AGC => s.agc = if value != 0 { 1 } else { 0 },
            _ => return Err(errors::PARAM),
        }
        Ok(())
    }

    // ----------------- Helpers -----------------

    fn require_init(&self) -> Result<(), CellError> {
        if self.initialized { Ok(()) } else { Err(errors::NOT_INIT) }
    }

    fn slot_idx(&self, device_id: i32) -> Result<usize, CellError> {
        self.slots.iter().position(|s| s.device_id == device_id).ok_or(errors::NOT_OPEN)
    }
}

impl Default for MicManager {
    fn default() -> Self {
        Self::new()
    }
}

// =====================================================================
// Tests
// =====================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn initialized() -> MicManager {
        let mut m = MicManager::new();
        m.init().unwrap();
        m
    }

    fn open_mic(m: &mut MicManager, id: i32) {
        m.open(id, MIC_TYPE_USBAUDIO, SIGTYPE_RAW, MicFormat::mono_16khz()).unwrap();
    }

    #[test]
    fn error_codes_byte_exact() {
        assert_eq!(errors::ALREADY_INIT.0, 0x8014_0101);
        assert_eq!(errors::DEVICE.0, 0x8014_0102);
        assert_eq!(errors::NOT_INIT.0, 0x8014_0103);
        assert_eq!(errors::PARAM.0, 0x8014_0104);
        assert_eq!(errors::PORT_FULL.0, 0x8014_0105);
        assert_eq!(errors::ALREADY_OPEN.0, 0x8014_0106);
        assert_eq!(errors::NOT_OPEN.0, 0x8014_0107);
        assert_eq!(errors::NOT_RUN.0, 0x8014_0108);
        assert_eq!(errors::DEVICE_NOT_FOUND.0, 0x8014_010e);
        assert_eq!(errors::DEVICE_NOT_SUPPORT.0, 0x8014_0110);
    }

    #[test]
    fn dsp_error_codes_byte_exact() {
        assert_eq!(dsp_errors::DSP.0, 0x8014_0200);
        assert_eq!(dsp_errors::DSP_ASSERT.0, 0x8014_0201);
        assert_eq!(dsp_errors::DSP_MEMALLOC.0, 0x8014_0205);
        assert_eq!(dsp_errors::DSP_ALIGN128.0, 0x8014_020a);
        assert_eq!(dsp_errors::DSP_LIB_HANDLER.0, 0x8014_0216);
        assert_eq!(dsp_errors::DSP_LIB_SAMPRATE.0, 0x8014_0219);
    }

    #[test]
    fn signal_type_mask_stable() {
        assert_eq!(SIGTYPE_NULL, 0);
        assert_eq!(SIGTYPE_DSP, 1);
        assert_eq!(SIGTYPE_AUX, 2);
        assert_eq!(SIGTYPE_RAW, 4);
        assert_eq!(SIGTYPE_ALL_MASK, 7);
    }

    #[test]
    fn mic_type_constants_stable() {
        assert_eq!(MIC_TYPE_UNDEF, -1);
        assert_eq!(MIC_TYPE_UNKNOWN, 0);
        assert_eq!(MIC_TYPE_EYETOY1, 1);
        assert_eq!(MIC_TYPE_EYETOY2, 2);
        assert_eq!(MIC_TYPE_USBAUDIO, 3);
        assert_eq!(MIC_TYPE_BLUETOOTH, 4);
        assert_eq!(MIC_TYPE_A2DP, 5);
    }

    #[test]
    fn devattr_constants_stable() {
        assert_eq!(DEVATTR_LED, 9);
        assert_eq!(DEVATTR_GAIN, 10);
        assert_eq!(DEVATTR_VOLUME, 201);
        assert_eq!(DEVATTR_AGC, 202);
        assert_eq!(DEVATTR_CHANVOL, 301);
        assert_eq!(DEVATTR_DSPTYPE, 302);
    }

    #[test]
    fn sigattr_constants_stable() {
        assert_eq!(SIGATTR_BKNGAIN, 0);
        assert_eq!(SIGATTR_REVERB, 9);
        assert_eq!(SIGATTR_AGCLEVEL, 26);
        assert_eq!(SIGATTR_VOLUME, 301);
        assert_eq!(SIGATTR_PITCHSHIFT, 331);
    }

    #[test]
    fn sigstate_constants_stable() {
        assert_eq!(SIGSTATE_LOCTALK, 0);
        assert_eq!(SIGSTATE_FARTALK, 1);
        assert_eq!(SIGSTATE_NSR, 3);
        assert_eq!(SIGSTATE_AGC, 4);
        assert_eq!(SIGSTATE_MICENG, 5);
        assert_eq!(SIGSTATE_SPKENG, 6);
    }

    #[test]
    fn start_flag_constants_stable() {
        assert_eq!(STARTFLAG_LATENCY_4, 0x01);
        assert_eq!(STARTFLAG_LATENCY_2, 0x02);
        assert_eq!(STARTFLAG_LATENCY_1, 0x03);
    }

    #[test]
    fn limit_constants_stable() {
        assert_eq!(MAX_MICS, 8);
        assert_eq!(MAX_MICS_PERMISSABLE, 4);
        assert_eq!(NULL_DEVICE_ID, -1);
    }

    #[test]
    fn init_happy_path() {
        let mut m = MicManager::new();
        m.init().unwrap();
        assert!(m.is_initialized());
    }

    #[test]
    fn init_twice_is_already_init() {
        let mut m = initialized();
        assert_eq!(m.init(), Err(errors::ALREADY_INIT));
    }

    #[test]
    fn end_without_init_is_not_init() {
        let mut m = MicManager::new();
        assert_eq!(m.end(), Err(errors::NOT_INIT));
    }

    #[test]
    fn end_happy_path() {
        let mut m = initialized();
        m.end().unwrap();
        assert!(!m.is_initialized());
    }

    #[test]
    fn open_without_init_is_not_init() {
        let mut m = MicManager::new();
        assert_eq!(
            m.open(0, MIC_TYPE_USBAUDIO, SIGTYPE_RAW, MicFormat::mono_16khz()),
            Err(errors::NOT_INIT)
        );
    }

    #[test]
    fn open_negative_device_id_rejected() {
        let mut m = initialized();
        assert_eq!(
            m.open(-1, MIC_TYPE_USBAUDIO, SIGTYPE_RAW, MicFormat::mono_16khz()),
            Err(errors::PARAM)
        );
    }

    #[test]
    fn open_unknown_mic_type_rejected() {
        let mut m = initialized();
        assert_eq!(
            m.open(0, 99, SIGTYPE_RAW, MicFormat::mono_16khz()),
            Err(errors::DEVICE_NOT_SUPPORT)
        );
    }

    #[test]
    fn open_zero_signal_types_rejected() {
        let mut m = initialized();
        assert_eq!(
            m.open(0, MIC_TYPE_USBAUDIO, SIGTYPE_NULL, MicFormat::mono_16khz()),
            Err(errors::PARAM)
        );
    }

    #[test]
    fn open_invalid_signal_bit_rejected() {
        let mut m = initialized();
        assert_eq!(
            m.open(0, MIC_TYPE_USBAUDIO, 0xF0, MicFormat::mono_16khz()),
            Err(errors::PARAM)
        );
    }

    #[test]
    fn open_bad_channel_count_rejected() {
        let mut m = initialized();
        let mut f = MicFormat::mono_16khz();
        f.channel_num = 3;
        assert_eq!(m.open(0, MIC_TYPE_USBAUDIO, SIGTYPE_RAW, f), Err(errors::PARAM));
    }

    #[test]
    fn open_zero_sample_rate_rejected() {
        let mut m = initialized();
        let mut f = MicFormat::mono_16khz();
        f.sample_rate = 0;
        assert_eq!(m.open(0, MIC_TYPE_USBAUDIO, SIGTYPE_RAW, f), Err(errors::PARAM));
    }

    #[test]
    fn open_bad_bit_resolution_rejected() {
        let mut m = initialized();
        let mut f = MicFormat::mono_16khz();
        f.bit_resolution = 12;
        assert_eq!(m.open(0, MIC_TYPE_USBAUDIO, SIGTYPE_RAW, f), Err(errors::PARAM));
    }

    #[test]
    fn open_same_device_id_is_already_open() {
        let mut m = initialized();
        open_mic(&mut m, 0);
        assert_eq!(
            m.open(0, MIC_TYPE_USBAUDIO, SIGTYPE_RAW, MicFormat::mono_16khz()),
            Err(errors::ALREADY_OPEN)
        );
    }

    #[test]
    fn open_exceeds_port_cap_is_port_full() {
        let mut m = initialized();
        for i in 0..MAX_MICS_PERMISSABLE as i32 {
            open_mic(&mut m, i);
        }
        assert_eq!(
            m.open(4, MIC_TYPE_USBAUDIO, SIGTYPE_RAW, MicFormat::mono_16khz()),
            Err(errors::PORT_FULL)
        );
    }

    #[test]
    fn open_combined_signal_types_accepted() {
        let mut m = initialized();
        m.open(0, MIC_TYPE_USBAUDIO, SIGTYPE_RAW | SIGTYPE_AUX | SIGTYPE_DSP, MicFormat::mono_16khz()).unwrap();
    }

    #[test]
    fn close_unknown_device_is_not_open() {
        let mut m = initialized();
        assert_eq!(m.close(99), Err(errors::NOT_OPEN));
    }

    #[test]
    fn close_happy_path() {
        let mut m = initialized();
        open_mic(&mut m, 0);
        m.close(0).unwrap();
        assert_eq!(m.open_count(), 0);
    }

    #[test]
    fn start_bad_flag_rejected() {
        let mut m = initialized();
        open_mic(&mut m, 0);
        assert_eq!(m.start(0, 99), Err(errors::PARAM));
    }

    #[test]
    fn start_unknown_device_is_not_open() {
        let mut m = initialized();
        assert_eq!(m.start(0, STARTFLAG_LATENCY_4), Err(errors::NOT_OPEN));
    }

    #[test]
    fn start_then_start_again_rejected() {
        let mut m = initialized();
        open_mic(&mut m, 0);
        m.start(0, STARTFLAG_LATENCY_4).unwrap();
        assert_eq!(m.start(0, STARTFLAG_LATENCY_4), Err(errors::PARAM));
    }

    #[test]
    fn stop_without_start_is_not_run() {
        let mut m = initialized();
        open_mic(&mut m, 0);
        assert_eq!(m.stop(0), Err(errors::NOT_RUN));
    }

    #[test]
    fn stop_clears_pending_pcm() {
        let mut m = initialized();
        open_mic(&mut m, 0);
        m.start(0, STARTFLAG_LATENCY_4).unwrap();
        m.inject_pcm(0, &[1, 2, 3, 4]).unwrap();
        m.stop(0).unwrap();
        // Re-start then read — buffer should be empty.
        m.start(0, STARTFLAG_LATENCY_4).unwrap();
        let mut buf = [0u8; 4];
        assert_eq!(m.read(0, &mut buf), Ok(0));
    }

    #[test]
    fn inject_pcm_without_start_is_not_run() {
        let mut m = initialized();
        open_mic(&mut m, 0);
        assert_eq!(m.inject_pcm(0, &[1]), Err(errors::NOT_RUN));
    }

    #[test]
    fn read_without_start_is_not_run() {
        let mut m = initialized();
        open_mic(&mut m, 0);
        let mut buf = [0u8; 4];
        assert_eq!(m.read(0, &mut buf), Err(errors::NOT_RUN));
    }

    #[test]
    fn read_returns_available_bytes() {
        let mut m = initialized();
        open_mic(&mut m, 0);
        m.start(0, STARTFLAG_LATENCY_4).unwrap();
        m.inject_pcm(0, &[10, 20, 30]).unwrap();
        let mut buf = [0u8; 8];
        let n = m.read(0, &mut buf).unwrap();
        assert_eq!(n, 3);
        assert_eq!(&buf[..3], &[10, 20, 30]);
    }

    #[test]
    fn read_larger_buffer_than_available_returns_partial() {
        let mut m = initialized();
        open_mic(&mut m, 0);
        m.start(0, STARTFLAG_LATENCY_4).unwrap();
        m.inject_pcm(0, &[1, 2]).unwrap();
        let mut buf = [0u8; 4];
        assert_eq!(m.read(0, &mut buf), Ok(2));
    }

    #[test]
    fn read_drains_and_returns_zero_when_empty() {
        let mut m = initialized();
        open_mic(&mut m, 0);
        m.start(0, STARTFLAG_LATENCY_4).unwrap();
        m.inject_pcm(0, &[7]).unwrap();
        let mut buf = [0u8; 4];
        assert_eq!(m.read(0, &mut buf), Ok(1));
        assert_eq!(m.read(0, &mut buf), Ok(0));
    }

    #[test]
    fn status_reports_running_flags() {
        let mut m = initialized();
        open_mic(&mut m, 0);
        let s = m.status(0).unwrap();
        assert_eq!(s.is_open, 1);
        assert_eq!(s.is_start, 0);
        m.start(0, STARTFLAG_LATENCY_4).unwrap();
        let s = m.status(0).unwrap();
        assert_eq!(s.is_start, 1);
        assert_eq!(s.raw_samprate, 16_000);
    }

    #[test]
    fn device_attr_round_trip() {
        let mut m = initialized();
        open_mic(&mut m, 0);
        m.set_device_attr(0, DEVATTR_VOLUME, 80).unwrap();
        assert_eq!(m.device_attr(0, DEVATTR_VOLUME), Ok(80));
        m.set_device_attr(0, DEVATTR_GAIN, 50).unwrap();
        assert_eq!(m.device_attr(0, DEVATTR_GAIN), Ok(50));
        m.set_device_attr(0, DEVATTR_AGC, 1).unwrap();
        assert_eq!(m.device_attr(0, DEVATTR_AGC), Ok(1));
    }

    #[test]
    fn device_attr_clamps_gain_and_volume() {
        let mut m = initialized();
        open_mic(&mut m, 0);
        m.set_device_attr(0, DEVATTR_VOLUME, 999).unwrap();
        assert_eq!(m.device_attr(0, DEVATTR_VOLUME), Ok(127));
        m.set_device_attr(0, DEVATTR_GAIN, -5).unwrap();
        assert_eq!(m.device_attr(0, DEVATTR_GAIN), Ok(0));
    }

    #[test]
    fn device_attr_unknown_attribute_rejected() {
        let mut m = initialized();
        open_mic(&mut m, 0);
        assert_eq!(m.device_attr(0, 9999), Err(errors::PARAM));
        assert_eq!(m.set_device_attr(0, 9999, 1), Err(errors::PARAM));
    }

    #[test]
    fn device_attr_without_init_is_not_init() {
        let m = MicManager::new();
        assert_eq!(m.device_attr(0, DEVATTR_LED), Err(errors::NOT_INIT));
    }

    #[test]
    fn is_known_mic_type_helper() {
        assert!(is_known_mic_type(MIC_TYPE_UNKNOWN));
        assert!(is_known_mic_type(MIC_TYPE_A2DP));
        assert!(!is_known_mic_type(MIC_TYPE_UNDEF));
        assert!(!is_known_mic_type(99));
    }

    #[test]
    fn is_known_start_flag_helper() {
        assert!(is_known_start_flag(STARTFLAG_LATENCY_4));
        assert!(is_known_start_flag(STARTFLAG_LATENCY_2));
        assert!(is_known_start_flag(STARTFLAG_LATENCY_1));
        assert!(!is_known_start_flag(0));
        assert!(!is_known_start_flag(99));
    }

    #[test]
    fn full_mic_lifecycle_smoke() {
        let mut m = MicManager::new();
        m.init().unwrap();
        m.open(0, MIC_TYPE_USBAUDIO, SIGTYPE_RAW | SIGTYPE_AUX, MicFormat::mono_16khz()).unwrap();
        m.set_device_attr(0, DEVATTR_VOLUME, 80).unwrap();
        m.start(0, STARTFLAG_LATENCY_2).unwrap();
        m.inject_pcm(0, &[0xAA, 0xBB, 0xCC, 0xDD]).unwrap();
        let mut buf = [0u8; 8];
        let n = m.read(0, &mut buf).unwrap();
        assert_eq!(n, 4);
        assert_eq!(&buf[..4], &[0xAA, 0xBB, 0xCC, 0xDD]);
        assert_eq!(m.status(0).unwrap().is_start, 1);
        m.stop(0).unwrap();
        m.close(0).unwrap();
        m.end().unwrap();
    }
}
