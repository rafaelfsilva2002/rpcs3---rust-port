//! `rpcs3-hle-cellvoice` — libvoice (voice chat / codec router) HLE.
//!
//! Ports `rpcs3/Emu/Cell/Modules/cellVoice.cpp`. `libvoice` routes PCM
//! between microphone / network / audio-out ports through a port-graph
//! of user-created "ports". The API shape is:
//!
//! 1. `Init(param)` — allocate resources, set event mask + app type.
//! 2. `CreatePort(param)` — allocate a typed port (MIC / PCM / VOICE).
//! 3. `ConnectIPortToOPort(in, out)` — add an edge.
//! 4. `StartSession(param)` — begin streaming.
//! 5. `Read/WritePort` (PCM), `PausePort`, `ResetPort`.
//! 6. `StopSession → End`.

use rpcs3_emu_types::CellError;

// =====================================================================
// Error codes — byte-exact with cellVoice.h:7-27
// =====================================================================

pub mod errors {
    use rpcs3_emu_types::CellError;

    pub const LIBVOICE_NOT_INIT: CellError = CellError(0x8031_0801);
    pub const LIBVOICE_INITIALIZED: CellError = CellError(0x8031_0802);
    pub const GENERAL: CellError = CellError(0x8031_0803);
    pub const PORT_INVALID: CellError = CellError(0x8031_0804);
    pub const ARGUMENT_INVALID: CellError = CellError(0x8031_0805);
    pub const CONTAINER_INVALID: CellError = CellError(0x8031_0806);
    pub const TOPOLOGY: CellError = CellError(0x8031_0807);
    pub const RESOURCE_INSUFFICIENT: CellError = CellError(0x8031_0808);
    pub const NOT_IMPLEMENTED: CellError = CellError(0x8031_0809);
    pub const ADDRESS_INVALID: CellError = CellError(0x8031_080a);
    pub const SERVICE_DETACHED: CellError = CellError(0x8031_080b);
    pub const SERVICE_ATTACHED: CellError = CellError(0x8031_080c);
    pub const SERVICE_NOT_FOUND: CellError = CellError(0x8031_080d);
    pub const SHAREDMEMORY: CellError = CellError(0x8031_080e);
    pub const EVENT_QUEUE: CellError = CellError(0x8031_080f);
    pub const SERVICE_HANDLE: CellError = CellError(0x8031_0810);
    pub const EVENT_DISPATCH: CellError = CellError(0x8031_0811);
    pub const DEVICE_NOT_PRESENT: CellError = CellError(0x8031_0812);
}

// =====================================================================
// Version / App type
// =====================================================================

pub const VERSION_100: u32 = 100;
pub const APPTYPE_GAME_1MB: u32 = 1 << 29;

// =====================================================================
// BitRate values (cellVoice.h:34-44)
// =====================================================================

pub const BITRATE_NULL: u32 = !0u32;
pub const BITRATE_3850: u32 = 3850;
pub const BITRATE_4650: u32 = 4650;
pub const BITRATE_5700: u32 = 5700;
pub const BITRATE_7300: u32 = 7300;
pub const BITRATE_14400: u32 = 14400;
pub const BITRATE_16000: u32 = 16000;
pub const BITRATE_22533: u32 = 22533;

#[must_use]
pub fn is_known_bitrate(b: u32) -> bool {
    matches!(
        b,
        BITRATE_3850 | BITRATE_4650 | BITRATE_5700 | BITRATE_7300 | BITRATE_14400 | BITRATE_16000 | BITRATE_22533
    )
}

// =====================================================================
// Event mask bits (cellVoice.h:46-55)
// =====================================================================

pub const EVENT_DATA_ERROR: u32 = 1 << 0;
pub const EVENT_PORT_ATTACHED: u32 = 1 << 1;
pub const EVENT_PORT_DETACHED: u32 = 1 << 2;
pub const EVENT_SERVICE_ATTACHED: u32 = 1 << 3;
pub const EVENT_SERVICE_DETACHED: u32 = 1 << 4;
pub const EVENT_PORT_WEAK_ATTACHED: u32 = 1 << 5;
pub const EVENT_PORT_WEAK_DETACHED: u32 = 1 << 6;
pub const EVENT_ALL_MASK: u32 = 0x7F;

// =====================================================================
// PCM data type / port attr / state / type / sampling rate
// =====================================================================

pub const PCM_NULL: u32 = !0u32;
pub const PCM_FLOAT: u32 = 0;
pub const PCM_FLOAT_LITTLE_ENDIAN: u32 = 1;
pub const PCM_SHORT: u32 = 2;
pub const PCM_SHORT_LITTLE_ENDIAN: u32 = 3;
pub const PCM_INTEGER: u32 = 4;
pub const PCM_INTEGER_LITTLE_ENDIAN: u32 = 5;

pub const ATTR_ENERGY_LEVEL: u32 = 1000;
pub const ATTR_VAD: u32 = 1001;
pub const ATTR_DTX: u32 = 1002;
pub const ATTR_AUTO_RESAMPLE: u32 = 1003;
pub const ATTR_LATENCY: u32 = 1004;
pub const ATTR_SILENCE_THRESHOLD: u32 = 1005;

pub const PORTSTATE_NULL: u32 = !0u32;
pub const PORTSTATE_IDLE: u32 = 0;
pub const PORTSTATE_READY: u32 = 1;
pub const PORTSTATE_BUFFERING: u32 = 2;
pub const PORTSTATE_RUNNING: u32 = 3;

pub const PORTTYPE_NULL: u32 = !0u32;
pub const PORTTYPE_IN_MIC: u32 = 0;
pub const PORTTYPE_IN_PCMAUDIO: u32 = 1;
pub const PORTTYPE_IN_VOICE: u32 = 2;
pub const PORTTYPE_OUT_PCMAUDIO: u32 = 3;
pub const PORTTYPE_OUT_VOICE: u32 = 4;
pub const PORTTYPE_OUT_SECONDARY: u32 = 5;

#[must_use]
pub fn is_known_port_type(t: u32) -> bool {
    (PORTTYPE_IN_MIC..=PORTTYPE_OUT_SECONDARY).contains(&t)
}

#[must_use]
pub fn is_input_port(t: u32) -> bool {
    matches!(t, PORTTYPE_IN_MIC | PORTTYPE_IN_PCMAUDIO | PORTTYPE_IN_VOICE)
}

#[must_use]
pub fn is_output_port(t: u32) -> bool {
    matches!(t, PORTTYPE_OUT_PCMAUDIO | PORTTYPE_OUT_VOICE | PORTTYPE_OUT_SECONDARY)
}

pub const SAMPLINGRATE_NULL: u32 = !0u32;
pub const SAMPLINGRATE_16000: u32 = 16000;

// =====================================================================
// Port limits (cellVoice.h:111-118)
// =====================================================================

pub const MAX_IN_VOICE_PORT: usize = 32;
pub const MAX_OUT_VOICE_PORT: usize = 4;
pub const GAME_1MB_MAX_IN_VOICE_PORT: usize = 8;
pub const GAME_1MB_MAX_OUT_VOICE_PORT: usize = 2;
pub const MAX_PORT: usize = 128;
pub const INVALID_PORT_ID: u32 = 0xFF;

// =====================================================================
// Domain types
// =====================================================================

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InitParam {
    pub event_mask: u32,
    pub version: u32,
    pub app_type: i32,
}

impl InitParam {
    fn validate(&self) -> Result<(), CellError> {
        if self.version != VERSION_100 {
            return Err(errors::ARGUMENT_INVALID);
        }
        if (self.event_mask & !EVENT_ALL_MASK) != 0 {
            return Err(errors::ARGUMENT_INVALID);
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PcmFormat {
    pub num_channels: u8,
    pub sample_alignment: u8,
    pub data_type: u32,
    pub sample_rate: u32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PortParam {
    pub port_type: u32,
    pub threshold: u16,
    pub b_mute: u16,
    pub volume: f32,
    pub bitrate: u32,
    pub buf_size: u32,
    pub pcm_format: PcmFormat,
    pub player_id: u32,
}

impl Default for PortParam {
    fn default() -> Self {
        Self {
            port_type: PORTTYPE_NULL,
            threshold: 0,
            b_mute: 0,
            volume: 1.0,
            bitrate: BITRATE_NULL,
            buf_size: 0,
            pcm_format: PcmFormat { num_channels: 1, sample_alignment: 0, data_type: PCM_NULL, sample_rate: SAMPLINGRATE_16000 },
            player_id: 0,
        }
    }
}

impl PortParam {
    fn validate(&self) -> Result<(), CellError> {
        if !is_known_port_type(self.port_type) {
            return Err(errors::PORT_INVALID);
        }
        match self.port_type {
            PORTTYPE_IN_VOICE | PORTTYPE_OUT_VOICE => {
                if !is_known_bitrate(self.bitrate) {
                    return Err(errors::ARGUMENT_INVALID);
                }
            }
            PORTTYPE_IN_PCMAUDIO | PORTTYPE_OUT_PCMAUDIO => {
                if self.buf_size == 0 {
                    return Err(errors::ARGUMENT_INVALID);
                }
                if self.pcm_format.num_channels == 0 {
                    return Err(errors::ARGUMENT_INVALID);
                }
                if self.pcm_format.sample_rate != SAMPLINGRATE_16000 {
                    return Err(errors::ARGUMENT_INVALID);
                }
                if !matches!(
                    self.pcm_format.data_type,
                    PCM_FLOAT | PCM_FLOAT_LITTLE_ENDIAN | PCM_SHORT | PCM_SHORT_LITTLE_ENDIAN | PCM_INTEGER | PCM_INTEGER_LITTLE_ENDIAN
                ) {
                    return Err(errors::ARGUMENT_INVALID);
                }
            }
            _ => {}
        }
        if !self.volume.is_finite() || self.volume < 0.0 || self.volume > 2.0 {
            return Err(errors::ARGUMENT_INVALID);
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BasePortInfo {
    pub port_type: u32,
    pub state: u32,
    pub num_edge: u16,
    pub num_byte: u32,
    pub frame_size: u32,
}

#[derive(Clone, Debug)]
pub struct Port {
    pub id: u32,
    pub state: u32,
    pub param: PortParam,
    pub edges_out: Vec<u32>, // IDs of output ports this port feeds
}

// =====================================================================
// VoiceManager
// =====================================================================

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SessionState {
    Uninitialized,
    Initialized,
    SessionRunning,
}

#[derive(Clone, Debug)]
pub struct VoiceManager {
    state: SessionState,
    app_type: i32,
    event_mask: u32,
    ports: Vec<Port>,
    next_id: u32,
}

impl VoiceManager {
    #[must_use]
    pub fn new() -> Self {
        Self {
            state: SessionState::Uninitialized,
            app_type: 0,
            event_mask: 0,
            ports: Vec::new(),
            next_id: 1,
        }
    }

    #[must_use]
    pub fn state(&self) -> SessionState {
        self.state
    }

    #[must_use]
    pub fn port_count(&self) -> usize {
        self.ports.len()
    }

    // ----------------- Lifecycle -----------------

    pub fn init(&mut self, param: InitParam) -> Result<(), CellError> {
        if self.state != SessionState::Uninitialized {
            return Err(errors::LIBVOICE_INITIALIZED);
        }
        param.validate()?;
        self.event_mask = param.event_mask;
        self.app_type = param.app_type;
        self.state = SessionState::Initialized;
        self.ports.clear();
        self.next_id = 1;
        Ok(())
    }

    pub fn end(&mut self) -> Result<(), CellError> {
        if self.state == SessionState::Uninitialized {
            return Err(errors::LIBVOICE_NOT_INIT);
        }
        self.state = SessionState::Uninitialized;
        self.ports.clear();
        Ok(())
    }

    // ----------------- Session control -----------------

    pub fn start_session(&mut self) -> Result<(), CellError> {
        if self.state == SessionState::Uninitialized {
            return Err(errors::LIBVOICE_NOT_INIT);
        }
        if self.state == SessionState::SessionRunning {
            return Err(errors::SERVICE_ATTACHED);
        }
        self.state = SessionState::SessionRunning;
        Ok(())
    }

    pub fn stop_session(&mut self) -> Result<(), CellError> {
        if self.state != SessionState::SessionRunning {
            return Err(errors::SERVICE_DETACHED);
        }
        self.state = SessionState::Initialized;
        for p in &mut self.ports {
            if p.state == PORTSTATE_RUNNING || p.state == PORTSTATE_BUFFERING {
                p.state = PORTSTATE_READY;
            }
        }
        Ok(())
    }

    // ----------------- Ports -----------------

    /// `cellVoiceCreatePort(param)`. Caps depend on `app_type`.
    pub fn create_port(&mut self, param: PortParam) -> Result<u32, CellError> {
        self.require_initialized()?;
        param.validate()?;

        // Respect per-app port limits.
        let (max_in_voice, max_out_voice) = if self.app_type as u32 == APPTYPE_GAME_1MB {
            (GAME_1MB_MAX_IN_VOICE_PORT, GAME_1MB_MAX_OUT_VOICE_PORT)
        } else {
            (MAX_IN_VOICE_PORT, MAX_OUT_VOICE_PORT)
        };
        if param.port_type == PORTTYPE_IN_VOICE {
            let cur = self.ports.iter().filter(|p| p.param.port_type == PORTTYPE_IN_VOICE).count();
            if cur >= max_in_voice {
                return Err(errors::RESOURCE_INSUFFICIENT);
            }
        }
        if param.port_type == PORTTYPE_OUT_VOICE {
            let cur = self.ports.iter().filter(|p| p.param.port_type == PORTTYPE_OUT_VOICE).count();
            if cur >= max_out_voice {
                return Err(errors::RESOURCE_INSUFFICIENT);
            }
        }
        if self.ports.len() >= MAX_PORT {
            return Err(errors::RESOURCE_INSUFFICIENT);
        }

        let id = self.next_id;
        self.next_id += 1;
        self.ports.push(Port { id, state: PORTSTATE_READY, param, edges_out: Vec::new() });
        Ok(id)
    }

    pub fn delete_port(&mut self, id: u32) -> Result<(), CellError> {
        self.require_initialized()?;
        let idx = self.port_idx(id)?;
        self.ports.remove(idx);
        // Drop any edges that referenced the deleted port.
        for p in &mut self.ports {
            p.edges_out.retain(|&x| x != id);
        }
        Ok(())
    }

    pub fn start_port(&mut self, id: u32) -> Result<(), CellError> {
        self.require_initialized()?;
        let idx = self.port_idx(id)?;
        if self.ports[idx].state != PORTSTATE_READY {
            return Err(errors::TOPOLOGY);
        }
        self.ports[idx].state = if self.state == SessionState::SessionRunning {
            PORTSTATE_RUNNING
        } else {
            PORTSTATE_BUFFERING
        };
        Ok(())
    }

    pub fn pause_port(&mut self, id: u32) -> Result<(), CellError> {
        self.require_initialized()?;
        let idx = self.port_idx(id)?;
        if self.ports[idx].state != PORTSTATE_RUNNING && self.ports[idx].state != PORTSTATE_BUFFERING {
            return Err(errors::TOPOLOGY);
        }
        self.ports[idx].state = PORTSTATE_READY;
        Ok(())
    }

    pub fn reset_port(&mut self, id: u32) -> Result<(), CellError> {
        self.require_initialized()?;
        let idx = self.port_idx(id)?;
        self.ports[idx].state = PORTSTATE_READY;
        Ok(())
    }

    /// `cellVoiceConnectIPortToOPort(in_id, out_id)`. Both ports must
    /// exist; `in_id` must be input, `out_id` must be output.
    pub fn connect_i_port_to_o_port(&mut self, in_id: u32, out_id: u32) -> Result<(), CellError> {
        self.require_initialized()?;
        let in_idx = self.port_idx(in_id)?;
        let out_idx = self.port_idx(out_id)?;
        if !is_input_port(self.ports[in_idx].param.port_type) {
            return Err(errors::TOPOLOGY);
        }
        if !is_output_port(self.ports[out_idx].param.port_type) {
            return Err(errors::TOPOLOGY);
        }
        if self.ports[in_idx].edges_out.contains(&out_id) {
            return Err(errors::SERVICE_ATTACHED);
        }
        self.ports[in_idx].edges_out.push(out_id);
        Ok(())
    }

    pub fn disconnect_i_port_from_o_port(&mut self, in_id: u32, out_id: u32) -> Result<(), CellError> {
        self.require_initialized()?;
        let in_idx = self.port_idx(in_id)?;
        let _ = self.port_idx(out_id)?;
        let pos = self.ports[in_idx]
            .edges_out
            .iter()
            .position(|&x| x == out_id)
            .ok_or(errors::SERVICE_DETACHED)?;
        self.ports[in_idx].edges_out.remove(pos);
        Ok(())
    }

    // ----------------- Port info / attr -----------------

    pub fn port_info(&self, id: u32) -> Result<BasePortInfo, CellError> {
        self.require_initialized()?;
        let idx = self.port_idx(id)?;
        let port = &self.ports[idx];
        let frame_size = match port.param.port_type {
            PORTTYPE_IN_VOICE | PORTTYPE_OUT_VOICE => 160, // 10ms @ 16kHz
            PORTTYPE_IN_PCMAUDIO | PORTTYPE_OUT_PCMAUDIO => port.param.buf_size,
            _ => 0,
        };
        Ok(BasePortInfo {
            port_type: port.param.port_type,
            state: port.state,
            num_edge: u16::try_from(port.edges_out.len()).unwrap_or(u16::MAX),
            num_byte: port.param.buf_size,
            frame_size,
        })
    }

    pub fn get_port_attr(&self, id: u32, attr: u32) -> Result<f32, CellError> {
        self.require_initialized()?;
        let idx = self.port_idx(id)?;
        let p = &self.ports[idx];
        match attr {
            ATTR_ENERGY_LEVEL => Ok(0.0),
            ATTR_VAD | ATTR_DTX | ATTR_AUTO_RESAMPLE => Ok(0.0),
            ATTR_LATENCY => Ok(0.0),
            ATTR_SILENCE_THRESHOLD => Ok(p.param.threshold as f32),
            _ => Err(errors::ARGUMENT_INVALID),
        }
    }

    pub fn set_port_attr(&mut self, id: u32, attr: u32, value: f32) -> Result<(), CellError> {
        self.require_initialized()?;
        if !value.is_finite() {
            return Err(errors::ARGUMENT_INVALID);
        }
        let idx = self.port_idx(id)?;
        match attr {
            ATTR_VAD | ATTR_DTX | ATTR_AUTO_RESAMPLE => Ok(()),
            ATTR_LATENCY => Ok(()),
            ATTR_SILENCE_THRESHOLD => {
                self.ports[idx].param.threshold = value.clamp(0.0, u16::MAX as f32) as u16;
                Ok(())
            }
            _ => Err(errors::ARGUMENT_INVALID),
        }
    }

    pub fn set_volume(&mut self, id: u32, volume: f32) -> Result<(), CellError> {
        self.require_initialized()?;
        if !volume.is_finite() || volume < 0.0 || volume > 2.0 {
            return Err(errors::ARGUMENT_INVALID);
        }
        let idx = self.port_idx(id)?;
        self.ports[idx].param.volume = volume;
        Ok(())
    }

    pub fn set_mute(&mut self, id: u32, muted: bool) -> Result<(), CellError> {
        self.require_initialized()?;
        let idx = self.port_idx(id)?;
        self.ports[idx].param.b_mute = u16::from(muted);
        Ok(())
    }

    fn require_initialized(&self) -> Result<(), CellError> {
        if self.state == SessionState::Uninitialized {
            Err(errors::LIBVOICE_NOT_INIT)
        } else {
            Ok(())
        }
    }

    fn port_idx(&self, id: u32) -> Result<usize, CellError> {
        if id == 0 || id == INVALID_PORT_ID {
            return Err(errors::PORT_INVALID);
        }
        self.ports.iter().position(|p| p.id == id).ok_or(errors::PORT_INVALID)
    }
}

impl Default for VoiceManager {
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

    fn ok_init() -> InitParam {
        InitParam { event_mask: EVENT_DATA_ERROR | EVENT_PORT_ATTACHED, version: VERSION_100, app_type: 0 }
    }

    fn voice_in_port() -> PortParam {
        PortParam { port_type: PORTTYPE_IN_VOICE, bitrate: BITRATE_16000, ..Default::default() }
    }

    fn voice_out_port() -> PortParam {
        PortParam { port_type: PORTTYPE_OUT_VOICE, bitrate: BITRATE_16000, ..Default::default() }
    }

    fn pcm_in_port() -> PortParam {
        PortParam {
            port_type: PORTTYPE_IN_PCMAUDIO,
            buf_size: 1024,
            pcm_format: PcmFormat {
                num_channels: 1,
                sample_alignment: 0,
                data_type: PCM_SHORT,
                sample_rate: SAMPLINGRATE_16000,
            },
            ..Default::default()
        }
    }

    fn mic_port() -> PortParam {
        PortParam { port_type: PORTTYPE_IN_MIC, ..Default::default() }
    }

    fn initialized() -> VoiceManager {
        let mut m = VoiceManager::new();
        m.init(ok_init()).unwrap();
        m
    }

    #[test]
    fn error_codes_byte_exact() {
        assert_eq!(errors::LIBVOICE_NOT_INIT.0, 0x8031_0801);
        assert_eq!(errors::LIBVOICE_INITIALIZED.0, 0x8031_0802);
        assert_eq!(errors::GENERAL.0, 0x8031_0803);
        assert_eq!(errors::PORT_INVALID.0, 0x8031_0804);
        assert_eq!(errors::ARGUMENT_INVALID.0, 0x8031_0805);
        assert_eq!(errors::TOPOLOGY.0, 0x8031_0807);
        assert_eq!(errors::RESOURCE_INSUFFICIENT.0, 0x8031_0808);
        assert_eq!(errors::ADDRESS_INVALID.0, 0x8031_080a);
        assert_eq!(errors::SERVICE_DETACHED.0, 0x8031_080b);
        assert_eq!(errors::SERVICE_ATTACHED.0, 0x8031_080c);
        assert_eq!(errors::DEVICE_NOT_PRESENT.0, 0x8031_0812);
    }

    #[test]
    fn bitrate_constants_stable() {
        assert_eq!(BITRATE_3850, 3850);
        assert_eq!(BITRATE_7300, 7300);
        assert_eq!(BITRATE_16000, 16000);
        assert_eq!(BITRATE_22533, 22533);
        assert_eq!(BITRATE_NULL, !0u32);
    }

    #[test]
    fn event_mask_bits_stable() {
        assert_eq!(EVENT_DATA_ERROR, 1);
        assert_eq!(EVENT_PORT_ATTACHED, 2);
        assert_eq!(EVENT_PORT_DETACHED, 4);
        assert_eq!(EVENT_SERVICE_ATTACHED, 8);
        assert_eq!(EVENT_SERVICE_DETACHED, 16);
        assert_eq!(EVENT_PORT_WEAK_ATTACHED, 32);
        assert_eq!(EVENT_PORT_WEAK_DETACHED, 64);
        assert_eq!(EVENT_ALL_MASK, 0x7F);
    }

    #[test]
    fn pcm_enum_stable() {
        assert_eq!(PCM_FLOAT, 0);
        assert_eq!(PCM_SHORT, 2);
        assert_eq!(PCM_INTEGER, 4);
        assert_eq!(PCM_NULL, !0u32);
    }

    #[test]
    fn port_state_enum_stable() {
        assert_eq!(PORTSTATE_IDLE, 0);
        assert_eq!(PORTSTATE_READY, 1);
        assert_eq!(PORTSTATE_BUFFERING, 2);
        assert_eq!(PORTSTATE_RUNNING, 3);
        assert_eq!(PORTSTATE_NULL, !0u32);
    }

    #[test]
    fn port_type_enum_stable() {
        assert_eq!(PORTTYPE_IN_MIC, 0);
        assert_eq!(PORTTYPE_IN_PCMAUDIO, 1);
        assert_eq!(PORTTYPE_IN_VOICE, 2);
        assert_eq!(PORTTYPE_OUT_PCMAUDIO, 3);
        assert_eq!(PORTTYPE_OUT_VOICE, 4);
        assert_eq!(PORTTYPE_OUT_SECONDARY, 5);
    }

    #[test]
    fn port_limits_stable() {
        assert_eq!(MAX_IN_VOICE_PORT, 32);
        assert_eq!(MAX_OUT_VOICE_PORT, 4);
        assert_eq!(GAME_1MB_MAX_IN_VOICE_PORT, 8);
        assert_eq!(GAME_1MB_MAX_OUT_VOICE_PORT, 2);
        assert_eq!(MAX_PORT, 128);
        assert_eq!(INVALID_PORT_ID, 0xFF);
    }

    #[test]
    fn is_input_output_helpers_stable() {
        assert!(is_input_port(PORTTYPE_IN_MIC));
        assert!(is_input_port(PORTTYPE_IN_VOICE));
        assert!(!is_input_port(PORTTYPE_OUT_VOICE));
        assert!(is_output_port(PORTTYPE_OUT_PCMAUDIO));
        assert!(!is_output_port(PORTTYPE_IN_PCMAUDIO));
    }

    #[test]
    fn init_happy_path() {
        let m = initialized();
        assert_eq!(m.state(), SessionState::Initialized);
    }

    #[test]
    fn init_bad_version_rejected() {
        let mut m = VoiceManager::new();
        let mut p = ok_init();
        p.version = 0;
        assert_eq!(m.init(p), Err(errors::ARGUMENT_INVALID));
    }

    #[test]
    fn init_bad_event_mask_rejected() {
        let mut m = VoiceManager::new();
        let mut p = ok_init();
        p.event_mask = 0x8000_0000;
        assert_eq!(m.init(p), Err(errors::ARGUMENT_INVALID));
    }

    #[test]
    fn init_twice_is_already_initialized() {
        let mut m = initialized();
        assert_eq!(m.init(ok_init()), Err(errors::LIBVOICE_INITIALIZED));
    }

    #[test]
    fn end_without_init_is_not_init() {
        let mut m = VoiceManager::new();
        assert_eq!(m.end(), Err(errors::LIBVOICE_NOT_INIT));
    }

    #[test]
    fn start_session_without_init_is_not_init() {
        let mut m = VoiceManager::new();
        assert_eq!(m.start_session(), Err(errors::LIBVOICE_NOT_INIT));
    }

    #[test]
    fn start_session_twice_is_service_attached() {
        let mut m = initialized();
        m.start_session().unwrap();
        assert_eq!(m.start_session(), Err(errors::SERVICE_ATTACHED));
    }

    #[test]
    fn stop_session_without_start_is_service_detached() {
        let mut m = initialized();
        assert_eq!(m.stop_session(), Err(errors::SERVICE_DETACHED));
    }

    #[test]
    fn session_start_stop_round_trip() {
        let mut m = initialized();
        m.start_session().unwrap();
        assert_eq!(m.state(), SessionState::SessionRunning);
        m.stop_session().unwrap();
        assert_eq!(m.state(), SessionState::Initialized);
    }

    #[test]
    fn create_port_voice_happy_path() {
        let mut m = initialized();
        let id = m.create_port(voice_in_port()).unwrap();
        assert_eq!(id, 1);
        assert_eq!(m.port_count(), 1);
    }

    #[test]
    fn create_port_bad_type_rejected() {
        let mut m = initialized();
        let p = PortParam { port_type: 42, ..Default::default() };
        assert_eq!(m.create_port(p), Err(errors::PORT_INVALID));
    }

    #[test]
    fn create_port_voice_bad_bitrate_rejected() {
        let mut m = initialized();
        let mut p = voice_in_port();
        p.bitrate = 12345;
        assert_eq!(m.create_port(p), Err(errors::ARGUMENT_INVALID));
    }

    #[test]
    fn create_port_pcm_zero_buf_rejected() {
        let mut m = initialized();
        let mut p = pcm_in_port();
        p.buf_size = 0;
        assert_eq!(m.create_port(p), Err(errors::ARGUMENT_INVALID));
    }

    #[test]
    fn create_port_pcm_bad_data_type_rejected() {
        let mut m = initialized();
        let mut p = pcm_in_port();
        p.pcm_format.data_type = 99;
        assert_eq!(m.create_port(p), Err(errors::ARGUMENT_INVALID));
    }

    #[test]
    fn create_port_pcm_bad_rate_rejected() {
        let mut m = initialized();
        let mut p = pcm_in_port();
        p.pcm_format.sample_rate = 44100;
        assert_eq!(m.create_port(p), Err(errors::ARGUMENT_INVALID));
    }

    #[test]
    fn create_port_bad_volume_rejected() {
        let mut m = initialized();
        let mut p = voice_in_port();
        p.volume = f32::NAN;
        assert_eq!(m.create_port(p), Err(errors::ARGUMENT_INVALID));
    }

    #[test]
    fn create_port_exceeds_voice_cap_rejected() {
        let mut m = initialized();
        for _ in 0..MAX_IN_VOICE_PORT {
            m.create_port(voice_in_port()).unwrap();
        }
        assert_eq!(m.create_port(voice_in_port()), Err(errors::RESOURCE_INSUFFICIENT));
    }

    #[test]
    fn game_1mb_app_type_enforces_tighter_caps() {
        let mut m = VoiceManager::new();
        let mut p = ok_init();
        p.app_type = APPTYPE_GAME_1MB as i32;
        m.init(p).unwrap();
        for _ in 0..GAME_1MB_MAX_IN_VOICE_PORT {
            m.create_port(voice_in_port()).unwrap();
        }
        assert_eq!(m.create_port(voice_in_port()), Err(errors::RESOURCE_INSUFFICIENT));
    }

    #[test]
    fn create_port_without_init_is_not_init() {
        let mut m = VoiceManager::new();
        assert_eq!(m.create_port(voice_in_port()), Err(errors::LIBVOICE_NOT_INIT));
    }

    #[test]
    fn delete_port_happy_path() {
        let mut m = initialized();
        let id = m.create_port(voice_in_port()).unwrap();
        m.delete_port(id).unwrap();
        assert_eq!(m.port_count(), 0);
    }

    #[test]
    fn delete_port_bad_id_rejected() {
        let mut m = initialized();
        assert_eq!(m.delete_port(0), Err(errors::PORT_INVALID));
        assert_eq!(m.delete_port(INVALID_PORT_ID), Err(errors::PORT_INVALID));
        assert_eq!(m.delete_port(999), Err(errors::PORT_INVALID));
    }

    #[test]
    fn start_port_transitions_state() {
        let mut m = initialized();
        let id = m.create_port(voice_in_port()).unwrap();
        m.start_port(id).unwrap();
        let info = m.port_info(id).unwrap();
        assert_eq!(info.state, PORTSTATE_BUFFERING);
        m.start_session().unwrap();
        // After session start new start_port → RUNNING (but port is already Buffering),
        // pause and restart to observe state change.
        m.pause_port(id).unwrap();
        m.start_port(id).unwrap();
        let info = m.port_info(id).unwrap();
        assert_eq!(info.state, PORTSTATE_RUNNING);
    }

    #[test]
    fn start_port_already_running_is_topology() {
        let mut m = initialized();
        let id = m.create_port(voice_in_port()).unwrap();
        m.start_port(id).unwrap();
        assert_eq!(m.start_port(id), Err(errors::TOPOLOGY));
    }

    #[test]
    fn pause_port_from_idle_is_topology() {
        let mut m = initialized();
        let id = m.create_port(voice_in_port()).unwrap();
        assert_eq!(m.pause_port(id), Err(errors::TOPOLOGY));
    }

    #[test]
    fn connect_input_to_output_happy_path() {
        let mut m = initialized();
        let i = m.create_port(voice_in_port()).unwrap();
        let o = m.create_port(voice_out_port()).unwrap();
        m.connect_i_port_to_o_port(i, o).unwrap();
        let info = m.port_info(i).unwrap();
        assert_eq!(info.num_edge, 1);
    }

    #[test]
    fn connect_duplicate_is_service_attached() {
        let mut m = initialized();
        let i = m.create_port(voice_in_port()).unwrap();
        let o = m.create_port(voice_out_port()).unwrap();
        m.connect_i_port_to_o_port(i, o).unwrap();
        assert_eq!(m.connect_i_port_to_o_port(i, o), Err(errors::SERVICE_ATTACHED));
    }

    #[test]
    fn connect_output_to_input_is_topology() {
        let mut m = initialized();
        let i = m.create_port(voice_in_port()).unwrap();
        let o = m.create_port(voice_out_port()).unwrap();
        assert_eq!(m.connect_i_port_to_o_port(o, i), Err(errors::TOPOLOGY));
    }

    #[test]
    fn disconnect_removes_edge() {
        let mut m = initialized();
        let i = m.create_port(voice_in_port()).unwrap();
        let o = m.create_port(voice_out_port()).unwrap();
        m.connect_i_port_to_o_port(i, o).unwrap();
        m.disconnect_i_port_from_o_port(i, o).unwrap();
        assert_eq!(m.port_info(i).unwrap().num_edge, 0);
    }

    #[test]
    fn disconnect_non_existent_edge_is_service_detached() {
        let mut m = initialized();
        let i = m.create_port(voice_in_port()).unwrap();
        let o = m.create_port(voice_out_port()).unwrap();
        assert_eq!(m.disconnect_i_port_from_o_port(i, o), Err(errors::SERVICE_DETACHED));
    }

    #[test]
    fn port_info_reports_frame_size_for_voice() {
        let mut m = initialized();
        let id = m.create_port(voice_in_port()).unwrap();
        let info = m.port_info(id).unwrap();
        assert_eq!(info.frame_size, 160);
    }

    #[test]
    fn port_info_reports_pcm_frame_size() {
        let mut m = initialized();
        let id = m.create_port(pcm_in_port()).unwrap();
        let info = m.port_info(id).unwrap();
        assert_eq!(info.frame_size, 1024);
    }

    #[test]
    fn set_volume_range_validated() {
        let mut m = initialized();
        let id = m.create_port(voice_in_port()).unwrap();
        m.set_volume(id, 0.5).unwrap();
        m.set_volume(id, 2.0).unwrap();
        assert_eq!(m.set_volume(id, -0.1), Err(errors::ARGUMENT_INVALID));
        assert_eq!(m.set_volume(id, 3.0), Err(errors::ARGUMENT_INVALID));
        assert_eq!(m.set_volume(id, f32::NAN), Err(errors::ARGUMENT_INVALID));
    }

    #[test]
    fn set_mute_roundtrip() {
        let mut m = initialized();
        let id = m.create_port(voice_in_port()).unwrap();
        m.set_mute(id, true).unwrap();
        m.set_mute(id, false).unwrap();
    }

    #[test]
    fn get_port_attr_known_and_unknown() {
        let mut m = initialized();
        let id = m.create_port(voice_in_port()).unwrap();
        assert_eq!(m.get_port_attr(id, ATTR_ENERGY_LEVEL), Ok(0.0));
        assert_eq!(m.get_port_attr(id, ATTR_SILENCE_THRESHOLD), Ok(0.0));
        assert_eq!(m.get_port_attr(id, 9999), Err(errors::ARGUMENT_INVALID));
    }

    #[test]
    fn set_port_attr_threshold_clamps_to_u16() {
        let mut m = initialized();
        let id = m.create_port(voice_in_port()).unwrap();
        m.set_port_attr(id, ATTR_SILENCE_THRESHOLD, 70000.0).unwrap();
        assert_eq!(m.get_port_attr(id, ATTR_SILENCE_THRESHOLD), Ok(u16::MAX as f32));
    }

    #[test]
    fn full_graph_smoke() {
        let mut m = initialized();
        let mic = m.create_port(mic_port()).unwrap();
        let voice_in = m.create_port(voice_in_port()).unwrap();
        let voice_out = m.create_port(voice_out_port()).unwrap();
        m.connect_i_port_to_o_port(mic, voice_out).unwrap();
        m.connect_i_port_to_o_port(voice_in, voice_out).unwrap();
        m.start_port(mic).unwrap();
        m.start_port(voice_in).unwrap();
        m.start_port(voice_out).unwrap();
        m.start_session().unwrap();
        m.stop_session().unwrap();
        m.end().unwrap();
    }
}
