//! `rpcs3-audio-device-enumerator` — Rust port of
//! `rpcs3/Emu/Audio/audio_device_enumerator.h` plus the post-processing
//! that `cubeb_enumerator.cpp` and `faudio_enumerator.cpp` perform after
//! scraping raw device info from their respective libraries.
//!
//! The raw enumeration calls (`cubeb_enumerate_devices`,
//! `FAudio_GetDeviceCount`/`FAudio_GetDeviceDetails`) are OS/library
//! calls that each frontend integrates differently. What's **portable
//! and byte-identical** is the filtering + normalization applied to the
//! scraped results:
//!
//! **Cubeb** (cpp:71..110):
//! - Skip devices with `state == CUBEB_DEVICE_STATE_UNPLUGGED`.
//! - Skip devices with empty `device_id`.
//! - If `friendly_name` is empty, use the id as the name.
//! - Sort final list by `name` ascending.
//!
//! **FAudio** (cpp:57..88):
//! - Use `str(dev_idx)` as id.
//! - Use UTF16→UTF8 of `DisplayName` as name; if empty, `"Device {id}"`.
//! - Sort final list by `name` ascending.
//!
//! We expose a shared `AudioDevice` struct and two pipelines
//! (`normalize_cubeb` and `normalize_faudio`) so frontends only need to
//! feed in the raw tuples from each library.

use std::cmp::Ordering;

/// Cubeb device state from `cubeb.h` — only the two values we act on.
/// Byte discriminants match the upstream header so passing `state as i32`
/// is safe.
#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CubebDeviceState {
    Disabled = 0,
    Unplugged = 1,
    Enabled = 2,
}

/// `audio_device_enumerator::audio_device` (the cpp struct).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AudioDevice {
    pub id: String,
    pub name: String,
    pub max_ch: u32,
}

/// Raw scrape result from cubeb. Mirrors the fields the cpp reads.
#[derive(Debug, Clone)]
pub struct CubebRawDevice {
    pub device_id: String,
    pub friendly_name: String,
    pub max_channels: u32,
    pub state: CubebDeviceState,
}

/// Raw scrape result from FAudio. The index is synthesised as the id.
#[derive(Debug, Clone)]
pub struct FaudioRawDevice {
    /// `dev_idx` from the enumeration loop.
    pub index: u32,
    /// Already UTF-16→UTF-8 decoded display name.
    pub display_name: String,
    pub num_channels: u32,
}

/// Normalize a Cubeb raw scrape into the final `AudioDevice` list (cpp:71..110).
/// Skipping rules and name fallback match cpp byte-for-byte; output is
/// sorted by `name` ascending.
#[must_use]
pub fn normalize_cubeb(devices: impl IntoIterator<Item = CubebRawDevice>) -> Vec<AudioDevice> {
    let mut out: Vec<AudioDevice> = Vec::new();
    for raw in devices {
        if raw.state == CubebDeviceState::Unplugged {
            continue;
        }
        if raw.device_id.is_empty() {
            // cpp logs "Empty device id - skipping".
            continue;
        }
        let name = if raw.friendly_name.is_empty() {
            raw.device_id.clone()
        } else {
            raw.friendly_name.clone()
        };
        out.push(AudioDevice { id: raw.device_id, name, max_ch: raw.max_channels });
    }
    sort_by_name(&mut out);
    out
}

/// Normalize a FAudio raw scrape (cpp:57..88). Same shape, different name
/// fallback: `"Device {id}"`.
#[must_use]
pub fn normalize_faudio(devices: impl IntoIterator<Item = FaudioRawDevice>) -> Vec<AudioDevice> {
    let mut out: Vec<AudioDevice> = Vec::new();
    for raw in devices {
        let id = raw.index.to_string();
        let name = if raw.display_name.is_empty() {
            format!("Device {id}")
        } else {
            raw.display_name.clone()
        };
        out.push(AudioDevice { id, name, max_ch: raw.num_channels });
    }
    sort_by_name(&mut out);
    out
}

/// Stable-sort the list by `name` (cpp uses `std::sort` which is unstable,
/// but the outputs are compared byte-for-byte with `<` so ordering among
/// equal names doesn't matter for device identity).
fn sort_by_name(list: &mut Vec<AudioDevice>) {
    list.sort_by(|a, b| match a.name.cmp(&b.name) {
        Ordering::Equal => Ordering::Equal,
        other => other,
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn raw_cubeb(id: &str, name: &str, ch: u32, state: CubebDeviceState) -> CubebRawDevice {
        CubebRawDevice {
            device_id: id.to_string(),
            friendly_name: name.to_string(),
            max_channels: ch,
            state,
        }
    }

    #[test]
    fn cubeb_state_discriminants() {
        assert_eq!(CubebDeviceState::Disabled as u32, 0);
        assert_eq!(CubebDeviceState::Unplugged as u32, 1);
        assert_eq!(CubebDeviceState::Enabled as u32, 2);
    }

    #[test]
    fn cubeb_skips_unplugged() {
        let raws = vec![
            raw_cubeb("id1", "alpha", 2, CubebDeviceState::Unplugged),
            raw_cubeb("id2", "beta", 6, CubebDeviceState::Enabled),
        ];
        let devs = normalize_cubeb(raws);
        assert_eq!(devs.len(), 1);
        assert_eq!(devs[0].id, "id2");
    }

    #[test]
    fn cubeb_skips_empty_id() {
        let raws = vec![
            raw_cubeb("", "alpha", 2, CubebDeviceState::Enabled),
            raw_cubeb("id2", "beta", 6, CubebDeviceState::Enabled),
        ];
        let devs = normalize_cubeb(raws);
        assert_eq!(devs.len(), 1);
        assert_eq!(devs[0].id, "id2");
    }

    #[test]
    fn cubeb_falls_back_to_id_when_name_empty() {
        let raws = vec![raw_cubeb("dev-id-abc", "", 8, CubebDeviceState::Enabled)];
        let devs = normalize_cubeb(raws);
        assert_eq!(devs[0].name, "dev-id-abc");
        assert_eq!(devs[0].id, "dev-id-abc");
    }

    #[test]
    fn cubeb_sorts_by_name_ascending() {
        let raws = vec![
            raw_cubeb("id1", "Zeta", 2, CubebDeviceState::Enabled),
            raw_cubeb("id2", "Alpha", 2, CubebDeviceState::Enabled),
            raw_cubeb("id3", "Mu", 2, CubebDeviceState::Enabled),
        ];
        let devs = normalize_cubeb(raws);
        assert_eq!(devs.iter().map(|d| &d.name).collect::<Vec<_>>(), vec!["Alpha", "Mu", "Zeta"]);
    }

    #[test]
    fn cubeb_max_ch_preserved() {
        let raws = vec![raw_cubeb("id1", "Speaker", 8, CubebDeviceState::Enabled)];
        let devs = normalize_cubeb(raws);
        assert_eq!(devs[0].max_ch, 8);
    }

    #[test]
    fn faudio_uses_index_as_id() {
        let raws = vec![
            FaudioRawDevice { index: 0, display_name: "Primary".into(), num_channels: 6 },
            FaudioRawDevice { index: 3, display_name: "Aux".into(), num_channels: 2 },
        ];
        let devs = normalize_faudio(raws);
        // Sorted by name: Aux, Primary.
        assert_eq!(devs[0].id, "3");
        assert_eq!(devs[1].id, "0");
    }

    #[test]
    fn faudio_fallback_name_is_device_plus_id() {
        let raws = vec![FaudioRawDevice {
            index: 7,
            display_name: String::new(),
            num_channels: 2,
        }];
        let devs = normalize_faudio(raws);
        assert_eq!(devs[0].name, "Device 7");
    }

    #[test]
    fn faudio_sorts_by_name() {
        let raws = vec![
            FaudioRawDevice { index: 0, display_name: "Zed".into(), num_channels: 2 },
            FaudioRawDevice { index: 1, display_name: "Apple".into(), num_channels: 2 },
        ];
        let devs = normalize_faudio(raws);
        assert_eq!(&devs[0].name, "Apple");
        assert_eq!(&devs[1].name, "Zed");
    }

    #[test]
    fn empty_inputs_yield_empty_outputs() {
        let devs = normalize_cubeb(Vec::<CubebRawDevice>::new());
        assert!(devs.is_empty());
        let devs = normalize_faudio(Vec::<FaudioRawDevice>::new());
        assert!(devs.is_empty());
    }

    #[test]
    fn faudio_max_ch_from_num_channels() {
        let raws = vec![FaudioRawDevice {
            index: 0,
            display_name: "X".into(),
            num_channels: 6,
        }];
        let devs = normalize_faudio(raws);
        assert_eq!(devs[0].max_ch, 6);
    }
}
