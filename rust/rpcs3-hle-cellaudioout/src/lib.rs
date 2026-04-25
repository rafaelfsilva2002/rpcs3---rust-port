//! Rust port of `rpcs3/Emu/Cell/Modules/cellAudioOut.cpp` — PS3 audio-out
//! configuration + sound-mode negotiation HLE (10 entries, 590 lines C++).
//!
//! Surface:
//! * `cellAudioOutGetNumberOfDevice/State/Configuration/SoundAvailability/2/
//!    DeviceInfo/Configure/SetCopyControl/Register+UnregisterCallback`.
//!
//! Models the dual-output (PRIMARY=0, SECONDARY=1) configuration with
//! sound-mode tables. Init seeds modes based on PSF SOUND_FORMAT flags +
//! audio_format config — preserved via pluggable `SoundFormatSource` trait.
//!
//! `no_std` + `alloc`. Single dep: `rpcs3-emu-types`.

#![no_std]

extern crate alloc;

use alloc::vec::Vec;
use rpcs3_emu_types::CellError;

pub const HOST_MODULE_NAME: &str = "cellSysutil";
pub const SUBMODULE_NAME: &str = "cellAudioOut";

/// 10 FNIDs in REG_FUNC order (cpp:579+).
pub const REGISTERED_ENTRY_POINTS: &[&str] = &[
    "cellAudioOutGetState",
    "cellAudioOutConfigure",
    "cellAudioOutGetSoundAvailability",
    "cellAudioOutGetSoundAvailability2",
    "cellAudioOutGetDeviceInfo",
    "cellAudioOutGetNumberOfDevice",
    "cellAudioOutGetConfiguration",
    "cellAudioOutSetCopyControl",
    "cellAudioOutRegisterCallback",
    "cellAudioOutUnregisterCallback",
];

// ---------------------------------------------------------------------------
// Errors byte-exato cellAudioOut.h:
// ---------------------------------------------------------------------------
pub const CELL_AUDIO_OUT_ERROR_NOT_IMPLEMENTED: CellError = CellError(0x8002_B240);
pub const CELL_AUDIO_OUT_ERROR_ILLEGAL_CONFIGURATION: CellError = CellError(0x8002_B241);
pub const CELL_AUDIO_OUT_ERROR_ILLEGAL_PARAMETER: CellError = CellError(0x8002_B242);
pub const CELL_AUDIO_OUT_ERROR_PARAMETER_OUT_OF_RANGE: CellError = CellError(0x8002_B243);
pub const CELL_AUDIO_OUT_ERROR_DEVICE_NOT_FOUND: CellError = CellError(0x8002_B244);
pub const CELL_AUDIO_OUT_ERROR_UNSUPPORTED_AUDIO_OUT: CellError = CellError(0x8002_B245);
pub const CELL_AUDIO_OUT_ERROR_UNSUPPORTED_SOUND_MODE: CellError = CellError(0x8002_B246);
pub const CELL_AUDIO_OUT_ERROR_CONDITION_BUSY: CellError = CellError(0x8002_B247);

// ---------------------------------------------------------------------------
// Constants (header byte-exato).
// ---------------------------------------------------------------------------
pub const CELL_AUDIO_OUT_PRIMARY: u32 = 0;
pub const CELL_AUDIO_OUT_SECONDARY: u32 = 1;

pub const CELL_AUDIO_OUT_DOWNMIXER_NONE: u32 = 0;
pub const CELL_AUDIO_OUT_DOWNMIXER_TYPE_A: u32 = 1;
pub const CELL_AUDIO_OUT_DOWNMIXER_TYPE_B: u32 = 2;

pub const CELL_AUDIO_OUT_CODING_TYPE_LPCM: u8 = 0;
pub const CELL_AUDIO_OUT_CODING_TYPE_AC3: u8 = 1;
pub const CELL_AUDIO_OUT_CODING_TYPE_DTS: u8 = 6;
pub const CELL_AUDIO_OUT_CODING_TYPE_BITSTREAM: u8 = 0xFF;

pub const CELL_AUDIO_OUT_CHNUM_2: u8 = 2;
pub const CELL_AUDIO_OUT_CHNUM_4: u8 = 4;
pub const CELL_AUDIO_OUT_CHNUM_6: u8 = 6;
pub const CELL_AUDIO_OUT_CHNUM_8: u8 = 8;

pub const CELL_AUDIO_OUT_FS_32KHZ: u8 = 0x01;
pub const CELL_AUDIO_OUT_FS_44KHZ: u8 = 0x02;
pub const CELL_AUDIO_OUT_FS_48KHZ: u8 = 0x04;
pub const CELL_AUDIO_OUT_FS_88KHZ: u8 = 0x08;
pub const CELL_AUDIO_OUT_FS_96KHZ: u8 = 0x10;
pub const CELL_AUDIO_OUT_FS_176KHZ: u8 = 0x20;
pub const CELL_AUDIO_OUT_FS_192KHZ: u8 = 0x40;

pub const CELL_AUDIO_OUT_SPEAKER_LAYOUT_DEFAULT: u32 = 0x0000_0000;
pub const CELL_AUDIO_OUT_SPEAKER_LAYOUT_2CH: u32 = 0x0000_0001;
pub const CELL_AUDIO_OUT_SPEAKER_LAYOUT_6CH_LREClr: u32 = 0x0001_0000;
pub const CELL_AUDIO_OUT_SPEAKER_LAYOUT_8CH_LREClrxy: u32 = 0x4000_0000;

pub const CELL_AUDIO_OUT_COPY_CONTROL_COPY_FREE: u32 = 0;
pub const CELL_AUDIO_OUT_COPY_CONTROL_COPY_ONCE: u32 = 1;
pub const CELL_AUDIO_OUT_COPY_CONTROL_COPY_NEVER: u32 = 2;

pub const CELL_AUDIO_OUT_OUTPUT_STATE_ENABLED: u32 = 0;
pub const CELL_AUDIO_OUT_OUTPUT_STATE_DISABLED: u32 = 1;

pub const CELL_AUDIO_OUT_EVENT_OUTPUT_DISABLED: u32 = 1;
pub const CELL_AUDIO_OUT_EVENT_OUTPUT_ENABLED: u32 = 3;

pub const NUM_AUDIO_OUTS: usize = 2;
pub const MAX_CALLBACKS: usize = 8;

// ---------------------------------------------------------------------------
// Wire structs.
// ---------------------------------------------------------------------------

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
#[repr(C)]
pub struct CellAudioOutSoundMode {
    pub ty: u8,
    pub channel: u8,
    pub fs: u8,
    pub _reserved: u8,
    pub layout: u32,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
#[repr(C)]
pub struct CellAudioOutState {
    pub state: u32,
    pub encoder: u32,
    pub downmixer: u32,
    pub sound_mode: CellAudioOutSoundMode,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
#[repr(C)]
pub struct CellAudioOutConfiguration {
    pub channel: u8,
    pub encoder: u8,
    pub _reserved: [u8; 2],
    pub downmixer: u32,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
#[repr(C)]
pub struct CellAudioOutOption {
    pub _reserved: [u8; 16],
}

// ---------------------------------------------------------------------------
// Pluggable sources for PSF/g_cfg.
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SoundFormatFlags {
    pub lpcm_2: bool,
    pub lpcm_5_1: bool,
    pub lpcm_7_1: bool,
    pub ac3: bool,
    pub dts: bool,
}

impl Default for SoundFormatFlags {
    fn default() -> Self {
        Self {
            lpcm_2: true, // default per cpp:44
            lpcm_5_1: false,
            lpcm_7_1: false,
            ac3: false,
            dts: false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AudioFormat {
    Stereo,
    Surround51,
    Surround71,
    Automatic,
    Manual(u32),
}

impl Default for AudioFormat {
    fn default() -> Self {
        AudioFormat::Stereo
    }
}

pub mod audio_format_flag {
    pub const LPCM_7_1_48KHZ: u32 = 0x01;
    pub const LPCM_5_1_48KHZ: u32 = 0x02;
    pub const AC3: u32 = 0x04;
    pub const DTS: u32 = 0x08;
}

// ---------------------------------------------------------------------------
// AudioOut state struct.
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AudioOut {
    pub state: u32,
    pub channels: u8,
    pub encoder: u8,
    pub downmixer: u32,
    pub copy_control: u32,
    pub sound_modes: Vec<CellAudioOutSoundMode>,
    pub sound_mode: CellAudioOutSoundMode,
}

impl Default for AudioOut {
    fn default() -> Self {
        Self {
            state: CELL_AUDIO_OUT_OUTPUT_STATE_ENABLED,
            channels: CELL_AUDIO_OUT_CHNUM_2,
            encoder: CELL_AUDIO_OUT_CODING_TYPE_LPCM,
            downmixer: CELL_AUDIO_OUT_DOWNMIXER_NONE,
            copy_control: CELL_AUDIO_OUT_COPY_CONTROL_COPY_FREE,
            sound_modes: Vec::new(),
            sound_mode: CellAudioOutSoundMode::default(),
        }
    }
}

// ---------------------------------------------------------------------------
// Manager.
// ---------------------------------------------------------------------------

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct AudioOutCallback {
    pub function: u32,
    pub user_data: u32,
}

#[derive(Debug)]
pub struct CellAudioOut {
    pub outs: [AudioOut; NUM_AUDIO_OUTS],
    pub callbacks: [Option<AudioOutCallback>; MAX_CALLBACKS],

    pub get_state_calls: u64,
    pub configure_calls: u64,
    pub get_sound_avail_calls: u64,
    pub get_sound_avail2_calls: u64,
    pub get_device_info_calls: u64,
    pub get_num_device_calls: u64,
    pub get_config_calls: u64,
    pub set_copy_control_calls: u64,
    pub register_cb_calls: u64,
    pub unregister_cb_calls: u64,
}

impl Default for CellAudioOut {
    fn default() -> Self {
        Self {
            outs: [AudioOut::default(), AudioOut::default()],
            callbacks: [None; MAX_CALLBACKS],
            get_state_calls: 0,
            configure_calls: 0,
            get_sound_avail_calls: 0,
            get_sound_avail2_calls: 0,
            get_device_info_calls: 0,
            get_num_device_calls: 0,
            get_config_calls: 0,
            set_copy_control_calls: 0,
            register_cb_calls: 0,
            unregister_cb_calls: 0,
        }
    }
}

impl CellAudioOut {
    /// Constructs with default sound-mode table seeded by PSF + audio_format
    /// (mirrors cpp:35-180 `audio_out_configuration::audio_out_configuration`).
    pub fn new_with_init(format: AudioFormat, flags: SoundFormatFlags) -> Self {
        let mut m = Self::default();
        m.seed_sound_modes(format, flags);
        m
    }

    pub fn new() -> Self {
        // Default = stereo + lpcm_2 only.
        Self::new_with_init(AudioFormat::Stereo, SoundFormatFlags::default())
    }

    fn add_sound_mode(
        &mut self,
        index: u32,
        ty: u8,
        channel: u8,
        fs: u8,
        layout: u32,
        supported: bool,
        initial_selected: &mut [bool; NUM_AUDIO_OUTS],
    ) {
        let i = index as usize;
        let mode = CellAudioOutSoundMode {
            ty,
            channel,
            fs,
            _reserved: 0,
            layout,
        };
        self.outs[i].sound_modes.push(mode);
        if !initial_selected[i] && supported {
            self.outs[i].channels = channel;
            self.outs[i].encoder = ty;
            self.outs[i].sound_mode = mode;
            initial_selected[i] = true;
        }
    }

    fn seed_sound_modes(&mut self, format: AudioFormat, flags: SoundFormatFlags) {
        let mut sel = [false, false];

        match format {
            AudioFormat::Stereo => {} // default LPCM 2 is added below
            AudioFormat::Surround71 => {
                self.add_sound_mode(
                    CELL_AUDIO_OUT_PRIMARY,
                    CELL_AUDIO_OUT_CODING_TYPE_LPCM,
                    CELL_AUDIO_OUT_CHNUM_8,
                    CELL_AUDIO_OUT_FS_48KHZ,
                    CELL_AUDIO_OUT_SPEAKER_LAYOUT_8CH_LREClrxy,
                    flags.lpcm_7_1,
                    &mut sel,
                );
                // fall-through 5.1 modes
                self.seed_5_1(flags, &mut sel);
            }
            AudioFormat::Surround51 => {
                self.seed_5_1(flags, &mut sel);
            }
            AudioFormat::Automatic => {
                if flags.lpcm_7_1 {
                    self.add_sound_mode(
                        CELL_AUDIO_OUT_PRIMARY,
                        CELL_AUDIO_OUT_CODING_TYPE_LPCM,
                        CELL_AUDIO_OUT_CHNUM_8,
                        CELL_AUDIO_OUT_FS_48KHZ,
                        CELL_AUDIO_OUT_SPEAKER_LAYOUT_8CH_LREClrxy,
                        true,
                        &mut sel,
                    );
                }
                if flags.lpcm_5_1 {
                    self.add_sound_mode(
                        CELL_AUDIO_OUT_PRIMARY,
                        CELL_AUDIO_OUT_CODING_TYPE_LPCM,
                        CELL_AUDIO_OUT_CHNUM_6,
                        CELL_AUDIO_OUT_FS_48KHZ,
                        CELL_AUDIO_OUT_SPEAKER_LAYOUT_6CH_LREClr,
                        true,
                        &mut sel,
                    );
                }
                if flags.ac3 {
                    self.add_sound_mode(
                        CELL_AUDIO_OUT_PRIMARY,
                        CELL_AUDIO_OUT_CODING_TYPE_AC3,
                        CELL_AUDIO_OUT_CHNUM_6,
                        CELL_AUDIO_OUT_FS_48KHZ,
                        CELL_AUDIO_OUT_SPEAKER_LAYOUT_6CH_LREClr,
                        true,
                        &mut sel,
                    );
                }
                if flags.dts {
                    self.add_sound_mode(
                        CELL_AUDIO_OUT_PRIMARY,
                        CELL_AUDIO_OUT_CODING_TYPE_DTS,
                        CELL_AUDIO_OUT_CHNUM_6,
                        CELL_AUDIO_OUT_FS_48KHZ,
                        CELL_AUDIO_OUT_SPEAKER_LAYOUT_6CH_LREClr,
                        true,
                        &mut sel,
                    );
                }
            }
            AudioFormat::Manual(selected_formats) => {
                if selected_formats & audio_format_flag::LPCM_7_1_48KHZ != 0 {
                    self.add_sound_mode(
                        CELL_AUDIO_OUT_PRIMARY,
                        CELL_AUDIO_OUT_CODING_TYPE_LPCM,
                        CELL_AUDIO_OUT_CHNUM_8,
                        CELL_AUDIO_OUT_FS_48KHZ,
                        CELL_AUDIO_OUT_SPEAKER_LAYOUT_8CH_LREClrxy,
                        flags.lpcm_7_1,
                        &mut sel,
                    );
                }
                if selected_formats & audio_format_flag::LPCM_5_1_48KHZ != 0 {
                    self.add_sound_mode(
                        CELL_AUDIO_OUT_PRIMARY,
                        CELL_AUDIO_OUT_CODING_TYPE_LPCM,
                        CELL_AUDIO_OUT_CHNUM_6,
                        CELL_AUDIO_OUT_FS_48KHZ,
                        CELL_AUDIO_OUT_SPEAKER_LAYOUT_6CH_LREClr,
                        flags.lpcm_5_1,
                        &mut sel,
                    );
                }
                if selected_formats & audio_format_flag::AC3 != 0 {
                    self.add_sound_mode(
                        CELL_AUDIO_OUT_PRIMARY,
                        CELL_AUDIO_OUT_CODING_TYPE_AC3,
                        CELL_AUDIO_OUT_CHNUM_6,
                        CELL_AUDIO_OUT_FS_48KHZ,
                        CELL_AUDIO_OUT_SPEAKER_LAYOUT_6CH_LREClr,
                        flags.ac3,
                        &mut sel,
                    );
                }
                if selected_formats & audio_format_flag::DTS != 0 {
                    self.add_sound_mode(
                        CELL_AUDIO_OUT_PRIMARY,
                        CELL_AUDIO_OUT_CODING_TYPE_DTS,
                        CELL_AUDIO_OUT_CHNUM_6,
                        CELL_AUDIO_OUT_FS_48KHZ,
                        CELL_AUDIO_OUT_SPEAKER_LAYOUT_6CH_LREClr,
                        flags.dts,
                        &mut sel,
                    );
                }
            }
        }

        // Always add LPCM 2 to PRIMARY (cpp:160).
        self.add_sound_mode(
            CELL_AUDIO_OUT_PRIMARY,
            CELL_AUDIO_OUT_CODING_TYPE_LPCM,
            CELL_AUDIO_OUT_CHNUM_2,
            CELL_AUDIO_OUT_FS_48KHZ,
            CELL_AUDIO_OUT_SPEAKER_LAYOUT_2CH,
            true,
            &mut sel,
        );
        // SECONDARY only LPCM 2 (cpp:163).
        self.add_sound_mode(
            CELL_AUDIO_OUT_SECONDARY,
            CELL_AUDIO_OUT_CODING_TYPE_LPCM,
            CELL_AUDIO_OUT_CHNUM_2,
            CELL_AUDIO_OUT_FS_48KHZ,
            CELL_AUDIO_OUT_SPEAKER_LAYOUT_2CH,
            true,
            &mut sel,
        );
    }

    fn seed_5_1(&mut self, flags: SoundFormatFlags, sel: &mut [bool; NUM_AUDIO_OUTS]) {
        self.add_sound_mode(
            CELL_AUDIO_OUT_PRIMARY,
            CELL_AUDIO_OUT_CODING_TYPE_LPCM,
            CELL_AUDIO_OUT_CHNUM_6,
            CELL_AUDIO_OUT_FS_48KHZ,
            CELL_AUDIO_OUT_SPEAKER_LAYOUT_6CH_LREClr,
            flags.lpcm_5_1,
            sel,
        );
        self.add_sound_mode(
            CELL_AUDIO_OUT_PRIMARY,
            CELL_AUDIO_OUT_CODING_TYPE_AC3,
            CELL_AUDIO_OUT_CHNUM_6,
            CELL_AUDIO_OUT_FS_48KHZ,
            CELL_AUDIO_OUT_SPEAKER_LAYOUT_6CH_LREClr,
            flags.ac3,
            sel,
        );
        self.add_sound_mode(
            CELL_AUDIO_OUT_PRIMARY,
            CELL_AUDIO_OUT_CODING_TYPE_DTS,
            CELL_AUDIO_OUT_CHNUM_6,
            CELL_AUDIO_OUT_FS_48KHZ,
            CELL_AUDIO_OUT_SPEAKER_LAYOUT_6CH_LREClr,
            flags.dts,
            sel,
        );
    }

    /// `cellAudioOutGetNumberOfDevice(audioOut)` — PRIMARY=1, SECONDARY=0.
    pub fn get_number_of_device(&mut self, audio_out: u32) -> Result<i32, CellError> {
        self.get_num_device_calls = self.get_num_device_calls.saturating_add(1);
        match audio_out {
            CELL_AUDIO_OUT_PRIMARY => Ok(1),
            CELL_AUDIO_OUT_SECONDARY => Ok(0),
            _ => Err(CELL_AUDIO_OUT_ERROR_ILLEGAL_PARAMETER),
        }
    }

    /// `cellAudioOutGetState(audioOut, deviceIndex, state)`.
    pub fn get_state(
        &mut self,
        audio_out: u32,
        _device_index: u32,
        out: Option<&mut CellAudioOutState>,
    ) -> Result<(), CellError> {
        self.get_state_calls = self.get_state_calls.saturating_add(1);
        if audio_out >= NUM_AUDIO_OUTS as u32 {
            return Err(CELL_AUDIO_OUT_ERROR_ILLEGAL_PARAMETER);
        }
        let slot = out.ok_or(CELL_AUDIO_OUT_ERROR_ILLEGAL_PARAMETER)?;
        let o = &self.outs[audio_out as usize];
        *slot = CellAudioOutState {
            state: o.state,
            encoder: o.encoder as u32,
            downmixer: o.downmixer,
            sound_mode: o.sound_mode,
        };
        Ok(())
    }

    /// `cellAudioOutGetConfiguration(audioOut, config, option)`.
    pub fn get_configuration(
        &mut self,
        audio_out: u32,
        config_out: Option<&mut CellAudioOutConfiguration>,
        _option: Option<&mut CellAudioOutOption>,
    ) -> Result<(), CellError> {
        self.get_config_calls = self.get_config_calls.saturating_add(1);
        if audio_out >= NUM_AUDIO_OUTS as u32 {
            return Err(CELL_AUDIO_OUT_ERROR_ILLEGAL_PARAMETER);
        }
        let slot = config_out.ok_or(CELL_AUDIO_OUT_ERROR_ILLEGAL_PARAMETER)?;
        let o = &self.outs[audio_out as usize];
        *slot = CellAudioOutConfiguration {
            channel: o.channels,
            encoder: o.encoder,
            _reserved: [0; 2],
            downmixer: o.downmixer,
        };
        Ok(())
    }

    /// `cellAudioOutConfigure(audioOut, config, option, waitForEvent)`.
    pub fn configure(
        &mut self,
        audio_out: u32,
        config: Option<&CellAudioOutConfiguration>,
        _option: Option<&CellAudioOutOption>,
        _wait_for_event: u32,
    ) -> Result<(), CellError> {
        self.configure_calls = self.configure_calls.saturating_add(1);
        if audio_out >= NUM_AUDIO_OUTS as u32 {
            return Err(CELL_AUDIO_OUT_ERROR_ILLEGAL_PARAMETER);
        }
        let cfg = config.ok_or(CELL_AUDIO_OUT_ERROR_ILLEGAL_PARAMETER)?;
        let o = &mut self.outs[audio_out as usize];
        o.channels = cfg.channel;
        o.encoder = cfg.encoder;
        o.downmixer = cfg.downmixer;
        Ok(())
    }

    /// `cellAudioOutGetSoundAvailability(audioOut, type, fs, option)`.
    /// Returns highest matching channel count or 0 if not supported.
    pub fn get_sound_availability(
        &mut self,
        audio_out: u32,
        ty: u32,
        fs: u32,
        _option: u32,
    ) -> Result<u32, CellError> {
        self.get_sound_avail_calls = self.get_sound_avail_calls.saturating_add(1);
        if audio_out >= NUM_AUDIO_OUTS as u32 {
            return Err(CELL_AUDIO_OUT_ERROR_ILLEGAL_PARAMETER);
        }
        let mut best_ch = 0u32;
        for m in &self.outs[audio_out as usize].sound_modes {
            if m.ty as u32 == ty && (m.fs as u32) & fs != 0 && (m.channel as u32) > best_ch {
                best_ch = m.channel as u32;
            }
        }
        Ok(best_ch)
    }

    /// `cellAudioOutGetSoundAvailability2(audioOut, type, fs, ch, option)`.
    pub fn get_sound_availability2(
        &mut self,
        audio_out: u32,
        ty: u32,
        fs: u32,
        ch: u32,
        _option: u32,
    ) -> Result<u32, CellError> {
        self.get_sound_avail2_calls = self.get_sound_avail2_calls.saturating_add(1);
        if audio_out >= NUM_AUDIO_OUTS as u32 {
            return Err(CELL_AUDIO_OUT_ERROR_ILLEGAL_PARAMETER);
        }
        for m in &self.outs[audio_out as usize].sound_modes {
            if m.ty as u32 == ty && (m.fs as u32) & fs != 0 && m.channel as u32 == ch {
                return Ok(ch);
            }
        }
        Ok(0)
    }

    /// `cellAudioOutSetCopyControl(audioOut, control)`.
    pub fn set_copy_control(&mut self, audio_out: u32, control: u32) -> Result<(), CellError> {
        self.set_copy_control_calls = self.set_copy_control_calls.saturating_add(1);
        if audio_out >= NUM_AUDIO_OUTS as u32 {
            return Err(CELL_AUDIO_OUT_ERROR_ILLEGAL_PARAMETER);
        }
        match control {
            CELL_AUDIO_OUT_COPY_CONTROL_COPY_FREE
            | CELL_AUDIO_OUT_COPY_CONTROL_COPY_ONCE
            | CELL_AUDIO_OUT_COPY_CONTROL_COPY_NEVER => {}
            _ => return Err(CELL_AUDIO_OUT_ERROR_ILLEGAL_PARAMETER),
        }
        self.outs[audio_out as usize].copy_control = control;
        Ok(())
    }

    /// `cellAudioOutGetDeviceInfo(audioOut, deviceIndex, info)` —
    /// only PRIMARY index 0 valid; secondary always returns DEVICE_NOT_FOUND.
    pub fn get_device_info(
        &mut self,
        audio_out: u32,
        device_index: u32,
        info_present: bool,
    ) -> Result<(), CellError> {
        self.get_device_info_calls = self.get_device_info_calls.saturating_add(1);
        if !info_present {
            return Err(CELL_AUDIO_OUT_ERROR_ILLEGAL_PARAMETER);
        }
        if audio_out != CELL_AUDIO_OUT_PRIMARY || device_index != 0 {
            return Err(CELL_AUDIO_OUT_ERROR_DEVICE_NOT_FOUND);
        }
        Ok(())
    }

    /// `cellAudioOutRegisterCallback(slot, function, userData)`.
    pub fn register_callback(
        &mut self,
        slot: u32,
        function: u32,
        user_data: u32,
    ) -> Result<(), CellError> {
        self.register_cb_calls = self.register_cb_calls.saturating_add(1);
        if (slot as usize) >= MAX_CALLBACKS {
            return Err(CELL_AUDIO_OUT_ERROR_PARAMETER_OUT_OF_RANGE);
        }
        if function == 0 {
            return Err(CELL_AUDIO_OUT_ERROR_ILLEGAL_PARAMETER);
        }
        self.callbacks[slot as usize] = Some(AudioOutCallback {
            function,
            user_data,
        });
        Ok(())
    }

    /// `cellAudioOutUnregisterCallback(slot)`.
    pub fn unregister_callback(&mut self, slot: u32) -> Result<(), CellError> {
        self.unregister_cb_calls = self.unregister_cb_calls.saturating_add(1);
        if (slot as usize) >= MAX_CALLBACKS {
            return Err(CELL_AUDIO_OUT_ERROR_PARAMETER_OUT_OF_RANGE);
        }
        self.callbacks[slot as usize] = None;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn module_and_entries() {
        assert_eq!(HOST_MODULE_NAME, "cellSysutil");
        assert_eq!(SUBMODULE_NAME, "cellAudioOut");
        assert_eq!(REGISTERED_ENTRY_POINTS.len(), 10);
    }

    #[test]
    fn errors_byte_exact() {
        assert_eq!(CELL_AUDIO_OUT_ERROR_NOT_IMPLEMENTED.0, 0x8002_B240);
        assert_eq!(CELL_AUDIO_OUT_ERROR_CONDITION_BUSY.0, 0x8002_B247);
    }

    #[test]
    fn default_init_seeds_lpcm_2_only() {
        let m = CellAudioOut::new();
        // PRIMARY has LPCM 2 mode, SECONDARY also has LPCM 2.
        assert!(!m.outs[0].sound_modes.is_empty());
        assert!(!m.outs[1].sound_modes.is_empty());
        // Initial selected channels = 2.
        assert_eq!(m.outs[0].channels, 2);
        assert_eq!(m.outs[1].channels, 2);
    }

    #[test]
    fn surround_71_seeds_8ch_then_falls_through_to_5_1() {
        let flags = SoundFormatFlags {
            lpcm_2: true,
            lpcm_5_1: true,
            lpcm_7_1: true,
            ac3: true,
            dts: true,
        };
        let m = CellAudioOut::new_with_init(AudioFormat::Surround71, flags);
        // Should contain 8ch LPCM + 6ch LPCM/AC3/DTS + 2ch LPCM = 5 modes minimum.
        assert!(m.outs[0].sound_modes.len() >= 5);
        // Initial selection picks first supported = 7.1 LPCM (8ch).
        assert_eq!(m.outs[0].channels, 8);
        assert_eq!(m.outs[0].encoder, CELL_AUDIO_OUT_CODING_TYPE_LPCM);
    }

    #[test]
    fn automatic_only_includes_supported() {
        let flags = SoundFormatFlags {
            lpcm_2: true,
            lpcm_5_1: false,
            lpcm_7_1: true, // only 7.1 + 2
            ac3: false,
            dts: false,
        };
        let m = CellAudioOut::new_with_init(AudioFormat::Automatic, flags);
        // Should have 7.1 LPCM + 2.0 LPCM = 2 modes minimum on PRIMARY.
        assert_eq!(m.outs[0].channels, 8);
    }

    #[test]
    fn manual_format_flags_filter() {
        let flags = SoundFormatFlags {
            lpcm_2: true,
            lpcm_5_1: true,
            lpcm_7_1: false,
            ac3: false,
            dts: false,
        };
        let m = CellAudioOut::new_with_init(
            AudioFormat::Manual(audio_format_flag::LPCM_5_1_48KHZ),
            flags,
        );
        // 5.1 LPCM gets selected as initial.
        assert_eq!(m.outs[0].channels, 6);
    }

    #[test]
    fn get_num_device_returns_1_primary_0_secondary() {
        let mut m = CellAudioOut::new();
        assert_eq!(m.get_number_of_device(CELL_AUDIO_OUT_PRIMARY).unwrap(), 1);
        assert_eq!(m.get_number_of_device(CELL_AUDIO_OUT_SECONDARY).unwrap(), 0);
        assert_eq!(
            m.get_number_of_device(99),
            Err(CELL_AUDIO_OUT_ERROR_ILLEGAL_PARAMETER)
        );
    }

    #[test]
    fn get_state_writes_initial_config() {
        let mut m = CellAudioOut::new();
        let mut s = CellAudioOutState::default();
        m.get_state(CELL_AUDIO_OUT_PRIMARY, 0, Some(&mut s)).unwrap();
        assert_eq!(s.state, CELL_AUDIO_OUT_OUTPUT_STATE_ENABLED);
        assert_eq!(s.encoder, CELL_AUDIO_OUT_CODING_TYPE_LPCM as u32);
        assert_eq!(s.downmixer, CELL_AUDIO_OUT_DOWNMIXER_NONE);
    }

    #[test]
    fn get_state_invalid_audio_out() {
        let mut m = CellAudioOut::new();
        let mut s = CellAudioOutState::default();
        assert_eq!(
            m.get_state(99, 0, Some(&mut s)),
            Err(CELL_AUDIO_OUT_ERROR_ILLEGAL_PARAMETER)
        );
    }

    #[test]
    fn get_state_null_out() {
        let mut m = CellAudioOut::new();
        assert_eq!(
            m.get_state(0, 0, None),
            Err(CELL_AUDIO_OUT_ERROR_ILLEGAL_PARAMETER)
        );
    }

    #[test]
    fn configure_writes_and_get_reads() {
        let mut m = CellAudioOut::new();
        let cfg = CellAudioOutConfiguration {
            channel: CELL_AUDIO_OUT_CHNUM_6,
            encoder: CELL_AUDIO_OUT_CODING_TYPE_AC3,
            _reserved: [0; 2],
            downmixer: CELL_AUDIO_OUT_DOWNMIXER_TYPE_A,
        };
        m.configure(CELL_AUDIO_OUT_PRIMARY, Some(&cfg), None, 0).unwrap();
        let mut got = CellAudioOutConfiguration::default();
        m.get_configuration(CELL_AUDIO_OUT_PRIMARY, Some(&mut got), None).unwrap();
        assert_eq!(got.channel, CELL_AUDIO_OUT_CHNUM_6);
        assert_eq!(got.encoder, CELL_AUDIO_OUT_CODING_TYPE_AC3);
        assert_eq!(got.downmixer, CELL_AUDIO_OUT_DOWNMIXER_TYPE_A);
    }

    #[test]
    fn sound_availability_finds_match() {
        let flags = SoundFormatFlags {
            lpcm_2: true,
            lpcm_5_1: true,
            lpcm_7_1: false,
            ac3: false,
            dts: false,
        };
        let mut m = CellAudioOut::new_with_init(AudioFormat::Surround51, flags);
        // LPCM at 48kHz, looking for 8ch — only 6ch LPCM exists.
        assert_eq!(
            m.get_sound_availability(
                CELL_AUDIO_OUT_PRIMARY,
                CELL_AUDIO_OUT_CODING_TYPE_LPCM as u32,
                CELL_AUDIO_OUT_FS_48KHZ as u32,
                0
            )
            .unwrap(),
            6
        );
    }

    #[test]
    fn sound_availability2_exact_match() {
        let flags = SoundFormatFlags {
            lpcm_2: true,
            lpcm_5_1: true,
            ..Default::default()
        };
        let mut m = CellAudioOut::new_with_init(AudioFormat::Surround51, flags);
        assert_eq!(
            m.get_sound_availability2(
                CELL_AUDIO_OUT_PRIMARY,
                CELL_AUDIO_OUT_CODING_TYPE_LPCM as u32,
                CELL_AUDIO_OUT_FS_48KHZ as u32,
                6,
                0
            )
            .unwrap(),
            6
        );
        // Asking 8ch returns 0 (not available).
        assert_eq!(
            m.get_sound_availability2(
                CELL_AUDIO_OUT_PRIMARY,
                CELL_AUDIO_OUT_CODING_TYPE_LPCM as u32,
                CELL_AUDIO_OUT_FS_48KHZ as u32,
                8,
                0
            )
            .unwrap(),
            0
        );
    }

    #[test]
    fn set_copy_control_validates_value() {
        let mut m = CellAudioOut::new();
        m.set_copy_control(0, CELL_AUDIO_OUT_COPY_CONTROL_COPY_FREE).unwrap();
        m.set_copy_control(0, CELL_AUDIO_OUT_COPY_CONTROL_COPY_ONCE).unwrap();
        m.set_copy_control(0, CELL_AUDIO_OUT_COPY_CONTROL_COPY_NEVER).unwrap();
        assert_eq!(m.outs[0].copy_control, CELL_AUDIO_OUT_COPY_CONTROL_COPY_NEVER);
        assert_eq!(
            m.set_copy_control(0, 99),
            Err(CELL_AUDIO_OUT_ERROR_ILLEGAL_PARAMETER)
        );
    }

    #[test]
    fn get_device_info_secondary_not_found() {
        let mut m = CellAudioOut::new();
        assert_eq!(
            m.get_device_info(CELL_AUDIO_OUT_SECONDARY, 0, true),
            Err(CELL_AUDIO_OUT_ERROR_DEVICE_NOT_FOUND)
        );
        assert_eq!(
            m.get_device_info(CELL_AUDIO_OUT_PRIMARY, 1, true),
            Err(CELL_AUDIO_OUT_ERROR_DEVICE_NOT_FOUND)
        );
        m.get_device_info(CELL_AUDIO_OUT_PRIMARY, 0, true).unwrap();
    }

    #[test]
    fn register_callback_within_slot_range() {
        let mut m = CellAudioOut::new();
        m.register_callback(0, 0xCAFE, 0xBEEF).unwrap();
        assert!(m.callbacks[0].is_some());
        assert_eq!(m.callbacks[0].unwrap().function, 0xCAFE);
        assert_eq!(
            m.register_callback(99, 0xCAFE, 0),
            Err(CELL_AUDIO_OUT_ERROR_PARAMETER_OUT_OF_RANGE)
        );
        assert_eq!(
            m.register_callback(1, 0, 0),
            Err(CELL_AUDIO_OUT_ERROR_ILLEGAL_PARAMETER)
        );
    }

    #[test]
    fn unregister_callback_clears_slot() {
        let mut m = CellAudioOut::new();
        m.register_callback(2, 0xAAAA, 0).unwrap();
        m.unregister_callback(2).unwrap();
        assert!(m.callbacks[2].is_none());
        assert_eq!(
            m.unregister_callback(99),
            Err(CELL_AUDIO_OUT_ERROR_PARAMETER_OUT_OF_RANGE)
        );
    }

    #[test]
    fn full_audioout_lifecycle_smoke() {
        let flags = SoundFormatFlags {
            lpcm_2: true,
            lpcm_5_1: true,
            lpcm_7_1: true,
            ac3: true,
            dts: true,
        };
        let mut m = CellAudioOut::new_with_init(AudioFormat::Automatic, flags);
        // Game queries device count.
        assert_eq!(m.get_number_of_device(CELL_AUDIO_OUT_PRIMARY).unwrap(), 1);
        // Game asks if 7.1 LPCM @ 48kHz available.
        assert_eq!(
            m.get_sound_availability2(
                CELL_AUDIO_OUT_PRIMARY,
                CELL_AUDIO_OUT_CODING_TYPE_LPCM as u32,
                CELL_AUDIO_OUT_FS_48KHZ as u32,
                8,
                0
            )
            .unwrap(),
            8
        );
        // Game configures for 7.1 LPCM.
        let cfg = CellAudioOutConfiguration {
            channel: 8,
            encoder: CELL_AUDIO_OUT_CODING_TYPE_LPCM,
            _reserved: [0; 2],
            downmixer: CELL_AUDIO_OUT_DOWNMIXER_NONE,
        };
        m.configure(CELL_AUDIO_OUT_PRIMARY, Some(&cfg), None, 0).unwrap();
        // Game registers a callback.
        m.register_callback(0, 0x4000_1000, 0xDEAD_BEEF).unwrap();
        // Game queries state.
        let mut state = CellAudioOutState::default();
        m.get_state(CELL_AUDIO_OUT_PRIMARY, 0, Some(&mut state)).unwrap();
        assert_eq!(state.state, CELL_AUDIO_OUT_OUTPUT_STATE_ENABLED);
        // Set copy control.
        m.set_copy_control(CELL_AUDIO_OUT_PRIMARY, CELL_AUDIO_OUT_COPY_CONTROL_COPY_NEVER)
            .unwrap();
        // Tear down.
        m.unregister_callback(0).unwrap();
    }
}
