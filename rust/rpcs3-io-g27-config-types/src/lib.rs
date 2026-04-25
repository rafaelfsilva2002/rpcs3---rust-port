//! `rpcs3-io-g27-config-types` — Rust port of
//! `rpcs3/Emu/Io/LogitechG27Config.{h,cpp}` (only the pure value types).
//!
//! Logitech G27 emulator uses SDL3 input. The config stores bindings
//! keyed by an `(sdl_mapping_type, id, hat_component, reverse)` tuple
//! plus a packed 64-bit device-type identifier that bakes the device's
//! geometry (buttons/hats/axes counts + vendor/product IDs) into the
//! config key so two devices that happen to share a vendor/product pair
//! but differ in axis count don't overwrite each other's bindings.
//!
//! Frozen:
//!
//! - `SdlMappingType` enum (`button=0`, `hat=1`, `axis=2`).
//! - `HatComponent` enum (`none=0`, `up`, `down`, `left`, `right`).
//! - Pretty-print strings byte-exato from cpp:11..20 and cpp:27..36.
//! - `EmulatedG27DeviceTypeId::as_u64()` bit-packing from h:34..42:
//!   - `product_id` in bits 0..=15
//!   - `vendor_id` in bits 16..=31
//!   - `num_axes` masked to 10 bits, shifted to bits 32..=41
//!   - `num_hats` masked to 10 bits, shifted to bits 42..=51
//!   - `num_buttons` masked to 10 bits, shifted to bits 52..=61

#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SdlMappingType {
    Button = 0,
    Hat = 1,
    Axis = 2,
}

impl SdlMappingType {
    /// Pretty-print (cpp:11..20).
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Button => "button",
            Self::Hat => "hat",
            Self::Axis => "axis",
        }
    }
}

#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HatComponent {
    None = 0,
    Up = 1,
    Down = 2,
    Left = 3,
    Right = 4,
}

impl HatComponent {
    /// Pretty-print (cpp:27..36). Note that `none` renders as empty.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Up => "up",
            Self::Down => "down",
            Self::Left => "left",
            Self::Right => "right",
            Self::None => "",
        }
    }
}

/// Packed device-type identifier (`emulated_g27_device_type_id` at
/// `LogitechG27Config.h:25..43`). Wire-format is a single `u64` with
/// the bit layout from cpp:34..42.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct EmulatedG27DeviceTypeId {
    pub product_id: u64,
    pub vendor_id: u64,
    pub num_axes: u64,
    pub num_hats: u64,
    pub num_buttons: u64,
}

impl EmulatedG27DeviceTypeId {
    /// Bit layout from cpp:34..42:
    ///
    /// ```text
    ///   bits 52..=61  num_buttons (10 bits)
    ///   bits 42..=51  num_hats    (10 bits)
    ///   bits 32..=41  num_axes    (10 bits)
    ///   bits 16..=31  vendor_id
    ///   bits  0..=15  product_id
    /// ```
    #[must_use]
    pub const fn as_u64(&self) -> u64 {
        let mut value = self.product_id;
        value |= self.vendor_id << 16;
        value |= (self.num_axes & ((1 << 10) - 1)) << 32;
        value |= (self.num_hats & ((1 << 10) - 1)) << 42;
        value |= (self.num_buttons & ((1 << 10) - 1)) << 52;
        value
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sdl_mapping_type_discriminants() {
        assert_eq!(SdlMappingType::Button as u32, 0);
        assert_eq!(SdlMappingType::Hat as u32, 1);
        assert_eq!(SdlMappingType::Axis as u32, 2);
    }

    #[test]
    fn sdl_mapping_type_names() {
        assert_eq!(SdlMappingType::Button.name(), "button");
        assert_eq!(SdlMappingType::Hat.name(), "hat");
        assert_eq!(SdlMappingType::Axis.name(), "axis");
    }

    #[test]
    fn hat_component_discriminants() {
        assert_eq!(HatComponent::None as u32, 0);
        assert_eq!(HatComponent::Up as u32, 1);
        assert_eq!(HatComponent::Right as u32, 4);
    }

    #[test]
    fn hat_component_names_with_empty_none() {
        assert_eq!(HatComponent::None.name(), "", "none renders empty per cpp");
        assert_eq!(HatComponent::Up.name(), "up");
        assert_eq!(HatComponent::Down.name(), "down");
        assert_eq!(HatComponent::Left.name(), "left");
        assert_eq!(HatComponent::Right.name(), "right");
    }

    #[test]
    fn device_type_id_as_u64_basic() {
        let id = EmulatedG27DeviceTypeId {
            product_id: 0x1234,
            vendor_id: 0x046D, // Logitech
            num_axes: 6,
            num_hats: 1,
            num_buttons: 23,
        };
        let packed = id.as_u64();
        // Extract via shifts.
        assert_eq!(packed & 0xFFFF, 0x1234);
        assert_eq!((packed >> 16) & 0xFFFF, 0x046D);
        assert_eq!((packed >> 32) & 0x3FF, 6);
        assert_eq!((packed >> 42) & 0x3FF, 1);
        assert_eq!((packed >> 52) & 0x3FF, 23);
    }

    #[test]
    fn device_type_id_counts_are_10_bit_masked() {
        let id = EmulatedG27DeviceTypeId {
            product_id: 0,
            vendor_id: 0,
            // 0x400 exceeds 10 bits — must be masked to 0.
            num_axes: 0x400,
            num_hats: 0x3FF, // max valid (10 bits all 1).
            num_buttons: 0x7FF, // exceeds 10 bits → masked to 0x3FF.
        };
        let packed = id.as_u64();
        assert_eq!((packed >> 32) & 0x3FF, 0);
        assert_eq!((packed >> 42) & 0x3FF, 0x3FF);
        assert_eq!((packed >> 52) & 0x3FF, 0x3FF);
    }

    #[test]
    fn device_type_id_zero_everywhere() {
        let id = EmulatedG27DeviceTypeId::default();
        assert_eq!(id.as_u64(), 0);
    }

    #[test]
    fn device_type_id_vendor_product_fit_in_16_bits_if_valid() {
        // Logitech G27 canonical: vendor=0x046D, product=0xC29B.
        let id = EmulatedG27DeviceTypeId {
            product_id: 0xC29B,
            vendor_id: 0x046D,
            num_axes: 4,
            num_hats: 1,
            num_buttons: 22,
        };
        let packed = id.as_u64();
        assert_eq!(packed & 0xFFFF, 0xC29B);
        assert_eq!((packed >> 16) & 0xFFFF, 0x046D);
    }
}
