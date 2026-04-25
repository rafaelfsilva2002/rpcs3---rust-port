//! `rpcs3-io-rb3drums-config` — Rust port of `rpcs3/Emu/Io/rb3drums_config.cpp` + `.h`.
//!
//! Defaults and bounds for RPCS3's MIDI → Rock Band 3 drum-kit mapper.
//! The kit talks to the host as a MIDI device; this config defines how
//! hits are pulsed, which MIDI CC drives the hi-hat pedal threshold,
//! and what "combos" trigger menu actions.
//!
//! Frozen from the cpp header:
//!
//! - Config file name: `rb3drums.yml`.
//! - `pulse_ms` `[1, 100]` default 30.
//! - `minimum_velocity` `[1, 127]` default 10.
//! - `combo_window_ms` `[1, 5000]` default 2000.
//! - `stagger_cymbals` bool default `true`.
//! - Default combo strings (Start / Select / ToggleHoldKick).
//! - MIDI CC defaults: status `0xB0`, number `4`, threshold `64`.

pub const CONFIG_FILE_NAME: &str = "rb3drums.yml";

pub const PULSE_MS_MIN: u32 = 1;
pub const PULSE_MS_MAX: u32 = 100;
pub const PULSE_MS_DEFAULT: u32 = 30;

pub const MIN_VELOCITY_MIN: u32 = 1;
pub const MIN_VELOCITY_MAX: u32 = 127;
pub const MIN_VELOCITY_DEFAULT: u32 = 10;

pub const COMBO_WINDOW_MS_MIN: u32 = 1;
pub const COMBO_WINDOW_MS_MAX: u32 = 5000;
pub const COMBO_WINDOW_MS_DEFAULT: u32 = 2000;

pub const STAGGER_CYMBALS_DEFAULT: bool = true;

pub const DEFAULT_MIDI_OVERRIDES: &str = "";
pub const DEFAULT_COMBO_START: &str = "HihatPedal,HihatPedal,HihatPedal,Snare";
pub const DEFAULT_COMBO_SELECT: &str = "HihatPedal,HihatPedal,HihatPedal,SnareRim";
pub const DEFAULT_COMBO_TOGGLE_HOLD_KICK: &str = "HihatPedal,HihatPedal,HihatPedal,Kick";

/// `cfg::uint<0, 255>` MIDI CC status byte. Default 0xB0 = "Control Change"
/// on MIDI channel 1.
pub const MIDI_CC_STATUS_DEFAULT: u32 = 0xB0;
pub const MIDI_CC_NUMBER_DEFAULT: u32 = 4;
pub const MIDI_CC_THRESHOLD_DEFAULT: u32 = 64;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rb3DrumsConfig {
    pub pulse_ms: u32,
    pub minimum_velocity: u32,
    pub combo_window_ms: u32,
    pub stagger_cymbals: bool,
    pub midi_overrides: String,
    pub combo_start: String,
    pub combo_select: String,
    pub combo_toggle_hold_kick: String,
    pub midi_cc_status: u32,
    pub midi_cc_number: u32,
    pub midi_cc_threshold: u32,
    pub midi_cc_invert_threshold: bool,
}

impl Default for Rb3DrumsConfig {
    fn default() -> Self {
        Self {
            pulse_ms: PULSE_MS_DEFAULT,
            minimum_velocity: MIN_VELOCITY_DEFAULT,
            combo_window_ms: COMBO_WINDOW_MS_DEFAULT,
            stagger_cymbals: STAGGER_CYMBALS_DEFAULT,
            midi_overrides: DEFAULT_MIDI_OVERRIDES.to_string(),
            combo_start: DEFAULT_COMBO_START.to_string(),
            combo_select: DEFAULT_COMBO_SELECT.to_string(),
            combo_toggle_hold_kick: DEFAULT_COMBO_TOGGLE_HOLD_KICK.to_string(),
            midi_cc_status: MIDI_CC_STATUS_DEFAULT,
            midi_cc_number: MIDI_CC_NUMBER_DEFAULT,
            midi_cc_threshold: MIDI_CC_THRESHOLD_DEFAULT,
            midi_cc_invert_threshold: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_file_name() {
        assert_eq!(CONFIG_FILE_NAME, "rb3drums.yml");
    }

    #[test]
    fn numeric_bounds_match_cpp() {
        assert_eq!(PULSE_MS_MIN, 1);
        assert_eq!(PULSE_MS_MAX, 100);
        assert_eq!(MIN_VELOCITY_MAX, 127);
        assert_eq!(COMBO_WINDOW_MS_MAX, 5000);
    }

    #[test]
    fn defaults_match_cpp_header() {
        let c = Rb3DrumsConfig::default();
        assert_eq!(c.pulse_ms, 30);
        assert_eq!(c.minimum_velocity, 10);
        assert_eq!(c.combo_window_ms, 2000);
        assert!(c.stagger_cymbals);
        assert_eq!(c.midi_cc_status, 0xB0);
        assert_eq!(c.midi_cc_number, 4);
        assert_eq!(c.midi_cc_threshold, 64);
        assert!(!c.midi_cc_invert_threshold);
    }

    #[test]
    fn default_combos_match_cpp_strings() {
        assert_eq!(DEFAULT_COMBO_START, "HihatPedal,HihatPedal,HihatPedal,Snare");
        assert_eq!(DEFAULT_COMBO_SELECT, "HihatPedal,HihatPedal,HihatPedal,SnareRim");
        assert_eq!(DEFAULT_COMBO_TOGGLE_HOLD_KICK, "HihatPedal,HihatPedal,HihatPedal,Kick");
        assert_eq!(DEFAULT_MIDI_OVERRIDES, "");
    }

    #[test]
    fn defaults_within_declared_bounds() {
        assert!(PULSE_MS_DEFAULT >= PULSE_MS_MIN && PULSE_MS_DEFAULT <= PULSE_MS_MAX);
        assert!(MIN_VELOCITY_DEFAULT >= MIN_VELOCITY_MIN && MIN_VELOCITY_DEFAULT <= MIN_VELOCITY_MAX);
        assert!(COMBO_WINDOW_MS_DEFAULT <= COMBO_WINDOW_MS_MAX);
        assert!(MIDI_CC_STATUS_DEFAULT <= 255);
        assert!(MIDI_CC_NUMBER_DEFAULT <= 127);
        assert!(MIDI_CC_THRESHOLD_DEFAULT <= 127);
    }
}
