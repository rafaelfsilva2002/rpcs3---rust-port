//! `rpcs3-hle-cellsubdisplay` — Second-display / Remote Play HLE.
//!
//! Ports `rpcs3/Emu/Cell/Modules/cellSubDisplay.cpp`. cellSubDisplay is
//! the framework the PS3 used to stream a secondary video/audio feed to a
//! connected PSP (via Remote Play). The API shape is:
//!
//! 1. `Init(param)` — allocate memory container + establish version.
//! 2. `Start(callback)` — register the peer handler.
//! 3. `SetVideoMemory` / `AudioOutBlocking` / `AudioOut` — feed frames.
//! 4. `End` — teardown.
//!
//! ## Entry points covered
//!
//! | HLE function                         | Rust wrapper                          |
//! |--------------------------------------|---------------------------------------|
//! | `cellSubDisplayInit`                 | [`SubDisplay::init`]                  |
//! | `cellSubDisplayEnd`                  | [`SubDisplay::end`]                   |
//! | `cellSubDisplayStart`                | [`SubDisplay::start`]                 |
//! | `cellSubDisplayStop`                 | [`SubDisplay::stop`]                  |
//! | `cellSubDisplayGetRequiredMemory`    | [`SubDisplay::required_memory`]       |
//! | `cellSubDisplayAudioOutBlocking`     | [`SubDisplay::audio_out_blocking`]    |
//! | `cellSubDisplayAudioOut`             | [`SubDisplay::audio_out`]             |
//! | `cellSubDisplaySetVideoMemory`       | [`SubDisplay::set_video_memory`]      |
//! | `cellSubDisplayGetPeerList`          | [`SubDisplay::peer_list`]             |
//! | `cellSubDisplayGetPeerNum`           | [`SubDisplay::peer_num`]              |

use rpcs3_emu_types::CellError;

// =====================================================================
// Error codes — byte-exact with cellSubDisplay.h:4-15
// =====================================================================

pub mod errors {
    use rpcs3_emu_types::CellError;

    pub const OUT_OF_MEMORY: CellError = CellError(0x8002_9851);
    pub const FATAL: CellError = CellError(0x8002_9852);
    pub const NOT_FOUND: CellError = CellError(0x8002_9853);
    pub const INVALID_VALUE: CellError = CellError(0x8002_9854);
    pub const NOT_INITIALIZED: CellError = CellError(0x8002_9855);
    pub const NOT_SUPPORTED: CellError = CellError(0x8002_9856);
    pub const SET_SAMPLE: CellError = CellError(0x8002_9860);
    pub const AUDIOOUT_IS_BUSY: CellError = CellError(0x8002_9861);
    pub const ZERO_REGISTERED: CellError = CellError(0x8002_9813);
}

// =====================================================================
// Constants — byte-exact with cellSubDisplay.h:18-53
// =====================================================================

// Status codes delivered to the handler callback.
pub const STATUS_JOIN: i32 = 1;
pub const STATUS_LEAVE: i32 = 2;
pub const STATUS_FATALERROR: i32 = 3;

// Protocol version.
pub const VERSION_0001: i32 = 1;
pub const VERSION_0002: i32 = 2;
pub const VERSION_0003: i32 = 3;

// Streaming mode (only Remote Play is supported).
pub const MODE_REMOTEPLAY: i32 = 1;

// Video formats.
pub const VIDEO_FORMAT_A8R8G8B8: i32 = 1;
pub const VIDEO_FORMAT_R8G8B8A8: i32 = 2;
pub const VIDEO_FORMAT_YUV420: i32 = 3;

// Aspect ratios.
pub const VIDEO_ASPECT_RATIO_16_9: i32 = 0;
pub const VIDEO_ASPECT_RATIO_4_3: i32 = 1;

// Video / audio capture modes.
pub const VIDEO_MODE_SETDATA: i32 = 0;
pub const VIDEO_MODE_CAPTURE: i32 = 1;
pub const AUDIO_MODE_SETDATA: i32 = 0;
pub const AUDIO_MODE_CAPTURE: i32 = 1;

// Memory container sizes (match C++ exactly).
pub const MEMORY_CONTAINER_SIZE_0001: u32 = 8 * 1024 * 1024;
pub const MEMORY_CONTAINER_SIZE_0002: u32 = 10 * 1024 * 1024;
pub const MEMORY_CONTAINER_SIZE_0003: u32 = 10 * 1024 * 1024;

// Fixed v0003 framebuffer geometry.
pub const V0003_WIDTH: u32 = 864;
pub const V0003_PITCH: u32 = 864;
pub const V0003_HEIGHT: u32 = 480;

pub const NICKNAME_LEN: usize = 256;
pub const PSPID_LEN: usize = 16;

// Touch
pub const TOUCH_STATUS_NONE: u8 = 0;
pub const TOUCH_STATUS_PRESS: u8 = 1;
pub const TOUCH_STATUS_RELEASE: u8 = 2;
pub const TOUCH_STATUS_MOVE: u8 = 3;
pub const TOUCH_STATUS_ABORT: u8 = 4;
pub const TOUCH_MAX_TOUCH_INFO: usize = 6;

// Peer-list hardware limit: Remote Play supports 1 simultaneous peer.
pub const MAX_PEERS: usize = 1;

// =====================================================================
// Domain types
// =====================================================================

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VideoParam {
    pub format: i32,
    pub width: i32,
    pub height: i32,
    pub pitch: i32,
    pub aspect_ratio: i32,
    pub video_mode: i32,
}

impl VideoParam {
    fn validate(&self) -> Result<(), CellError> {
        if ![VIDEO_FORMAT_A8R8G8B8, VIDEO_FORMAT_R8G8B8A8, VIDEO_FORMAT_YUV420].contains(&self.format) {
            return Err(errors::INVALID_VALUE);
        }
        if self.width <= 0 || self.height <= 0 || self.pitch <= 0 {
            return Err(errors::INVALID_VALUE);
        }
        if self.pitch < self.width {
            return Err(errors::INVALID_VALUE);
        }
        if ![VIDEO_ASPECT_RATIO_16_9, VIDEO_ASPECT_RATIO_4_3].contains(&self.aspect_ratio) {
            return Err(errors::INVALID_VALUE);
        }
        if ![VIDEO_MODE_SETDATA, VIDEO_MODE_CAPTURE].contains(&self.video_mode) {
            return Err(errors::INVALID_VALUE);
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AudioParam {
    pub ch: i32,
    pub audio_mode: i32,
}

impl AudioParam {
    fn validate(&self) -> Result<(), CellError> {
        if !(1..=8).contains(&self.ch) {
            return Err(errors::INVALID_VALUE);
        }
        if ![AUDIO_MODE_SETDATA, AUDIO_MODE_CAPTURE].contains(&self.audio_mode) {
            return Err(errors::INVALID_VALUE);
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SubDisplayParam {
    pub version: i32,
    pub mode: i32,
    pub n_group: i32,
    pub n_peer: i32,
    pub video: VideoParam,
    pub audio: AudioParam,
}

impl SubDisplayParam {
    fn validate(&self) -> Result<(), CellError> {
        if !matches!(self.version, VERSION_0001 | VERSION_0002 | VERSION_0003) {
            return Err(errors::INVALID_VALUE);
        }
        if self.mode != MODE_REMOTEPLAY {
            return Err(errors::NOT_SUPPORTED);
        }
        if self.n_group < 1 || self.n_peer < 1 {
            return Err(errors::INVALID_VALUE);
        }
        if self.n_peer as usize > MAX_PEERS {
            // Remote Play is documented as supporting 1 peer only.
            return Err(errors::INVALID_VALUE);
        }
        self.video.validate()?;
        self.audio.validate()?;
        Ok(())
    }

    #[must_use]
    pub fn required_memory(&self) -> u32 {
        match self.version {
            VERSION_0001 => MEMORY_CONTAINER_SIZE_0001,
            VERSION_0002 => MEMORY_CONTAINER_SIZE_0002,
            VERSION_0003 => MEMORY_CONTAINER_SIZE_0003,
            _ => 0,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct PeerInfo {
    pub session_id: u64,
    pub port_no: u32,
    pub psp_id: [u8; PSPID_LEN],
    pub psp_nickname: String, // len ≤ NICKNAME_LEN
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct TouchInfo {
    pub status: u8,
    pub force: u8,
    pub x: u16,
    pub y: u16,
}

// =====================================================================
// FSM — Closed → Initialized → Running → Initialized → Closed
// =====================================================================

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    Closed,
    Initialized,
    Running,
}

// =====================================================================
// SubDisplay manager
// =====================================================================

#[derive(Clone, Debug)]
pub struct SubDisplay {
    state: State,
    param: Option<SubDisplayParam>,
    peers: Vec<PeerInfo>,
    audio_busy: bool,
    handler_registered: bool,
    video_memory_addr: Option<u64>,
}

impl SubDisplay {
    #[must_use]
    pub fn new() -> Self {
        Self {
            state: State::Closed,
            param: None,
            peers: Vec::new(),
            audio_busy: false,
            handler_registered: false,
            video_memory_addr: None,
        }
    }

    #[must_use]
    pub fn state(&self) -> State {
        self.state
    }

    // ----------------- Static-ish query -----------------

    /// `cellSubDisplayGetRequiredMemory(param)` is callable even before Init.
    pub fn required_memory(param: &SubDisplayParam) -> Result<u32, CellError> {
        param.validate()?;
        Ok(param.required_memory())
    }

    // ----------------- Lifecycle -----------------

    /// `cellSubDisplayInit(param, callback, userData)`.
    pub fn init(&mut self, param: SubDisplayParam) -> Result<(), CellError> {
        if self.state != State::Closed {
            return Err(errors::FATAL);
        }
        param.validate()?;
        self.param = Some(param);
        self.state = State::Initialized;
        self.peers.clear();
        self.audio_busy = false;
        self.handler_registered = false;
        self.video_memory_addr = None;
        Ok(())
    }

    pub fn end(&mut self) -> Result<(), CellError> {
        if self.state == State::Closed {
            return Err(errors::NOT_INITIALIZED);
        }
        self.state = State::Closed;
        self.param = None;
        self.peers.clear();
        self.audio_busy = false;
        self.handler_registered = false;
        self.video_memory_addr = None;
        Ok(())
    }

    /// `cellSubDisplayStart` registers the peer-event handler and moves
    /// the FSM to Running. Must have been Init'd first.
    pub fn start(&mut self) -> Result<(), CellError> {
        if self.state != State::Initialized {
            return Err(errors::NOT_INITIALIZED);
        }
        self.state = State::Running;
        self.handler_registered = true;
        Ok(())
    }

    pub fn stop(&mut self) -> Result<(), CellError> {
        if self.state != State::Running {
            return Err(errors::NOT_INITIALIZED);
        }
        self.state = State::Initialized;
        Ok(())
    }

    // ----------------- Peer management -----------------

    pub fn peer_num(&self) -> Result<u32, CellError> {
        self.require_initialized()?;
        Ok(u32::try_from(self.peers.len()).unwrap_or(u32::MAX))
    }

    pub fn peer_list(&self) -> Result<&[PeerInfo], CellError> {
        self.require_initialized()?;
        Ok(&self.peers)
    }

    /// Injects a peer JOIN; the real lib receives this via the underlying
    /// Remote Play session. Test backends call this directly.
    pub fn peer_join(&mut self, peer: PeerInfo) -> Result<i32, CellError> {
        self.require_running()?;
        if peer.psp_nickname.len() > NICKNAME_LEN {
            return Err(errors::INVALID_VALUE);
        }
        if self.peers.len() >= MAX_PEERS {
            return Err(errors::OUT_OF_MEMORY);
        }
        self.peers.push(peer);
        Ok(STATUS_JOIN)
    }

    pub fn peer_leave(&mut self, session_id: u64) -> Result<i32, CellError> {
        self.require_running()?;
        let pos = self.peers.iter().position(|p| p.session_id == session_id).ok_or(errors::NOT_FOUND)?;
        self.peers.remove(pos);
        Ok(STATUS_LEAVE)
    }

    // ----------------- Audio -----------------

    /// `cellSubDisplayAudioOutBlocking(group_id, audio_data)`. audio_data
    /// must be 16-bit samples packed to `ch` channels.
    pub fn audio_out_blocking(&mut self, group_id: i32, audio_data: &[i16]) -> Result<(), CellError> {
        self.require_running()?;
        if group_id < 0 {
            return Err(errors::INVALID_VALUE);
        }
        let param = self.param.as_ref().ok_or(errors::NOT_INITIALIZED)?;
        if audio_data.is_empty() {
            return Err(errors::SET_SAMPLE);
        }
        if audio_data.len() % param.audio.ch as usize != 0 {
            return Err(errors::SET_SAMPLE);
        }
        if self.audio_busy {
            return Err(errors::AUDIOOUT_IS_BUSY);
        }
        // Non-blocking returns immediately; the blocking call waits for
        // drain. We model both on a shared path; the tests decide by
        // flipping the `audio_busy` flag via `begin_audio_out` below.
        Ok(())
    }

    pub fn audio_out(&mut self, group_id: i32, audio_data: &[i16]) -> Result<(), CellError> {
        self.require_running()?;
        if group_id < 0 {
            return Err(errors::INVALID_VALUE);
        }
        if self.audio_busy {
            return Err(errors::AUDIOOUT_IS_BUSY);
        }
        let param = self.param.as_ref().ok_or(errors::NOT_INITIALIZED)?;
        if audio_data.is_empty() {
            return Err(errors::SET_SAMPLE);
        }
        if audio_data.len() % param.audio.ch as usize != 0 {
            return Err(errors::SET_SAMPLE);
        }
        self.audio_busy = true;
        Ok(())
    }

    pub fn audio_out_finish(&mut self) {
        self.audio_busy = false;
    }

    // ----------------- Video memory -----------------

    pub fn set_video_memory(&mut self, group_id: i32, addr: u64) -> Result<(), CellError> {
        self.require_running()?;
        if group_id < 0 {
            return Err(errors::INVALID_VALUE);
        }
        if addr == 0 {
            return Err(errors::INVALID_VALUE);
        }
        self.video_memory_addr = Some(addr);
        Ok(())
    }

    #[must_use]
    pub fn video_memory_addr(&self) -> Option<u64> {
        self.video_memory_addr
    }

    fn require_initialized(&self) -> Result<(), CellError> {
        if self.state == State::Closed {
            Err(errors::NOT_INITIALIZED)
        } else {
            Ok(())
        }
    }

    fn require_running(&self) -> Result<(), CellError> {
        if self.state == State::Running {
            Ok(())
        } else {
            Err(errors::NOT_INITIALIZED)
        }
    }
}

impl Default for SubDisplay {
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

    fn ok_param() -> SubDisplayParam {
        SubDisplayParam {
            version: VERSION_0003,
            mode: MODE_REMOTEPLAY,
            n_group: 1,
            n_peer: 1,
            video: VideoParam {
                format: VIDEO_FORMAT_A8R8G8B8,
                width: V0003_WIDTH as i32,
                height: V0003_HEIGHT as i32,
                pitch: V0003_PITCH as i32,
                aspect_ratio: VIDEO_ASPECT_RATIO_16_9,
                video_mode: VIDEO_MODE_SETDATA,
            },
            audio: AudioParam { ch: 2, audio_mode: AUDIO_MODE_SETDATA },
        }
    }

    #[test]
    fn error_codes_byte_exact() {
        assert_eq!(errors::OUT_OF_MEMORY.0, 0x8002_9851);
        assert_eq!(errors::FATAL.0, 0x8002_9852);
        assert_eq!(errors::NOT_FOUND.0, 0x8002_9853);
        assert_eq!(errors::INVALID_VALUE.0, 0x8002_9854);
        assert_eq!(errors::NOT_INITIALIZED.0, 0x8002_9855);
        assert_eq!(errors::NOT_SUPPORTED.0, 0x8002_9856);
        assert_eq!(errors::SET_SAMPLE.0, 0x8002_9860);
        assert_eq!(errors::AUDIOOUT_IS_BUSY.0, 0x8002_9861);
        assert_eq!(errors::ZERO_REGISTERED.0, 0x8002_9813);
    }

    #[test]
    fn status_constants_stable() {
        assert_eq!(STATUS_JOIN, 1);
        assert_eq!(STATUS_LEAVE, 2);
        assert_eq!(STATUS_FATALERROR, 3);
    }

    #[test]
    fn version_constants_stable() {
        assert_eq!(VERSION_0001, 1);
        assert_eq!(VERSION_0002, 2);
        assert_eq!(VERSION_0003, 3);
    }

    #[test]
    fn video_format_constants_stable() {
        assert_eq!(VIDEO_FORMAT_A8R8G8B8, 1);
        assert_eq!(VIDEO_FORMAT_R8G8B8A8, 2);
        assert_eq!(VIDEO_FORMAT_YUV420, 3);
    }

    #[test]
    fn aspect_ratio_constants_stable() {
        assert_eq!(VIDEO_ASPECT_RATIO_16_9, 0);
        assert_eq!(VIDEO_ASPECT_RATIO_4_3, 1);
    }

    #[test]
    fn touch_status_constants_stable() {
        assert_eq!(TOUCH_STATUS_NONE, 0);
        assert_eq!(TOUCH_STATUS_PRESS, 1);
        assert_eq!(TOUCH_STATUS_RELEASE, 2);
        assert_eq!(TOUCH_STATUS_MOVE, 3);
        assert_eq!(TOUCH_STATUS_ABORT, 4);
        assert_eq!(TOUCH_MAX_TOUCH_INFO, 6);
    }

    #[test]
    fn memory_container_sizes_stable() {
        assert_eq!(MEMORY_CONTAINER_SIZE_0001, 8 * 1024 * 1024);
        assert_eq!(MEMORY_CONTAINER_SIZE_0002, 10 * 1024 * 1024);
        assert_eq!(MEMORY_CONTAINER_SIZE_0003, 10 * 1024 * 1024);
    }

    #[test]
    fn v0003_framebuffer_constants_stable() {
        assert_eq!(V0003_WIDTH, 864);
        assert_eq!(V0003_PITCH, 864);
        assert_eq!(V0003_HEIGHT, 480);
    }

    #[test]
    fn nickname_pspid_len_stable() {
        assert_eq!(NICKNAME_LEN, 256);
        assert_eq!(PSPID_LEN, 16);
    }

    #[test]
    fn required_memory_maps_version_correctly() {
        let mut p = ok_param();
        p.version = VERSION_0001;
        assert_eq!(SubDisplay::required_memory(&p), Ok(MEMORY_CONTAINER_SIZE_0001));
        p.version = VERSION_0002;
        assert_eq!(SubDisplay::required_memory(&p), Ok(MEMORY_CONTAINER_SIZE_0002));
        p.version = VERSION_0003;
        assert_eq!(SubDisplay::required_memory(&p), Ok(MEMORY_CONTAINER_SIZE_0003));
    }

    #[test]
    fn required_memory_rejects_invalid_version() {
        let mut p = ok_param();
        p.version = 42;
        assert_eq!(SubDisplay::required_memory(&p), Err(errors::INVALID_VALUE));
    }

    #[test]
    fn init_happy_path() {
        let mut sd = SubDisplay::new();
        sd.init(ok_param()).unwrap();
        assert_eq!(sd.state(), State::Initialized);
    }

    #[test]
    fn init_twice_is_fatal() {
        let mut sd = SubDisplay::new();
        sd.init(ok_param()).unwrap();
        assert_eq!(sd.init(ok_param()), Err(errors::FATAL));
    }

    #[test]
    fn init_invalid_mode_is_not_supported() {
        let mut p = ok_param();
        p.mode = 42;
        let mut sd = SubDisplay::new();
        assert_eq!(sd.init(p), Err(errors::NOT_SUPPORTED));
    }

    #[test]
    fn init_invalid_video_format_is_invalid_value() {
        let mut p = ok_param();
        p.video.format = 99;
        let mut sd = SubDisplay::new();
        assert_eq!(sd.init(p), Err(errors::INVALID_VALUE));
    }

    #[test]
    fn init_pitch_less_than_width_rejected() {
        let mut p = ok_param();
        p.video.pitch = p.video.width / 2;
        let mut sd = SubDisplay::new();
        assert_eq!(sd.init(p), Err(errors::INVALID_VALUE));
    }

    #[test]
    fn init_invalid_audio_channel_count_rejected() {
        let mut p = ok_param();
        p.audio.ch = 99;
        let mut sd = SubDisplay::new();
        assert_eq!(sd.init(p), Err(errors::INVALID_VALUE));
    }

    #[test]
    fn init_n_peer_over_max_rejected() {
        let mut p = ok_param();
        p.n_peer = 2;
        let mut sd = SubDisplay::new();
        assert_eq!(sd.init(p), Err(errors::INVALID_VALUE));
    }

    #[test]
    fn end_without_init_is_not_initialized() {
        let mut sd = SubDisplay::new();
        assert_eq!(sd.end(), Err(errors::NOT_INITIALIZED));
    }

    #[test]
    fn end_from_initialized_ok() {
        let mut sd = SubDisplay::new();
        sd.init(ok_param()).unwrap();
        sd.end().unwrap();
        assert_eq!(sd.state(), State::Closed);
    }

    #[test]
    fn start_from_initialized_transitions_to_running() {
        let mut sd = SubDisplay::new();
        sd.init(ok_param()).unwrap();
        sd.start().unwrap();
        assert_eq!(sd.state(), State::Running);
    }

    #[test]
    fn start_without_init_is_not_initialized() {
        let mut sd = SubDisplay::new();
        assert_eq!(sd.start(), Err(errors::NOT_INITIALIZED));
    }

    #[test]
    fn stop_round_trip() {
        let mut sd = SubDisplay::new();
        sd.init(ok_param()).unwrap();
        sd.start().unwrap();
        sd.stop().unwrap();
        assert_eq!(sd.state(), State::Initialized);
    }

    #[test]
    fn peer_num_requires_init() {
        let sd = SubDisplay::new();
        assert_eq!(sd.peer_num(), Err(errors::NOT_INITIALIZED));
    }

    #[test]
    fn peer_join_only_when_running() {
        let mut sd = SubDisplay::new();
        sd.init(ok_param()).unwrap();
        assert_eq!(
            sd.peer_join(PeerInfo { session_id: 1, port_no: 42, psp_id: [0; 16], psp_nickname: "Test".into() }),
            Err(errors::NOT_INITIALIZED)
        );
        sd.start().unwrap();
        sd.peer_join(PeerInfo { session_id: 1, port_no: 42, psp_id: [0; 16], psp_nickname: "Test".into() })
            .unwrap();
        assert_eq!(sd.peer_num().unwrap(), 1);
    }

    #[test]
    fn peer_join_nickname_over_limit_rejected() {
        let mut sd = SubDisplay::new();
        sd.init(ok_param()).unwrap();
        sd.start().unwrap();
        let too_long = "a".repeat(NICKNAME_LEN + 1);
        assert_eq!(
            sd.peer_join(PeerInfo { session_id: 1, port_no: 1, psp_id: [0; 16], psp_nickname: too_long }),
            Err(errors::INVALID_VALUE)
        );
    }

    #[test]
    fn peer_join_beyond_max_is_out_of_memory() {
        let mut sd = SubDisplay::new();
        sd.init(ok_param()).unwrap();
        sd.start().unwrap();
        sd.peer_join(PeerInfo { session_id: 1, port_no: 1, psp_id: [0; 16], psp_nickname: "A".into() }).unwrap();
        assert_eq!(
            sd.peer_join(PeerInfo { session_id: 2, port_no: 2, psp_id: [0; 16], psp_nickname: "B".into() }),
            Err(errors::OUT_OF_MEMORY)
        );
    }

    #[test]
    fn peer_leave_not_found() {
        let mut sd = SubDisplay::new();
        sd.init(ok_param()).unwrap();
        sd.start().unwrap();
        assert_eq!(sd.peer_leave(42), Err(errors::NOT_FOUND));
    }

    #[test]
    fn peer_join_then_leave_cycle() {
        let mut sd = SubDisplay::new();
        sd.init(ok_param()).unwrap();
        sd.start().unwrap();
        sd.peer_join(PeerInfo { session_id: 7, port_no: 3, psp_id: [0; 16], psp_nickname: "Bob".into() }).unwrap();
        assert_eq!(sd.peer_leave(7), Ok(STATUS_LEAVE));
        assert_eq!(sd.peer_num().unwrap(), 0);
    }

    #[test]
    fn audio_out_requires_running() {
        let mut sd = SubDisplay::new();
        sd.init(ok_param()).unwrap();
        assert_eq!(sd.audio_out(0, &[0; 64]), Err(errors::NOT_INITIALIZED));
    }

    #[test]
    fn audio_out_empty_data_set_sample() {
        let mut sd = SubDisplay::new();
        sd.init(ok_param()).unwrap();
        sd.start().unwrap();
        assert_eq!(sd.audio_out(0, &[]), Err(errors::SET_SAMPLE));
    }

    #[test]
    fn audio_out_misaligned_to_channels_set_sample() {
        let mut sd = SubDisplay::new();
        sd.init(ok_param()).unwrap();
        sd.start().unwrap();
        // ch=2, so data.len() must be even
        assert_eq!(sd.audio_out(0, &[0; 5]), Err(errors::SET_SAMPLE));
    }

    #[test]
    fn audio_out_marks_busy_until_finish() {
        let mut sd = SubDisplay::new();
        sd.init(ok_param()).unwrap();
        sd.start().unwrap();
        sd.audio_out(0, &[0; 2]).unwrap();
        assert_eq!(sd.audio_out(0, &[0; 2]), Err(errors::AUDIOOUT_IS_BUSY));
        sd.audio_out_finish();
        sd.audio_out(0, &[0; 2]).unwrap();
    }

    #[test]
    fn audio_out_negative_group_invalid() {
        let mut sd = SubDisplay::new();
        sd.init(ok_param()).unwrap();
        sd.start().unwrap();
        assert_eq!(sd.audio_out(-1, &[0; 2]), Err(errors::INVALID_VALUE));
    }

    #[test]
    fn audio_out_blocking_happy_path() {
        let mut sd = SubDisplay::new();
        sd.init(ok_param()).unwrap();
        sd.start().unwrap();
        sd.audio_out_blocking(0, &[0; 4]).unwrap();
    }

    #[test]
    fn set_video_memory_happy_path() {
        let mut sd = SubDisplay::new();
        sd.init(ok_param()).unwrap();
        sd.start().unwrap();
        sd.set_video_memory(0, 0xC000_0000).unwrap();
        assert_eq!(sd.video_memory_addr(), Some(0xC000_0000));
    }

    #[test]
    fn set_video_memory_zero_addr_rejected() {
        let mut sd = SubDisplay::new();
        sd.init(ok_param()).unwrap();
        sd.start().unwrap();
        assert_eq!(sd.set_video_memory(0, 0), Err(errors::INVALID_VALUE));
    }

    #[test]
    fn set_video_memory_before_start_rejected() {
        let mut sd = SubDisplay::new();
        sd.init(ok_param()).unwrap();
        assert_eq!(sd.set_video_memory(0, 0xC000_0000), Err(errors::NOT_INITIALIZED));
    }

    #[test]
    fn full_lifecycle_smoke() {
        let mut sd = SubDisplay::new();
        assert_eq!(SubDisplay::required_memory(&ok_param()).unwrap(), MEMORY_CONTAINER_SIZE_0003);
        sd.init(ok_param()).unwrap();
        sd.start().unwrap();
        sd.peer_join(PeerInfo { session_id: 1, port_no: 1, psp_id: [0xAA; 16], psp_nickname: "Psp".into() }).unwrap();
        sd.set_video_memory(0, 0xC010_0000).unwrap();
        sd.audio_out(0, &[0; 64]).unwrap();
        sd.audio_out_finish();
        sd.peer_leave(1).unwrap();
        sd.stop().unwrap();
        sd.end().unwrap();
    }
}
