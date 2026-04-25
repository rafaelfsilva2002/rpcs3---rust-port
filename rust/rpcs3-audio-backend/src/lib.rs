//! `rpcs3-audio-backend` — Rust port of `rpcs3/Emu/Audio/AudioBackend.cpp`.
//!
//! The C++ `AudioBackend` class is mostly an abstract base + a pile of
//! host-agnostic DSP helpers. The abstract side (device enumeration,
//! callbacks, `Open`/`Close`/`Play`) is a host-specific concern each
//! frontend implements against Cubeb / FAudio / XAudio2. The DSP side is
//! universal, and that's what this crate covers:
//!
//! - `convert_to_s16` — float → S16 with the `* 32768.5f` scale the C++
//!   source uses (cpp:50..56), clamped to `[-32768, 32767]`.
//! - `apply_volume_static` — mute + unity + multiply fast paths
//!   (cpp:109..134). Mute memsets zeros; unity memcopies (or skips if
//!   src==dst); else sample * vol.
//! - `apply_volume` — linear ramp over `VOLUME_CHANGE_DURATION=0.032s`,
//!   with the per-sample increment stepping toward `target_volume` and a
//!   `1e-6` epsilon to avoid infinite ramps (cpp:58..107).
//! - `normalize` — soft-clip tanh above 0.95, hard-clip at 1.0
//!   (cpp:136..170).
//! - `default_layout_channel_count(layout)` / `default_layout(channels)` /
//!   `layout_channel_count(channels, layout)` — layout table from
//!   cpp:205..244. Quirk preserved: 7 input channels → `surround_5_1`,
//!   not `surround_7_1` (cpp:240).
//! - `setup_channel_layout` — returns `(layout, channels)` after the
//!   fallback cascade in cpp:246..264.
use rpcs3_audio_resampler::AudioChannelCnt;

/// Linear-ramp duration for `apply_volume` transitions (cpp:400).
pub const VOLUME_CHANGE_DURATION: f32 = 0.032;

/// `audio_channel_layout` enum from `Emu/system_config_types.h:97..107`.
/// Discriminants are positional (0..=7) because the C++ enum is plain.
#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AudioChannelLayout {
    Automatic = 0,
    Mono = 1,
    Stereo = 2,
    StereoLfe = 3,
    Quadraphonic = 4,
    QuadraphonicLfe = 5,
    Surround5_1 = 6,
    Surround7_1 = 7,
}

/// `AudioBackend::VolumeParam` (AudioBackend.h:52..59). Holds the live
/// ramp state so a subsequent call can pick up where the previous one
/// left off.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VolumeParam {
    pub initial_volume: f32,
    pub current_volume: f32,
    pub target_volume: f32,
    pub freq: u32,
    pub ch_cnt: u32,
}

impl Default for VolumeParam {
    fn default() -> Self {
        Self {
            initial_volume: 1.0,
            current_volume: 1.0,
            target_volume: 1.0,
            freq: 48_000,
            ch_cnt: 2,
        }
    }
}

/// `AudioBackend::convert_to_s16(cnt, src, dst)` (cpp:50..56).
pub fn convert_to_s16(src: &[f32], dst: &mut [i16]) {
    let n = src.len().min(dst.len());
    for i in 0..n {
        // cpp uses 32768.5f then clamps to [-32768, 32767].
        let scaled = (src[i] * 32768.5).clamp(-32_768.0, 32_767.0);
        dst[i] = scaled as i16;
    }
}

/// `AudioBackend::apply_volume_static(vol, cnt, src, dst)` (cpp:109..134).
/// Unity + mute fast paths preserved.
pub fn apply_volume_static(vol: f32, src: &[f32], dst: &mut [f32]) {
    let n = src.len().min(dst.len());
    if vol == 1.0 {
        // cpp skips memcpy when src==dst; we can't observe that through slices,
        // so just copy unconditionally — callers can avoid this path by
        // passing the same buffer in both slots and checking `vol` themselves.
        dst[..n].copy_from_slice(&src[..n]);
        return;
    }
    if vol == 0.0 {
        dst[..n].fill(0.0);
        return;
    }
    for i in 0..n {
        dst[i] = src[i] * vol;
    }
}

/// `AudioBackend::apply_volume(param, sample_cnt, src, dst)` (cpp:58..107).
/// Linearly ramps `current_volume` toward `target_volume` across the buffer,
/// then fills any trailing samples at `target_volume`. Returns the volume
/// that ended the ramp so the next call can continue from there.
///
/// Requires `ch_cnt > 1 && ch_cnt % 2 == 0` (cpp:60 `ensure`). Violations
/// panic here too — this is an internal invariant, not user input.
pub fn apply_volume(param: VolumeParam, src: &[f32], dst: &mut [f32]) -> f32 {
    assert!(
        param.ch_cnt > 1 && param.ch_cnt.is_multiple_of(2),
        "ch_cnt must be even and >= 2 (matches cpp:60 ensure)"
    );

    let sample_cnt = src.len().min(dst.len());

    if (param.current_volume - param.target_volume).abs() == 0.0 {
        apply_volume_static(param.target_volume, &src[..sample_cnt], &mut dst[..sample_cnt]);
        return param.target_volume;
    }

    let freq = param.freq.max(1) as f32;
    let vol_incr = (param.target_volume - param.initial_volume) / (VOLUME_CHANGE_DURATION * freq);
    let mut crnt_vol = param.current_volume;
    let mut sample_idx = 0usize;
    const EPSILON: f32 = 1e-6;
    let ch = param.ch_cnt as usize;

    if vol_incr >= 0.0 {
        while sample_idx < sample_cnt && (param.target_volume - crnt_vol) > EPSILON {
            crnt_vol = (crnt_vol + vol_incr).min(param.target_volume);
            for i in 0..ch {
                if sample_idx + i < sample_cnt {
                    dst[sample_idx + i] = src[sample_idx + i] * crnt_vol;
                }
            }
            sample_idx += ch;
        }
    } else {
        while sample_idx < sample_cnt && (crnt_vol - param.target_volume) > EPSILON {
            crnt_vol = (crnt_vol + vol_incr).max(param.target_volume);
            for i in 0..ch {
                if sample_idx + i < sample_cnt {
                    dst[sample_idx + i] = src[sample_idx + i] * crnt_vol;
                }
            }
            sample_idx += ch;
        }
    }

    if sample_cnt > sample_idx {
        apply_volume_static(
            param.target_volume,
            &src[sample_idx..sample_cnt],
            &mut dst[sample_idx..sample_cnt],
        );
    }

    crnt_vol
}

/// `AudioBackend::normalize(cnt, src, dst)` (cpp:136..170). Soft-clip with a
/// tanh-shaped curve above 0.95, hard-clip at 1.0.
pub fn normalize(src: &[f32], dst: &mut [f32]) {
    const SOFT_CLIP_THRESHOLD: f32 = 0.95;
    const HARD_CLIP_LIMIT: f32 = 1.0;
    let n = src.len().min(dst.len());
    for i in 0..n {
        let sample = src[i];
        let abs_sample = sample.abs();
        if abs_sample > SOFT_CLIP_THRESHOLD {
            let sign = if sample >= 0.0 { 1.0 } else { -1.0 };
            if abs_sample > HARD_CLIP_LIMIT {
                dst[i] = sign * HARD_CLIP_LIMIT;
            } else {
                let excess = (abs_sample - SOFT_CLIP_THRESHOLD)
                    / (HARD_CLIP_LIMIT - SOFT_CLIP_THRESHOLD);
                let soft_factor = SOFT_CLIP_THRESHOLD
                    + (HARD_CLIP_LIMIT - SOFT_CLIP_THRESHOLD) * excess.tanh();
                dst[i] = sign * soft_factor;
            }
        } else {
            dst[i] = sample;
        }
    }
}

/// `AudioBackend::default_layout_channel_count(layout)` (cpp:205..218).
/// Returns `None` for `Automatic` (C++ version throws).
#[must_use]
pub const fn default_layout_channel_count(layout: AudioChannelLayout) -> Option<u32> {
    match layout {
        AudioChannelLayout::Mono => Some(1),
        AudioChannelLayout::Stereo => Some(2),
        AudioChannelLayout::StereoLfe => Some(3),
        AudioChannelLayout::Quadraphonic => Some(4),
        AudioChannelLayout::QuadraphonicLfe => Some(5),
        AudioChannelLayout::Surround5_1 => Some(6),
        AudioChannelLayout::Surround7_1 => Some(8),
        AudioChannelLayout::Automatic => None,
    }
}

/// `AudioBackend::default_layout(channels)` (cpp:230..244). Note the quirk
/// at cpp:240: 7 channels maps to `surround_5_1`, not `surround_7_1` — we
/// preserve that exactly.
#[must_use]
pub const fn default_layout(channels: u32) -> AudioChannelLayout {
    match channels {
        1 => AudioChannelLayout::Mono,
        2 => AudioChannelLayout::Stereo,
        3 => AudioChannelLayout::StereoLfe,
        4 => AudioChannelLayout::Quadraphonic,
        5 => AudioChannelLayout::QuadraphonicLfe,
        6 => AudioChannelLayout::Surround5_1,
        // cpp:240 — 7 falls back to 5.1, not 7.1.
        7 => AudioChannelLayout::Surround5_1,
        8 => AudioChannelLayout::Surround7_1,
        _ => AudioChannelLayout::Stereo,
    }
}

/// `AudioBackend::layout_channel_count(channels, layout)` (cpp:220..228).
/// Zero channels is invalid — returns 0 (C++ throws; callers can check).
#[must_use]
pub fn layout_channel_count(channels: u32, layout: AudioChannelLayout) -> u32 {
    if channels == 0 {
        return 0;
    }
    let default = default_layout_channel_count(layout).unwrap_or(channels);
    channels.min(default)
}

/// Log severity tag emitted by the fallback cascade so callers can hook it
/// into their logger without depending on RPCS3's `logs::channel`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SetupWarning {
    /// cpp:252 — `"Mixing from %d to %d channels is not implemented."`
    MixFromTo { from: u32, to: u32 },
    /// cpp:258 — `"Can't use layout %s with %d channels."`
    LayoutIncompatible { layout: AudioChannelLayout, channels: u32 },
}

/// Result of the `setup_channel_layout` flow, preserving the warning
/// cascade from cpp:246..264 as structured data.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ChannelLayoutSetup {
    pub layout: AudioChannelLayout,
    pub channels: u32,
    pub warning: Option<SetupWarning>,
}

/// `AudioBackend::setup_channel_layout(in_ch, out_ch, layout)` (cpp:246..264).
#[must_use]
pub fn setup_channel_layout(
    input_channel_count: u32,
    output_channel_count: u32,
    mut layout: AudioChannelLayout,
) -> ChannelLayoutSetup {
    let channels = input_channel_count.min(output_channel_count);
    let mut warning = None;

    if layout != AudioChannelLayout::Automatic && output_channel_count > input_channel_count {
        warning = Some(SetupWarning::MixFromTo {
            from: input_channel_count,
            to: output_channel_count,
        });
        layout = AudioChannelLayout::Automatic;
    }

    if layout != AudioChannelLayout::Automatic {
        let need = default_layout_channel_count(layout).unwrap_or(channels);
        if channels < need {
            warning = Some(SetupWarning::LayoutIncompatible { layout, channels });
            layout = AudioChannelLayout::Automatic;
        }
    }

    let resolved_layout = if layout == AudioChannelLayout::Automatic {
        default_layout(channels)
    } else {
        layout
    };
    let resolved_channels = layout_channel_count(channels, resolved_layout);

    ChannelLayoutSetup {
        layout: resolved_layout,
        channels: resolved_channels,
        warning,
    }
}

/// `get_max_channel_count` contribution from a single declared sound mode
/// (cpp:180..203). Given the list of channel counts a device reports, the
/// C++ loop returns `STEREO` by default, upgrades to `5.1` on seeing a
/// 6-channel mode, and returns `7.1` immediately on seeing an 8-channel
/// mode.
#[must_use]
pub fn max_channel_count_from_sound_modes(modes_channels: &[u8]) -> AudioChannelCnt {
    let mut count = AudioChannelCnt::Stereo;
    for &ch in modes_channels {
        match ch {
            6 => count = AudioChannelCnt::Surround5_1,
            8 => return AudioChannelCnt::Surround7_1,
            _ => {}
        }
    }
    count
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn convert_to_s16_scales_and_clamps() {
        let src = [0.0f32, 1.0, -1.0, 0.5, -0.5, 2.0, -2.0];
        let mut dst = [0i16; 7];
        convert_to_s16(&src, &mut dst);
        // 0.0 * 32768.5 = 0
        assert_eq!(dst[0], 0);
        // 1.0 * 32768.5 = 32768.5 → clamped to 32767
        assert_eq!(dst[1], 32_767);
        // -1.0 * 32768.5 = -32768.5 → clamped to -32768
        assert_eq!(dst[2], -32_768);
        // 0.5 * 32768.5 = 16384.25 → truncated to 16384
        assert_eq!(dst[3], 16_384);
        // -0.5 * 32768.5 = -16384.25 → truncated to -16384
        assert_eq!(dst[4], -16_384);
        // 2.0 / -2.0 both clamp at saturation
        assert_eq!(dst[5], 32_767);
        assert_eq!(dst[6], -32_768);
    }

    #[test]
    fn apply_volume_static_unity_copies() {
        let src = [0.1f32, 0.2, 0.3, 0.4];
        let mut dst = [9.9f32; 4];
        apply_volume_static(1.0, &src, &mut dst);
        assert_eq!(dst, src);
    }

    #[test]
    fn apply_volume_static_mute_zeros() {
        let src = [0.1f32, 0.2, 0.3, 0.4];
        let mut dst = [9.9f32; 4];
        apply_volume_static(0.0, &src, &mut dst);
        assert_eq!(dst, [0.0; 4]);
    }

    #[test]
    fn apply_volume_static_scales() {
        let src = [1.0f32, 2.0, 4.0, 8.0];
        let mut dst = [0.0; 4];
        apply_volume_static(0.5, &src, &mut dst);
        assert_eq!(dst, [0.5, 1.0, 2.0, 4.0]);
    }

    #[test]
    fn apply_volume_flat_current_equals_target_is_static() {
        let src = [1.0f32; 8];
        let mut dst = [0.0; 8];
        let p = VolumeParam {
            initial_volume: 0.5,
            current_volume: 0.5,
            target_volume: 0.5,
            freq: 48_000,
            ch_cnt: 2,
        };
        let end = apply_volume(p, &src, &mut dst);
        assert_eq!(end, 0.5);
        assert_eq!(dst, [0.5; 8]);
    }

    #[test]
    fn apply_volume_ramp_up_reaches_target() {
        // Small freq + tiny buffer so we converge quickly within the ramp.
        let src = [1.0f32; 64];
        let mut dst = [0.0; 64];
        let p = VolumeParam {
            initial_volume: 0.0,
            current_volume: 0.0,
            target_volume: 1.0,
            freq: 1_000, // ramp spans 0.032*1000 = 32 steps
            ch_cnt: 2,
        };
        let end = apply_volume(p, &src, &mut dst);
        // We should either reach the target or be within epsilon.
        assert!((end - 1.0).abs() <= 1e-3, "end = {end}");
        // Last samples should be at target (fill phase).
        assert!((dst[63] - 1.0).abs() < 1e-3, "trailing fill = {}", dst[63]);
    }

    #[test]
    fn apply_volume_ramp_down_reaches_target() {
        let src = [1.0f32; 64];
        let mut dst = [0.0; 64];
        let p = VolumeParam {
            initial_volume: 1.0,
            current_volume: 1.0,
            target_volume: 0.0,
            freq: 1_000,
            ch_cnt: 2,
        };
        let end = apply_volume(p, &src, &mut dst);
        assert!((end - 0.0).abs() <= 1e-3, "end = {end}");
        assert!((dst[63] - 0.0).abs() < 1e-3, "trailing fill = {}", dst[63]);
    }

    #[test]
    #[should_panic(expected = "ch_cnt must be even")]
    fn apply_volume_panics_on_odd_ch_cnt() {
        let src = [1.0f32; 8];
        let mut dst = [0.0; 8];
        let p = VolumeParam { ch_cnt: 3, ..VolumeParam::default() };
        let _ = apply_volume(p, &src, &mut dst);
    }

    #[test]
    fn normalize_passthrough_below_threshold() {
        let src = [0.0f32, 0.5, -0.5, 0.94, -0.94];
        let mut dst = [0.0; 5];
        normalize(&src, &mut dst);
        assert_eq!(dst, src);
    }

    #[test]
    fn normalize_hard_clips_above_one() {
        let src = [1.5f32, -2.0, 1.0001];
        let mut dst = [0.0; 3];
        normalize(&src, &mut dst);
        assert_eq!(dst[0], 1.0);
        assert_eq!(dst[1], -1.0);
        assert_eq!(dst[2], 1.0);
    }

    #[test]
    fn normalize_soft_clips_between_threshold_and_limit() {
        let src = [0.975f32, -0.975, 0.999];
        let mut dst = [0.0; 3];
        normalize(&src, &mut dst);
        // Expect a value slightly above 0.95 but strictly below the input.
        for (i, &s) in src.iter().enumerate() {
            let out = dst[i];
            let abs_in = s.abs();
            let abs_out = out.abs();
            assert!(abs_out > 0.95, "abs(out)={abs_out} must exceed threshold");
            assert!(abs_out < abs_in, "abs(out)={abs_out} must dip below abs(in)={abs_in}");
            assert_eq!(out.signum(), s.signum());
        }
    }

    #[test]
    fn default_layout_channel_count_table() {
        assert_eq!(default_layout_channel_count(AudioChannelLayout::Mono), Some(1));
        assert_eq!(default_layout_channel_count(AudioChannelLayout::Stereo), Some(2));
        assert_eq!(default_layout_channel_count(AudioChannelLayout::StereoLfe), Some(3));
        assert_eq!(default_layout_channel_count(AudioChannelLayout::Quadraphonic), Some(4));
        assert_eq!(default_layout_channel_count(AudioChannelLayout::QuadraphonicLfe), Some(5));
        assert_eq!(default_layout_channel_count(AudioChannelLayout::Surround5_1), Some(6));
        assert_eq!(default_layout_channel_count(AudioChannelLayout::Surround7_1), Some(8));
        assert_eq!(default_layout_channel_count(AudioChannelLayout::Automatic), None);
    }

    #[test]
    fn default_layout_preserves_7ch_quirk() {
        // cpp:240 — 7 channels maps to surround_5_1, not 7.1.
        assert_eq!(default_layout(7), AudioChannelLayout::Surround5_1);
    }

    #[test]
    fn default_layout_standard_cases() {
        assert_eq!(default_layout(1), AudioChannelLayout::Mono);
        assert_eq!(default_layout(2), AudioChannelLayout::Stereo);
        assert_eq!(default_layout(6), AudioChannelLayout::Surround5_1);
        assert_eq!(default_layout(8), AudioChannelLayout::Surround7_1);
        // out-of-range falls back to stereo.
        assert_eq!(default_layout(0), AudioChannelLayout::Stereo);
        assert_eq!(default_layout(42), AudioChannelLayout::Stereo);
    }

    #[test]
    fn layout_channel_count_clips() {
        assert_eq!(layout_channel_count(2, AudioChannelLayout::Surround7_1), 2);
        assert_eq!(layout_channel_count(8, AudioChannelLayout::Stereo), 2);
        assert_eq!(layout_channel_count(6, AudioChannelLayout::Surround5_1), 6);
        assert_eq!(layout_channel_count(0, AudioChannelLayout::Stereo), 0);
    }

    #[test]
    fn setup_channel_layout_outch_greater_triggers_fallback() {
        let setup = setup_channel_layout(2, 6, AudioChannelLayout::Surround5_1);
        assert_eq!(
            setup.warning,
            Some(SetupWarning::MixFromTo { from: 2, to: 6 })
        );
        // Automatic → default_layout(min(2,6))==Stereo
        assert_eq!(setup.layout, AudioChannelLayout::Stereo);
        assert_eq!(setup.channels, 2);
    }

    #[test]
    fn setup_channel_layout_too_few_channels_for_layout_falls_back() {
        let setup = setup_channel_layout(4, 8, AudioChannelLayout::Surround7_1);
        assert_eq!(
            setup.warning,
            Some(SetupWarning::MixFromTo { from: 4, to: 8 })
        );
        assert_eq!(setup.layout, AudioChannelLayout::Quadraphonic);
    }

    #[test]
    fn setup_channel_layout_automatic_preserves() {
        let setup = setup_channel_layout(6, 6, AudioChannelLayout::Automatic);
        assert_eq!(setup.warning, None);
        assert_eq!(setup.layout, AudioChannelLayout::Surround5_1);
        assert_eq!(setup.channels, 6);
    }

    #[test]
    fn max_channel_count_from_sound_modes_selects_max() {
        assert_eq!(max_channel_count_from_sound_modes(&[]), AudioChannelCnt::Stereo);
        assert_eq!(max_channel_count_from_sound_modes(&[2]), AudioChannelCnt::Stereo);
        assert_eq!(max_channel_count_from_sound_modes(&[2, 6]), AudioChannelCnt::Surround5_1);
        // 8 short-circuits even if earlier modes were 6.
        assert_eq!(
            max_channel_count_from_sound_modes(&[6, 8, 6]),
            AudioChannelCnt::Surround7_1
        );
    }

    #[test]
    fn volume_change_duration_frozen() {
        assert_eq!(VOLUME_CHANGE_DURATION, 0.032);
    }
}
