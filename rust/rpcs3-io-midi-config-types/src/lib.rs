//! `rpcs3-io-midi-config-types` — Rust port of
//! `rpcs3/Emu/Io/midi_config_types.{h,cpp}`.
//!
//! MIDI device types (RPCS3 supports up to 3 simultaneously) and the
//! triple-sharp `ßßß` separator used to serialize a `{type, name}` pair
//! into a single YAML string value.
//!
//! Frozen:
//!
//! - `MAX_MIDI_DEVICES = 3` (cpp header:6).
//! - `MidiDeviceType` enum with positional discriminants matching the
//!   cpp declaration order (keyboard=0, guitar=1, guitar_22fret=2,
//!   drums=3).
//! - Pretty-print strings: `"Keyboard"`, `"Guitar (17 frets)"`,
//!   `"Guitar (22 frets)"`, `"Drums"` (cpp:11..17).
//! - `MidiDevice::from_string("<type>ßßß<name>")` parser (cpp:30..50).
//! - Serialize format `"<type>ßßß<name>"` (cpp:27 `"%sßßß%s"`).

/// Matches cpp `static constexpr usz max_midi_devices = 3`.
pub const MAX_MIDI_DEVICES: usize = 3;

/// Separator between the enum name and the device label.
pub const DEVICE_SEPARATOR: &str = "ßßß";

#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MidiDeviceType {
    Keyboard = 0,
    Guitar = 1,
    Guitar22Fret = 2,
    Drums = 3,
}

impl MidiDeviceType {
    /// Pretty-print (cpp:11..17).
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Keyboard => "Keyboard",
            Self::Guitar => "Guitar (17 frets)",
            Self::Guitar22Fret => "Guitar (22 frets)",
            Self::Drums => "Drums",
        }
    }

    /// Parse the cpp pretty-print back into a value (cpp:38 calls
    /// `try_to_enum_value` against the same format fn).
    pub fn from_name(s: &str) -> Option<Self> {
        match s {
            "Keyboard" => Some(Self::Keyboard),
            "Guitar (17 frets)" => Some(Self::Guitar),
            "Guitar (22 frets)" => Some(Self::Guitar22Fret),
            "Drums" => Some(Self::Drums),
            _ => None,
        }
    }
}

impl Default for MidiDeviceType {
    fn default() -> Self {
        Self::Keyboard
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MidiDevice {
    pub type_: MidiDeviceType,
    pub name: String,
}

impl MidiDevice {
    /// `midi_device::from_string(str)` (cpp:30..50). Splits on
    /// `DEVICE_SEPARATOR`; the first part is parsed as a
    /// `MidiDeviceType`, the second (if present) becomes the label.
    /// Unrecognized type strings leave `type_` at the default
    /// (`Keyboard`) — mirroring the cpp behavior where
    /// `try_to_enum_value` silently fails.
    pub fn from_string(str: &str) -> Self {
        let mut res = MidiDevice::default();
        let mut parts = str.splitn(2, DEVICE_SEPARATOR);

        if let Some(type_str) = parts.next() {
            if let Some(t) = MidiDeviceType::from_name(type_str) {
                res.type_ = t;
            }
        }
        if let Some(name_str) = parts.next() {
            res.name = name_str.to_string();
        }
        res
    }

    /// Serialize to the cpp string format `"%sßßß%s"` (cpp:27).
    #[must_use]
    pub fn to_string_cpp(&self) -> String {
        format!("{}{}{}", self.type_.name(), DEVICE_SEPARATOR, self.name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn max_devices_constant() {
        assert_eq!(MAX_MIDI_DEVICES, 3);
    }

    #[test]
    fn separator_is_triple_sharp_s() {
        assert_eq!(DEVICE_SEPARATOR, "ßßß");
    }

    #[test]
    fn type_discriminants_match_cpp_declaration_order() {
        assert_eq!(MidiDeviceType::Keyboard as u32, 0);
        assert_eq!(MidiDeviceType::Guitar as u32, 1);
        assert_eq!(MidiDeviceType::Guitar22Fret as u32, 2);
        assert_eq!(MidiDeviceType::Drums as u32, 3);
    }

    #[test]
    fn pretty_print_strings_match_cpp() {
        assert_eq!(MidiDeviceType::Keyboard.name(), "Keyboard");
        assert_eq!(MidiDeviceType::Guitar.name(), "Guitar (17 frets)");
        assert_eq!(MidiDeviceType::Guitar22Fret.name(), "Guitar (22 frets)");
        assert_eq!(MidiDeviceType::Drums.name(), "Drums");
    }

    #[test]
    fn from_name_roundtrip() {
        for v in [
            MidiDeviceType::Keyboard,
            MidiDeviceType::Guitar,
            MidiDeviceType::Guitar22Fret,
            MidiDeviceType::Drums,
        ] {
            assert_eq!(MidiDeviceType::from_name(v.name()), Some(v));
        }
        assert_eq!(MidiDeviceType::from_name("unknown"), None);
    }

    #[test]
    fn midi_device_default() {
        let d = MidiDevice::default();
        assert_eq!(d.type_, MidiDeviceType::Keyboard);
        assert_eq!(d.name, "");
    }

    #[test]
    fn from_string_parses_type_and_name() {
        let d = MidiDevice::from_string("Guitar (22 frets)ßßßRB3 Mustang");
        assert_eq!(d.type_, MidiDeviceType::Guitar22Fret);
        assert_eq!(d.name, "RB3 Mustang");
    }

    #[test]
    fn from_string_unknown_type_leaves_default() {
        let d = MidiDevice::from_string("Not a typeßßßSome Name");
        assert_eq!(d.type_, MidiDeviceType::Keyboard, "fallback to default");
        assert_eq!(d.name, "Some Name");
    }

    #[test]
    fn from_string_missing_name_leaves_empty() {
        let d = MidiDevice::from_string("Drums");
        assert_eq!(d.type_, MidiDeviceType::Drums);
        assert_eq!(d.name, "");
    }

    #[test]
    fn to_string_roundtrip() {
        let d = MidiDevice {
            type_: MidiDeviceType::Drums,
            name: "Alesis Nitro".to_string(),
        };
        let s = d.to_string_cpp();
        assert_eq!(s, "DrumsßßßAlesis Nitro");
    }

    #[test]
    fn full_roundtrip_from_and_to() {
        let original = MidiDevice {
            type_: MidiDeviceType::Guitar,
            name: "Fender".to_string(),
        };
        let s = original.to_string_cpp();
        let back = MidiDevice::from_string(&s);
        assert_eq!(back, original);
    }

    #[test]
    fn name_with_separator_inside_still_parses_first_split() {
        // `splitn(2, ...)` stops after the first separator, so a trailing
        // separator inside the name survives into the parsed label.
        let d = MidiDevice::from_string("Keyboardßßßhas ßßß inside");
        assert_eq!(d.type_, MidiDeviceType::Keyboard);
        assert_eq!(d.name, "has ßßß inside");
    }
}
