//! `rpcs3-audio-utils` — Rust port of `rpcs3/Emu/Audio/audio_utils.cpp`.
//!
//! The C++ module keeps a single `audio_fxo::audio_muted` atomic plus a
//! global `g_cfg.audio.volume` integer, and exposes three entry points:
//! `get_volume()`, `toggle_mute()`, and `change_volume(delta)`. The last is
//! the interesting one — it applies a non-linear step adjustment so that
//! keybind presses feel right at extremes:
//!
//! - at `volume < 25` any delta with `|delta| > 1` collapses to ±1 (fine
//!   control in the quiet range), and
//! - at `volume > 75` any delta with `|delta| < 5` doubles up but caps at ±5
//!   (faster climb near the top),
//! - otherwise the delta is applied as-is.
//!
//! The resulting value is then clamped to `[min, max]`.
//!
//! This crate replicates those rules byte-for-byte so frontends can reuse
//! them without depending on the full RPCS3 config stack. The UI
//! notifications (`rsx::overlays::queue_message`) and callbacks
//! (`update_emu_settings`) are out of scope; we just return the new state
//! so a wrapper can translate it to whatever overlay system it has.
#![no_std]
extern crate alloc;

/// RPCS3 config defaults (see `Emu/system_config.h` — volume is `s32` in
/// `[0, 200]` with default 100).
pub const VOLUME_MIN_DEFAULT: i32 = 0;
pub const VOLUME_MAX_DEFAULT: i32 = 200;

/// Low-volume cutoff (cpp:36). Below this, big deltas collapse to ±1 for
/// fine-grained adjustment.
pub const FINE_CONTROL_BELOW: i32 = 25;
/// High-volume cutoff (cpp:41). Above this, small deltas double up (capped
/// at ±5) so the slider climbs faster near max.
pub const FAST_STEP_ABOVE: i32 = 75;

/// Snapshot of the audio globals, mirroring the fields the C++ helpers
/// read/write: `audio_fxo::audio_muted` + `g_cfg.audio.volume` + its bounds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AudioState {
    pub muted: bool,
    pub volume: i32,
    pub min: i32,
    pub max: i32,
}

impl AudioState {
    #[must_use]
    pub const fn new(volume: i32) -> Self {
        Self { muted: false, volume, min: VOLUME_MIN_DEFAULT, max: VOLUME_MAX_DEFAULT }
    }

    #[must_use]
    pub const fn with_bounds(volume: i32, min: i32, max: i32) -> Self {
        Self { muted: false, volume, min, max }
    }
}

impl Default for AudioState {
    fn default() -> Self {
        Self::new(100)
    }
}

/// `audio::get_volume()` — returns 0.0 when muted, else `volume / 100.0`
/// (cpp:11..14). Matches the C++ `f32` narrowing.
#[must_use]
pub fn get_volume(state: &AudioState) -> f32 {
    if state.muted {
        0.0
    } else {
        state.volume as f32 / 100.0
    }
}

/// `audio::toggle_mute()` — flips the mute bit (cpp:16..23). Returns the new
/// state so the caller can drive the overlay + emu-settings callback.
pub fn toggle_mute(state: &mut AudioState) -> bool {
    state.muted = !state.muted;
    state.muted
}

/// Computed outcome of a `change_volume` call. `NoOp` is returned when the
/// state is muted (cpp:28..29) or when clamping pins the value to the same
/// spot (cpp:49..50).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VolumeChange {
    Changed { old: i32, new: i32 },
    NoOp,
}

/// Re-applies the non-linear step-sizing rules from cpp:35..45 without
/// touching any state — useful for tests and preview UIs.
#[must_use]
pub fn adjust_delta(old_volume: i32, delta: i32) -> i32 {
    let abs_delta = delta.unsigned_abs();
    if old_volume < FINE_CONTROL_BELOW && abs_delta > 1 {
        if delta > 0 { 1 } else { -1 }
    } else if old_volume > FAST_STEP_ABOVE && abs_delta < 5 {
        let scaled = delta.saturating_mul(2);
        if delta > 0 { scaled.min(5) } else { scaled.max(-5) }
    } else {
        delta
    }
}

/// `audio::change_volume(delta)` (cpp:25..56). Applies the non-linear step
/// sizing and clamps into `[min, max]`. Returns `NoOp` on mute or zero-net
/// change, `Changed { old, new }` otherwise.
pub fn change_volume(state: &mut AudioState, delta: i32) -> VolumeChange {
    if state.muted {
        return VolumeChange::NoOp;
    }

    let old = state.volume;
    let adjusted = adjust_delta(old, delta);
    let unclamped = old.saturating_add(adjusted);
    let new = unclamped.clamp(state.min, state.max);

    if new == old {
        return VolumeChange::NoOp;
    }

    state.volume = new;
    VolumeChange::Changed { old, new }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn get_volume_muted_is_zero() {
        let mut s = AudioState::new(80);
        s.muted = true;
        assert_eq!(get_volume(&s), 0.0);
    }

    #[test]
    fn get_volume_divides_by_100() {
        let s = AudioState::new(50);
        assert_eq!(get_volume(&s), 0.5);
        let s = AudioState::new(200);
        assert_eq!(get_volume(&s), 2.0);
    }

    #[test]
    fn toggle_mute_flips_bit() {
        let mut s = AudioState::default();
        assert!(!s.muted);
        assert!(toggle_mute(&mut s));
        assert!(s.muted);
        assert!(!toggle_mute(&mut s));
        assert!(!s.muted);
    }

    #[test]
    fn change_volume_noop_when_muted() {
        let mut s = AudioState::new(50);
        s.muted = true;
        assert_eq!(change_volume(&mut s, 10), VolumeChange::NoOp);
        assert_eq!(s.volume, 50);
    }

    #[test]
    fn low_volume_big_delta_collapses_to_one() {
        // volume 10, delta +5 → adjusted delta becomes +1 (fine control).
        let mut s = AudioState::new(10);
        assert_eq!(change_volume(&mut s, 5), VolumeChange::Changed { old: 10, new: 11 });
        let mut s = AudioState::new(10);
        assert_eq!(change_volume(&mut s, -5), VolumeChange::Changed { old: 10, new: 9 });
    }

    #[test]
    fn low_volume_unit_delta_preserved() {
        // abs(delta) == 1 bypasses the collapse path.
        let mut s = AudioState::new(10);
        assert_eq!(change_volume(&mut s, 1), VolumeChange::Changed { old: 10, new: 11 });
    }

    #[test]
    fn high_volume_small_delta_doubles_capped_to_five() {
        // volume 80, delta +2 → doubled to +4.
        let mut s = AudioState::new(80);
        assert_eq!(change_volume(&mut s, 2), VolumeChange::Changed { old: 80, new: 84 });
        // volume 80, delta +4 → doubled to +8 but capped at +5.
        let mut s = AudioState::new(80);
        assert_eq!(change_volume(&mut s, 4), VolumeChange::Changed { old: 80, new: 85 });
        // negative side too.
        let mut s = AudioState::new(80);
        assert_eq!(change_volume(&mut s, -4), VolumeChange::Changed { old: 80, new: 75 });
    }

    #[test]
    fn high_volume_large_delta_passes_through() {
        // abs(delta) == 5 falls out of the doubling branch.
        let mut s = AudioState::new(80);
        assert_eq!(change_volume(&mut s, 5), VolumeChange::Changed { old: 80, new: 85 });
    }

    #[test]
    fn clamp_pins_at_max_and_is_noop_when_already_max() {
        let mut s = AudioState::new(199);
        // Delta 1 goes to 200 (max); ok change.
        assert_eq!(change_volume(&mut s, 1), VolumeChange::Changed { old: 199, new: 200 });
        // Now at max: another +1 clamps back to 200 → noop.
        assert_eq!(change_volume(&mut s, 1), VolumeChange::NoOp);
    }

    #[test]
    fn clamp_pins_at_min_and_is_noop_when_already_min() {
        let mut s = AudioState::new(1);
        assert_eq!(change_volume(&mut s, -1), VolumeChange::Changed { old: 1, new: 0 });
        assert_eq!(change_volume(&mut s, -1), VolumeChange::NoOp);
    }

    #[test]
    fn mid_range_delta_passes_through() {
        let mut s = AudioState::new(50);
        assert_eq!(change_volume(&mut s, 10), VolumeChange::Changed { old: 50, new: 60 });
        assert_eq!(change_volume(&mut s, -20), VolumeChange::Changed { old: 60, new: 40 });
    }

    #[test]
    fn adjust_delta_direct_spot_checks() {
        assert_eq!(adjust_delta(10, 5), 1);
        assert_eq!(adjust_delta(10, -5), -1);
        assert_eq!(adjust_delta(10, 1), 1);
        assert_eq!(adjust_delta(80, 2), 4);
        assert_eq!(adjust_delta(80, 4), 5);
        assert_eq!(adjust_delta(80, -3), -5);
        assert_eq!(adjust_delta(50, 7), 7);
        // Boundary: 25 and 75 are NOT triggered (strict <, >).
        assert_eq!(adjust_delta(25, 10), 10);
        assert_eq!(adjust_delta(75, 2), 2);
    }
}
