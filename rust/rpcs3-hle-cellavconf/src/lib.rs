//! `rpcs3-hle-cellavconf` — audio-out / audio-in AV configuration HLE.
//!
//! Ports the two files that collaborate to form PS3's audio AV config
//! surface:
//!
//! - `rpcs3/Emu/Cell/Modules/cellAudioOut.cpp` — `cellAudioOut*`
//! - `rpcs3/Emu/Cell/Modules/cellAvconfExt.cpp` — USB/BT audio-in side
//!
//! Scope: state + device-info + sound-availability queries + configure.
//! The `audio_out_configuration` FXO in C++ holds two outputs (primary +
//! secondary), each with a cached [`SoundMode`] list; [`AvconfManager`]
//! reproduces that shape.
//!
//! ## Entry points covered
//!
//! | HLE function                          | Rust wrapper                       |
//! |---------------------------------------|------------------------------------|
//! | `cellAudioOutGetNumberOfDevice`       | [`cell_audio_out_get_number_of_device`] |
//! | `cellAudioOutGetState`                | [`cell_audio_out_get_state`]       |
//! | `cellAudioOutGetDeviceInfo`           | [`cell_audio_out_get_device_info`] |
//! | `cellAudioOutGetSoundAvailability`    | [`cell_audio_out_get_sound_availability`] |
//! | `cellAudioOutGetSoundAvailability2`   | [`cell_audio_out_get_sound_availability2`] |
//! | `cellAudioOutConfigure`               | [`cell_audio_out_configure`]       |
//! | `cellAudioOutGetConfiguration`        | [`cell_audio_out_get_configuration`] |
//! | `cellAudioOutSetCopyControl`          | [`cell_audio_out_set_copy_control`] |
//! | `cellAudioInGetNumberOfDevice`        | [`cell_audio_in_get_number_of_device`] |
//! | `cellAudioInGetDeviceInfo`            | [`cell_audio_in_get_device_info`]  |

use rpcs3_emu_types::CellError;

// =====================================================================
// Error codes — audio-out facility 0x8002b240 (cellAudioOut.h:7-17)
// =====================================================================

pub mod out_errors {
    use rpcs3_emu_types::CellError;

    pub const NOT_IMPLEMENTED: CellError = CellError(0x8002_b240);
    pub const ILLEGAL_CONFIGURATION: CellError = CellError(0x8002_b241);
    pub const ILLEGAL_PARAMETER: CellError = CellError(0x8002_b242);
    pub const PARAMETER_OUT_OF_RANGE: CellError = CellError(0x8002_b243);
    pub const DEVICE_NOT_FOUND: CellError = CellError(0x8002_b244);
    pub const UNSUPPORTED_AUDIO_OUT: CellError = CellError(0x8002_b245);
    pub const UNSUPPORTED_SOUND_MODE: CellError = CellError(0x8002_b246);
    pub const CONDITION_BUSY: CellError = CellError(0x8002_b247);
}

// =====================================================================
// Error codes — audio-in facility 0x8002b260 (cellAudioIn.h:7-16)
// =====================================================================

pub mod in_errors {
    use rpcs3_emu_types::CellError;

    pub const NOT_IMPLEMENTED: CellError = CellError(0x8002_b260);
    pub const ILLEGAL_CONFIGURATION: CellError = CellError(0x8002_b261);
    pub const ILLEGAL_PARAMETER: CellError = CellError(0x8002_b262);
    pub const PARAMETER_OUT_OF_RANGE: CellError = CellError(0x8002_b263);
    pub const DEVICE_NOT_FOUND: CellError = CellError(0x8002_b264);
    pub const UNSUPPORTED_AUDIO_IN: CellError = CellError(0x8002_b265);
    pub const UNSUPPORTED_SOUND_MODE: CellError = CellError(0x8002_b266);
    pub const CONDITION_BUSY: CellError = CellError(0x8002_b267);
}

// =====================================================================
// CellAudioOut constants
// =====================================================================

pub const AUDIO_OUT_PRIMARY: u32 = 0;
pub const AUDIO_OUT_SECONDARY: u32 = 1;

pub const DOWNMIXER_NONE: u32 = 0;
pub const DOWNMIXER_TYPE_A: u32 = 1;
pub const DOWNMIXER_TYPE_B: u32 = 2;

pub const PORT_HDMI: u8 = 0;
pub const PORT_SPDIF: u8 = 1;
pub const PORT_ANALOG: u8 = 2;
pub const PORT_USB: u8 = 3;
pub const PORT_BLUETOOTH: u8 = 4;
pub const PORT_NETWORK: u8 = 5;

pub const DEVICE_STATE_UNAVAILABLE: u8 = 0;
pub const DEVICE_STATE_AVAILABLE: u8 = 1;

pub const OUTPUT_STATE_ENABLED: u32 = 0;
pub const OUTPUT_STATE_DISABLED: u32 = 1;
pub const OUTPUT_STATE_PREPARING: u32 = 2;

pub const CODING_LPCM: u8 = 0;
pub const CODING_AC3: u8 = 1;
pub const CODING_MPEG1: u8 = 2;
pub const CODING_MP3: u8 = 3;
pub const CODING_MPEG2: u8 = 4;
pub const CODING_AAC: u8 = 5;
pub const CODING_DTS: u8 = 6;
pub const CODING_ATRAC: u8 = 7;
pub const CODING_DOLBY_TRUE_HD: u8 = 8;
pub const CODING_DOLBY_DIGITAL_PLUS: u8 = 9;
pub const CODING_DTS_HD_HIGHRES: u8 = 10;
pub const CODING_DTS_HD_MASTER: u8 = 11;
pub const CODING_BITSTREAM: u8 = 0xff;

pub const CHNUM_2: u8 = 2;
pub const CHNUM_4: u8 = 4;
pub const CHNUM_6: u8 = 6;
pub const CHNUM_8: u8 = 8;

pub const FS_32KHZ: u8 = 0x01;
pub const FS_44KHZ: u8 = 0x02;
pub const FS_48KHZ: u8 = 0x04;
pub const FS_88KHZ: u8 = 0x08;
pub const FS_96KHZ: u8 = 0x10;
pub const FS_176KHZ: u8 = 0x20;
pub const FS_192KHZ: u8 = 0x40;

pub const SPEAKER_LAYOUT_DEFAULT: u32 = 0x0000_0000;
pub const SPEAKER_LAYOUT_2CH: u32 = 0x0000_0001;
pub const SPEAKER_LAYOUT_6CH: u32 = 0x0001_0000;
pub const SPEAKER_LAYOUT_8CH: u32 = 0x4000_0000;

pub const COPY_CONTROL_FREE: u32 = 0;
pub const COPY_CONTROL_ONCE: u32 = 1;
pub const COPY_CONTROL_NEVER: u32 = 2;

// =====================================================================
// CellAudioIn constants — audio capture side (mics, BT headsets)
// =====================================================================

pub const IN_PORT_USB: u8 = 3;
pub const IN_PORT_BLUETOOTH: u8 = 4;

pub const IN_CODING_LPCM: u8 = 0;

pub const IN_CHNUM_NONE: u8 = 0;
pub const IN_CHNUM_1: u8 = 1;
pub const IN_CHNUM_2: u8 = 2;

pub const IN_FS_UNDEFINED: u16 = 0x00;
pub const IN_FS_8KHZ: u16 = 0x01;
pub const IN_FS_12KHZ: u16 = 0x02;
pub const IN_FS_16KHZ: u16 = 0x04;
pub const IN_FS_24KHZ: u16 = 0x08;
pub const IN_FS_32KHZ: u16 = 0x10;
pub const IN_FS_48KHZ: u16 = 0x20;

// =====================================================================
// Domain types — shadow state
// =====================================================================

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct SoundMode {
    pub coding_type: u8, // audio-out coding constants
    pub channel: u8,
    pub fs: u8,       // bitfield, only one bit set in a sound_mode entry
    pub reserved: u8, // kept byte-exact with C++ layout
    pub layout: u32,  // speaker layout — be_t<u32> in C++
}

impl SoundMode {
    #[must_use]
    pub const fn lpcm_stereo_48khz() -> Self {
        Self {
            coding_type: CODING_LPCM,
            channel: CHNUM_2,
            fs: FS_48KHZ,
            reserved: 0,
            layout: SPEAKER_LAYOUT_2CH,
        }
    }

    #[must_use]
    pub const fn lpcm_7_1_48khz() -> Self {
        Self {
            coding_type: CODING_LPCM,
            channel: CHNUM_8,
            fs: FS_48KHZ,
            reserved: 0,
            layout: SPEAKER_LAYOUT_8CH,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AudioOutConfiguration {
    pub channel: u8,
    pub encoder: u8,
    pub down_mixer: u32,
}

impl Default for AudioOutConfiguration {
    fn default() -> Self {
        Self { channel: CHNUM_2, encoder: CODING_LPCM, down_mixer: DOWNMIXER_NONE }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct AudioOutState {
    pub state: u32,
    pub encoder: u8,
    pub down_mixer: u32,
    pub sound_mode: SoundMode,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AudioOutDeviceInfo {
    pub port_type: u8,
    pub available_mode_count: u8,
    pub state: u8,
    pub latency: u16,
    pub available_modes: Vec<SoundMode>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AudioInDeviceInfo {
    pub port_type: u8,
    pub available_mode_count: u8,
    pub state: u8,
    pub device_number: u8,
    pub device_id: u64,
    pub device_type: u64,
    pub name: String,
    pub available_modes: Vec<AudioInSoundMode>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct AudioInSoundMode {
    pub coding_type: u8,
    pub channel: u8,
    pub fs: u16,
}

// =====================================================================
// AvconfManager — models `audio_out_configuration` FXO + avconf_manager
// =====================================================================

#[derive(Clone, Debug)]
pub struct AudioOutPort {
    pub state: u32,
    pub channels: u8,
    pub encoder: u8,
    pub down_mixer: u32,
    pub copy_control: u32,
    pub sound_modes: Vec<SoundMode>,
    pub sound_mode: SoundMode,
}

impl AudioOutPort {
    fn primary_defaults() -> Self {
        // Primary HDMI defaults: LPCM 2ch 48kHz, encoder LPCM, downmix none,
        // sound_modes populated with LPCM stereo as the single fallback.
        let sound_mode = SoundMode::lpcm_stereo_48khz();
        Self {
            state: OUTPUT_STATE_ENABLED,
            channels: CHNUM_2,
            encoder: CODING_LPCM,
            down_mixer: DOWNMIXER_NONE,
            copy_control: COPY_CONTROL_FREE,
            sound_modes: vec![sound_mode],
            sound_mode,
        }
    }

    fn secondary_defaults() -> Self {
        // Secondary peripheral output: single fixed mode, stereo only.
        Self::primary_defaults()
    }
}

#[derive(Clone, Debug)]
pub struct AvconfManager {
    pub out: [AudioOutPort; 2],
    pub in_devices: Vec<AudioInDeviceInfo>,
}

impl AvconfManager {
    #[must_use]
    pub fn new() -> Self {
        Self { out: [AudioOutPort::primary_defaults(), AudioOutPort::secondary_defaults()], in_devices: Vec::new() }
    }

    pub fn register_in_device(&mut self, device: AudioInDeviceInfo) {
        self.in_devices.push(device);
    }

    // ----------------- cellAudioOutGetNumberOfDevice -----------------

    /// Returns number of devices on `audio_out`:
    /// primary → 1, secondary → 0, any other index → illegal-parameter.
    pub fn audio_out_number_of_devices(&self, audio_out: u32) -> Result<u32, CellError> {
        match audio_out {
            AUDIO_OUT_PRIMARY => Ok(1),
            AUDIO_OUT_SECONDARY => Ok(0),
            _ => Err(out_errors::ILLEGAL_PARAMETER),
        }
    }

    // ----------------- cellAudioOutGetSoundAvailability --------------

    /// Returns the max channel count of a mode matching `(coding_type, fs)`
    /// on `audio_out`. 0 means unsupported. Mirrors the primary-only branch
    /// in C++; invalid `audio_out` returns 0 (not an error) too.
    #[must_use]
    pub fn audio_out_sound_availability(&self, audio_out: u32, coding_type: u8, fs: u8) -> u32 {
        let Some(port) = self.port(audio_out) else { return 0 };
        if audio_out != AUDIO_OUT_PRIMARY {
            return 0;
        }
        let mut best = 0u32;
        for mode in &port.sound_modes {
            if mode.coding_type == coding_type && mode.fs == fs {
                best = best.max(u32::from(mode.channel));
            }
        }
        best
    }

    /// Checks exact match of `(coding_type, fs, channel)`. Returns `ch` if
    /// found, 0 otherwise. Non-primary → 0.
    #[must_use]
    pub fn audio_out_sound_availability2(&self, audio_out: u32, coding_type: u8, fs: u8, channel: u8) -> u32 {
        let Some(port) = self.port(audio_out) else { return 0 };
        if audio_out != AUDIO_OUT_PRIMARY {
            return 0;
        }
        for mode in &port.sound_modes {
            if mode.coding_type == coding_type && mode.fs == fs && mode.channel == channel {
                return u32::from(channel);
            }
        }
        0
    }

    // ----------------- cellAudioOutGetState --------------------------

    /// Returns the shadow `AudioOutState` for (audio_out, device_index).
    pub fn audio_out_state(&self, audio_out: u32, device_index: u32) -> Result<AudioOutState, CellError> {
        let num = self.audio_out_number_of_devices(audio_out)?;
        if device_index >= num {
            // C++ branch for SECONDARY returns a pseudo-state, primary returns
            // PARAMETER_OUT_OF_RANGE. We mirror that shape.
            if audio_out == AUDIO_OUT_SECONDARY {
                return Ok(AudioOutState {
                    state: 0x10,
                    encoder: 0,
                    down_mixer: 0,
                    sound_mode: SoundMode { layout: 0xD00C_1680, ..SoundMode::default() },
                });
            }
            return Err(out_errors::PARAMETER_OUT_OF_RANGE);
        }
        let port = self.port(audio_out).ok_or(out_errors::ILLEGAL_PARAMETER)?;
        Ok(AudioOutState {
            state: port.state,
            encoder: port.encoder,
            down_mixer: port.down_mixer,
            sound_mode: port.sound_mode,
        })
    }

    // ----------------- cellAudioOutGetDeviceInfo ---------------------

    /// Returns hardcoded HDMI device info with the port's sound modes.
    pub fn audio_out_device_info(&self, audio_out: u32, device_index: u32) -> Result<AudioOutDeviceInfo, CellError> {
        let num = self.audio_out_number_of_devices(audio_out)?;
        if device_index >= num {
            if audio_out == AUDIO_OUT_SECONDARY {
                // C++ zeroes the output struct and returns OK.
                return Ok(AudioOutDeviceInfo {
                    port_type: 0,
                    available_mode_count: 0,
                    state: 0,
                    latency: 0,
                    available_modes: Vec::new(),
                });
            }
            return Err(out_errors::PARAMETER_OUT_OF_RANGE);
        }
        let port = self.port(audio_out).ok_or(out_errors::ILLEGAL_PARAMETER)?;
        if port.sound_modes.len() > 16 {
            return Err(out_errors::ILLEGAL_CONFIGURATION);
        }
        Ok(AudioOutDeviceInfo {
            port_type: PORT_HDMI,
            available_mode_count: u8::try_from(port.sound_modes.len()).unwrap_or(0),
            state: DEVICE_STATE_AVAILABLE,
            latency: 13,
            available_modes: port.sound_modes.clone(),
        })
    }

    // ----------------- cellAudioOutConfigure -------------------------

    /// Applies new config to primary. Invalid downMixer values are silently
    /// dropped, matching C++. Returns Ok with a flag indicating whether a
    /// reset pipeline should run. The caller handles the side-effect.
    pub fn audio_out_configure(&mut self, audio_out: u32, config: &AudioOutConfiguration) -> Result<bool, CellError> {
        match audio_out {
            AUDIO_OUT_PRIMARY => {}
            AUDIO_OUT_SECONDARY => return Err(out_errors::UNSUPPORTED_AUDIO_OUT),
            _ => return Err(out_errors::ILLEGAL_PARAMETER),
        }
        let port = &mut self.out[audio_out as usize];
        let mut needs_reset = false;
        if port.channels != config.channel || port.encoder != config.encoder || port.down_mixer != config.down_mixer {
            port.channels = config.channel;
            port.encoder = config.encoder;
            if config.down_mixer <= DOWNMIXER_TYPE_B {
                port.down_mixer = config.down_mixer;
            }
            // Try to find the best matching sound mode, else keep current.
            if let Some(found) =
                port.sound_modes.iter().find(|m| m.coding_type == port.encoder && m.channel == port.channels).copied()
            {
                port.sound_mode = found;
            }
            needs_reset = true;
        }
        Ok(needs_reset)
    }

    /// Returns active config (primary only; secondary → UNSUPPORTED).
    pub fn audio_out_get_configuration(&self, audio_out: u32) -> Result<AudioOutConfiguration, CellError> {
        match audio_out {
            AUDIO_OUT_PRIMARY => {}
            AUDIO_OUT_SECONDARY => return Err(out_errors::UNSUPPORTED_AUDIO_OUT),
            _ => return Err(out_errors::ILLEGAL_PARAMETER),
        }
        let port = &self.out[audio_out as usize];
        Ok(AudioOutConfiguration { channel: port.channels, encoder: port.encoder, down_mixer: port.down_mixer })
    }

    // ----------------- cellAudioOutSetCopyControl --------------------

    pub fn audio_out_set_copy_control(&mut self, audio_out: u32, control: u32) -> Result<(), CellError> {
        if control > COPY_CONTROL_NEVER {
            return Err(out_errors::ILLEGAL_PARAMETER);
        }
        match audio_out {
            AUDIO_OUT_PRIMARY => {
                self.out[0].copy_control = control;
                Ok(())
            }
            AUDIO_OUT_SECONDARY => Err(out_errors::UNSUPPORTED_AUDIO_OUT),
            _ => Err(out_errors::ILLEGAL_PARAMETER),
        }
    }

    // ----------------- cellAudioInGetNumberOfDevice ------------------

    /// Number of registered audio-in devices. (Matches
    /// `avconf_manager::devices.size()` in C++.)
    #[must_use]
    pub fn audio_in_number_of_devices(&self) -> u32 {
        u32::try_from(self.in_devices.len()).unwrap_or(u32::MAX)
    }

    pub fn audio_in_device_info(&self, device_index: u32) -> Result<AudioInDeviceInfo, CellError> {
        let idx = usize::try_from(device_index).map_err(|_| in_errors::PARAMETER_OUT_OF_RANGE)?;
        self.in_devices.get(idx).cloned().ok_or(in_errors::PARAMETER_OUT_OF_RANGE)
    }

    fn port(&self, audio_out: u32) -> Option<&AudioOutPort> {
        self.out.get(audio_out as usize)
    }
}

impl Default for AvconfManager {
    fn default() -> Self {
        Self::new()
    }
}

// =====================================================================
// Free-function HLE wrappers — thin re-exports for the emu-core dispatch
// =====================================================================

pub fn cell_audio_out_get_number_of_device(mgr: &AvconfManager, audio_out: u32) -> Result<u32, CellError> {
    mgr.audio_out_number_of_devices(audio_out)
}

pub fn cell_audio_out_get_sound_availability(mgr: &AvconfManager, audio_out: u32, coding_type: u8, fs: u8) -> u32 {
    mgr.audio_out_sound_availability(audio_out, coding_type, fs)
}

pub fn cell_audio_out_get_sound_availability2(
    mgr: &AvconfManager,
    audio_out: u32,
    coding_type: u8,
    fs: u8,
    channel: u8,
) -> u32 {
    mgr.audio_out_sound_availability2(audio_out, coding_type, fs, channel)
}

pub fn cell_audio_out_get_state(mgr: &AvconfManager, audio_out: u32, device_index: u32) -> Result<AudioOutState, CellError> {
    mgr.audio_out_state(audio_out, device_index)
}

pub fn cell_audio_out_get_device_info(mgr: &AvconfManager, audio_out: u32, device_index: u32) -> Result<AudioOutDeviceInfo, CellError> {
    mgr.audio_out_device_info(audio_out, device_index)
}

pub fn cell_audio_out_configure(mgr: &mut AvconfManager, audio_out: u32, config: &AudioOutConfiguration) -> Result<bool, CellError> {
    mgr.audio_out_configure(audio_out, config)
}

pub fn cell_audio_out_get_configuration(mgr: &AvconfManager, audio_out: u32) -> Result<AudioOutConfiguration, CellError> {
    mgr.audio_out_get_configuration(audio_out)
}

pub fn cell_audio_out_set_copy_control(mgr: &mut AvconfManager, audio_out: u32, control: u32) -> Result<(), CellError> {
    mgr.audio_out_set_copy_control(audio_out, control)
}

pub fn cell_audio_in_get_number_of_device(mgr: &AvconfManager) -> u32 {
    mgr.audio_in_number_of_devices()
}

pub fn cell_audio_in_get_device_info(mgr: &AvconfManager, device_index: u32) -> Result<AudioInDeviceInfo, CellError> {
    mgr.audio_in_device_info(device_index)
}

// =====================================================================
// Tests
// =====================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn mgr_with_rich_modes() -> AvconfManager {
        let mut mgr = AvconfManager::new();
        mgr.out[0].sound_modes = vec![
            SoundMode { coding_type: CODING_LPCM, channel: CHNUM_2, fs: FS_48KHZ, reserved: 0, layout: SPEAKER_LAYOUT_2CH },
            SoundMode { coding_type: CODING_LPCM, channel: CHNUM_6, fs: FS_48KHZ, reserved: 0, layout: SPEAKER_LAYOUT_6CH },
            SoundMode { coding_type: CODING_LPCM, channel: CHNUM_8, fs: FS_48KHZ, reserved: 0, layout: SPEAKER_LAYOUT_8CH },
            SoundMode { coding_type: CODING_AC3, channel: CHNUM_6, fs: FS_48KHZ, reserved: 0, layout: SPEAKER_LAYOUT_6CH },
            SoundMode { coding_type: CODING_DTS, channel: CHNUM_6, fs: FS_48KHZ, reserved: 0, layout: SPEAKER_LAYOUT_6CH },
        ];
        mgr.out[0].sound_mode = mgr.out[0].sound_modes[0];
        mgr
    }

    #[test]
    fn error_codes_byte_exact() {
        assert_eq!(out_errors::NOT_IMPLEMENTED.0, 0x8002_b240);
        assert_eq!(out_errors::ILLEGAL_PARAMETER.0, 0x8002_b242);
        assert_eq!(out_errors::UNSUPPORTED_AUDIO_OUT.0, 0x8002_b245);
        assert_eq!(out_errors::CONDITION_BUSY.0, 0x8002_b247);
        assert_eq!(in_errors::NOT_IMPLEMENTED.0, 0x8002_b260);
        assert_eq!(in_errors::PARAMETER_OUT_OF_RANGE.0, 0x8002_b263);
        assert_eq!(in_errors::CONDITION_BUSY.0, 0x8002_b267);
    }

    #[test]
    fn port_type_constants_stable() {
        assert_eq!(PORT_HDMI, 0);
        assert_eq!(PORT_SPDIF, 1);
        assert_eq!(PORT_ANALOG, 2);
        assert_eq!(PORT_USB, 3);
        assert_eq!(PORT_BLUETOOTH, 4);
        assert_eq!(PORT_NETWORK, 5);
    }

    #[test]
    fn coding_type_constants_stable() {
        assert_eq!(CODING_LPCM, 0);
        assert_eq!(CODING_AC3, 1);
        assert_eq!(CODING_AAC, 5);
        assert_eq!(CODING_ATRAC, 7);
        assert_eq!(CODING_BITSTREAM, 0xff);
    }

    #[test]
    fn fs_bitmask_constants_stable() {
        assert_eq!(FS_32KHZ, 0x01);
        assert_eq!(FS_44KHZ, 0x02);
        assert_eq!(FS_48KHZ, 0x04);
        assert_eq!(FS_88KHZ, 0x08);
        assert_eq!(FS_96KHZ, 0x10);
        assert_eq!(FS_176KHZ, 0x20);
        assert_eq!(FS_192KHZ, 0x40);
    }

    #[test]
    fn number_of_devices_primary_1_secondary_0() {
        let mgr = AvconfManager::new();
        assert_eq!(mgr.audio_out_number_of_devices(AUDIO_OUT_PRIMARY), Ok(1));
        assert_eq!(mgr.audio_out_number_of_devices(AUDIO_OUT_SECONDARY), Ok(0));
    }

    #[test]
    fn number_of_devices_unknown_is_illegal_param() {
        let mgr = AvconfManager::new();
        assert_eq!(mgr.audio_out_number_of_devices(2), Err(out_errors::ILLEGAL_PARAMETER));
        assert_eq!(mgr.audio_out_number_of_devices(0xFFFF), Err(out_errors::ILLEGAL_PARAMETER));
    }

    #[test]
    fn sound_availability_reports_max_channels_for_matching_mode() {
        let mgr = mgr_with_rich_modes();
        assert_eq!(mgr.audio_out_sound_availability(AUDIO_OUT_PRIMARY, CODING_LPCM, FS_48KHZ), u32::from(CHNUM_8));
        assert_eq!(mgr.audio_out_sound_availability(AUDIO_OUT_PRIMARY, CODING_AC3, FS_48KHZ), u32::from(CHNUM_6));
    }

    #[test]
    fn sound_availability_unsupported_combo_returns_zero() {
        let mgr = mgr_with_rich_modes();
        assert_eq!(mgr.audio_out_sound_availability(AUDIO_OUT_PRIMARY, CODING_DTS, FS_96KHZ), 0);
        assert_eq!(mgr.audio_out_sound_availability(AUDIO_OUT_PRIMARY, CODING_MPEG2, FS_48KHZ), 0);
    }

    #[test]
    fn sound_availability_secondary_returns_zero() {
        let mgr = mgr_with_rich_modes();
        assert_eq!(mgr.audio_out_sound_availability(AUDIO_OUT_SECONDARY, CODING_LPCM, FS_48KHZ), 0);
    }

    #[test]
    fn sound_availability2_exact_match_returns_channel() {
        let mgr = mgr_with_rich_modes();
        assert_eq!(mgr.audio_out_sound_availability2(AUDIO_OUT_PRIMARY, CODING_LPCM, FS_48KHZ, CHNUM_6), u32::from(CHNUM_6));
    }

    #[test]
    fn sound_availability2_channel_mismatch_returns_zero() {
        let mgr = mgr_with_rich_modes();
        assert_eq!(mgr.audio_out_sound_availability2(AUDIO_OUT_PRIMARY, CODING_LPCM, FS_48KHZ, CHNUM_4), 0);
    }

    #[test]
    fn get_state_primary_returns_shadow_state() {
        let mgr = AvconfManager::new();
        let state = mgr.audio_out_state(AUDIO_OUT_PRIMARY, 0).expect("valid");
        assert_eq!(state.state, OUTPUT_STATE_ENABLED);
        assert_eq!(state.encoder, CODING_LPCM);
        assert_eq!(state.down_mixer, DOWNMIXER_NONE);
        assert_eq!(state.sound_mode, SoundMode::lpcm_stereo_48khz());
    }

    #[test]
    fn get_state_secondary_out_of_range_returns_pseudo_state() {
        // Secondary has 0 devices; device_index=0 >= 0 → the C++ "uninitialized"
        // branch, which we fill with the fixed snapshot.
        let mgr = AvconfManager::new();
        let state = mgr.audio_out_state(AUDIO_OUT_SECONDARY, 0).expect("OK path");
        assert_eq!(state.state, 0x10);
        assert_eq!(state.sound_mode.layout, 0xD00C_1680);
    }

    #[test]
    fn get_state_primary_out_of_range_is_out_of_range() {
        let mgr = AvconfManager::new();
        assert_eq!(mgr.audio_out_state(AUDIO_OUT_PRIMARY, 1), Err(out_errors::PARAMETER_OUT_OF_RANGE));
    }

    #[test]
    fn get_state_unknown_audio_out_is_illegal_param() {
        let mgr = AvconfManager::new();
        assert_eq!(mgr.audio_out_state(9, 0), Err(out_errors::ILLEGAL_PARAMETER));
    }

    #[test]
    fn get_device_info_returns_hdmi_with_all_modes() {
        let mgr = mgr_with_rich_modes();
        let info = mgr.audio_out_device_info(AUDIO_OUT_PRIMARY, 0).expect("valid");
        assert_eq!(info.port_type, PORT_HDMI);
        assert_eq!(info.state, DEVICE_STATE_AVAILABLE);
        assert_eq!(info.latency, 13);
        assert_eq!(info.available_mode_count, 5);
        assert_eq!(info.available_modes.len(), 5);
    }

    #[test]
    fn get_device_info_secondary_out_of_range_returns_zeroed() {
        let mgr = AvconfManager::new();
        let info = mgr.audio_out_device_info(AUDIO_OUT_SECONDARY, 0).expect("OK path");
        assert_eq!(info.available_mode_count, 0);
        assert_eq!(info.available_modes.len(), 0);
    }

    #[test]
    fn configure_primary_updates_channels_encoder_downmixer() {
        let mut mgr = mgr_with_rich_modes();
        let cfg = AudioOutConfiguration { channel: CHNUM_6, encoder: CODING_LPCM, down_mixer: DOWNMIXER_TYPE_A };
        let reset = mgr.audio_out_configure(AUDIO_OUT_PRIMARY, &cfg).expect("primary configurable");
        assert!(reset);
        let port = &mgr.out[0];
        assert_eq!(port.channels, CHNUM_6);
        assert_eq!(port.encoder, CODING_LPCM);
        assert_eq!(port.down_mixer, DOWNMIXER_TYPE_A);
        assert_eq!(port.sound_mode.channel, CHNUM_6);
    }

    #[test]
    fn configure_ignores_invalid_downmixer() {
        let mut mgr = mgr_with_rich_modes();
        let cfg = AudioOutConfiguration { channel: CHNUM_8, encoder: CODING_LPCM, down_mixer: 99 };
        let _ = mgr.audio_out_configure(AUDIO_OUT_PRIMARY, &cfg).expect("primary configurable");
        // Kept the default (NONE) since 99 > TYPE_B.
        assert_eq!(mgr.out[0].down_mixer, DOWNMIXER_NONE);
    }

    #[test]
    fn configure_noop_does_not_request_reset() {
        let mut mgr = AvconfManager::new();
        let cfg = AudioOutConfiguration::default();
        let reset = mgr.audio_out_configure(AUDIO_OUT_PRIMARY, &cfg).expect("primary configurable");
        assert!(!reset);
    }

    #[test]
    fn configure_secondary_returns_unsupported() {
        let mut mgr = AvconfManager::new();
        let cfg = AudioOutConfiguration::default();
        assert_eq!(mgr.audio_out_configure(AUDIO_OUT_SECONDARY, &cfg), Err(out_errors::UNSUPPORTED_AUDIO_OUT));
    }

    #[test]
    fn configure_unknown_returns_illegal_param() {
        let mut mgr = AvconfManager::new();
        let cfg = AudioOutConfiguration::default();
        assert_eq!(mgr.audio_out_configure(7, &cfg), Err(out_errors::ILLEGAL_PARAMETER));
    }

    #[test]
    fn get_configuration_returns_live_state() {
        let mut mgr = AvconfManager::new();
        let initial = mgr.audio_out_get_configuration(AUDIO_OUT_PRIMARY).expect("primary OK");
        assert_eq!(initial, AudioOutConfiguration::default());
        let new_cfg = AudioOutConfiguration { channel: CHNUM_8, encoder: CODING_LPCM, down_mixer: DOWNMIXER_TYPE_B };
        mgr.audio_out_configure(AUDIO_OUT_PRIMARY, &new_cfg).unwrap();
        let live = mgr.audio_out_get_configuration(AUDIO_OUT_PRIMARY).expect("primary OK");
        assert_eq!(live, new_cfg);
    }

    #[test]
    fn get_configuration_secondary_is_unsupported() {
        let mgr = AvconfManager::new();
        assert_eq!(mgr.audio_out_get_configuration(AUDIO_OUT_SECONDARY), Err(out_errors::UNSUPPORTED_AUDIO_OUT));
    }

    #[test]
    fn set_copy_control_accepts_all_three_values_on_primary() {
        let mut mgr = AvconfManager::new();
        for c in [COPY_CONTROL_FREE, COPY_CONTROL_ONCE, COPY_CONTROL_NEVER] {
            assert!(mgr.audio_out_set_copy_control(AUDIO_OUT_PRIMARY, c).is_ok());
            assert_eq!(mgr.out[0].copy_control, c);
        }
    }

    #[test]
    fn set_copy_control_bad_value_is_illegal_param() {
        let mut mgr = AvconfManager::new();
        assert_eq!(mgr.audio_out_set_copy_control(AUDIO_OUT_PRIMARY, 99), Err(out_errors::ILLEGAL_PARAMETER));
    }

    #[test]
    fn set_copy_control_secondary_is_unsupported() {
        let mut mgr = AvconfManager::new();
        assert_eq!(
            mgr.audio_out_set_copy_control(AUDIO_OUT_SECONDARY, COPY_CONTROL_FREE),
            Err(out_errors::UNSUPPORTED_AUDIO_OUT)
        );
    }

    #[test]
    fn audio_in_empty_by_default() {
        let mgr = AvconfManager::new();
        assert_eq!(mgr.audio_in_number_of_devices(), 0);
        assert_eq!(mgr.audio_in_device_info(0), Err(in_errors::PARAMETER_OUT_OF_RANGE));
    }

    #[test]
    fn register_and_lookup_in_device() {
        let mut mgr = AvconfManager::new();
        mgr.register_in_device(AudioInDeviceInfo {
            port_type: IN_PORT_USB,
            available_mode_count: 1,
            state: DEVICE_STATE_AVAILABLE,
            device_number: 0,
            device_id: 0xE11C_C0DE,
            device_type: 0xC0DE_E11C,
            name: "USB Mic".into(),
            available_modes: vec![AudioInSoundMode {
                coding_type: IN_CODING_LPCM,
                channel: IN_CHNUM_2,
                fs: IN_FS_8KHZ | IN_FS_16KHZ | IN_FS_48KHZ,
            }],
        });
        assert_eq!(mgr.audio_in_number_of_devices(), 1);
        let dev = mgr.audio_in_device_info(0).expect("registered");
        assert_eq!(dev.port_type, IN_PORT_USB);
        assert_eq!(dev.name, "USB Mic");
        assert_eq!(dev.available_modes.len(), 1);
        assert_eq!(dev.available_modes[0].fs, IN_FS_8KHZ | IN_FS_16KHZ | IN_FS_48KHZ);
    }

    #[test]
    fn register_two_in_devices_preserves_order() {
        let mut mgr = AvconfManager::new();
        mgr.register_in_device(AudioInDeviceInfo {
            port_type: IN_PORT_USB,
            available_mode_count: 1,
            state: DEVICE_STATE_AVAILABLE,
            device_number: 0,
            device_id: 0x1,
            device_type: 0x2,
            name: "A".into(),
            available_modes: Vec::new(),
        });
        mgr.register_in_device(AudioInDeviceInfo {
            port_type: IN_PORT_BLUETOOTH,
            available_mode_count: 1,
            state: DEVICE_STATE_AVAILABLE,
            device_number: 1,
            device_id: 0x3,
            device_type: 0x4,
            name: "B".into(),
            available_modes: Vec::new(),
        });
        assert_eq!(mgr.audio_in_number_of_devices(), 2);
        assert_eq!(mgr.audio_in_device_info(0).unwrap().name, "A");
        assert_eq!(mgr.audio_in_device_info(1).unwrap().name, "B");
    }

    #[test]
    fn free_function_wrappers_delegate_to_manager() {
        let mut mgr = AvconfManager::new();
        assert_eq!(cell_audio_out_get_number_of_device(&mgr, AUDIO_OUT_PRIMARY), Ok(1));
        assert_eq!(
            cell_audio_out_get_sound_availability(&mgr, AUDIO_OUT_PRIMARY, CODING_LPCM, FS_48KHZ),
            u32::from(CHNUM_2)
        );
        assert_eq!(cell_audio_in_get_number_of_device(&mgr), 0);
        let _ = cell_audio_out_configure(
            &mut mgr,
            AUDIO_OUT_PRIMARY,
            &AudioOutConfiguration { channel: CHNUM_2, encoder: CODING_LPCM, down_mixer: DOWNMIXER_NONE },
        );
    }

    #[test]
    fn sound_mode_default_is_silence() {
        let m = SoundMode::default();
        assert_eq!(m.coding_type, 0);
        assert_eq!(m.channel, 0);
        assert_eq!(m.fs, 0);
        assert_eq!(m.layout, 0);
    }
}
