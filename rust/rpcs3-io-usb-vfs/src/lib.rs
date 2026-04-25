//! `rpcs3-io-usb-vfs` — Rust port of `rpcs3/Emu/Io/usb_vfs.cpp`.
//!
//! Emulates an "SMI Corporation / USB DISK" mass-storage stick with
//! user-supplied VID/PID so PS3 games that look for a specific device
//! pass their checks. The host-side filesystem is plugged in separately
//! (the RPCS3 class delegates to VFS); what's portable and testable is
//! the fixed USB descriptor layout.
//!
//! Frozen:
//!
//! - Constant descriptor bytes (cpp:11..57). `bcdDevice` is intentionally
//!   set equal to `pid` (cpp:20 — a minor quirk worth preserving).
//! - Mass-storage class triple: `bInterfaceClass=0x08`, `SubClass=0x06`,
//!   `Protocol=0x50` ("bulk-only transport").
//! - Bulk IN/OUT endpoints at addresses 0x81/0x02 with 512-byte packets
//!   and a 0xFF interval.
//! - String descriptors: `["SMI Corporation", "USB DISK", <serial>]`.

pub const USB_BCD_USB: u16 = 0x0200;
pub const USB_MAX_PACKET_SIZE_0: u8 = 0x40;
pub const USB_CONFIG_W_TOTAL_LENGTH: u16 = 0x0020;
pub const USB_CONFIG_NUM_INTERFACES: u8 = 0x01;
pub const USB_CONFIG_VALUE: u8 = 0x01;
pub const USB_CONFIG_BM_ATTRIBUTES: u8 = 0x80;
pub const USB_CONFIG_MAX_POWER: u8 = 0x32;

/// Mass-storage class/subclass/protocol (cpp:40..42).
pub const USB_MS_INTERFACE_CLASS: u8 = 0x08;
pub const USB_MS_SUBCLASS_SCSI: u8 = 0x06;
pub const USB_MS_PROTOCOL_BULK_ONLY: u8 = 0x50;

pub const USB_ENDPOINT_IN_ADDRESS: u8 = 0x81;
pub const USB_ENDPOINT_OUT_ADDRESS: u8 = 0x02;
pub const USB_ENDPOINT_BM_ATTRIBUTES_BULK: u8 = 0x02;
pub const USB_ENDPOINT_W_MAX_PACKET_SIZE: u16 = 0x0200;
pub const USB_ENDPOINT_B_INTERVAL: u8 = 0xFF;

/// Descriptor strings (cpp:59). Fixed manufacturer + product; serial
/// comes from the user-supplied `device_info`.
pub const MANUFACTURER_STRING: &str = "SMI Corporation";
pub const PRODUCT_STRING: &str = "USB DISK";

/// Compute the `bcdDevice` field from a PID (cpp:20 quirk — RPCS3 writes
/// `pid` into the `bcdDevice` slot rather than a firmware version).
#[must_use]
pub const fn bcd_device_from_pid(pid: u16) -> u16 {
    pid
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constant_descriptor_bytes_match_cpp() {
        assert_eq!(USB_BCD_USB, 0x0200);
        assert_eq!(USB_MAX_PACKET_SIZE_0, 0x40);
        assert_eq!(USB_CONFIG_W_TOTAL_LENGTH, 0x0020);
        assert_eq!(USB_CONFIG_MAX_POWER, 0x32);
    }

    #[test]
    fn mass_storage_class_triple() {
        assert_eq!(USB_MS_INTERFACE_CLASS, 0x08);
        assert_eq!(USB_MS_SUBCLASS_SCSI, 0x06);
        assert_eq!(USB_MS_PROTOCOL_BULK_ONLY, 0x50);
    }

    #[test]
    fn endpoints_bulk_0x81_and_0x02_with_512b_packets() {
        assert_eq!(USB_ENDPOINT_IN_ADDRESS, 0x81);
        assert_eq!(USB_ENDPOINT_OUT_ADDRESS, 0x02);
        assert_eq!(USB_ENDPOINT_BM_ATTRIBUTES_BULK, 0x02);
        assert_eq!(USB_ENDPOINT_W_MAX_PACKET_SIZE, 0x0200);
        assert_eq!(USB_ENDPOINT_B_INTERVAL, 0xFF);
    }

    #[test]
    fn descriptor_strings_frozen() {
        assert_eq!(MANUFACTURER_STRING, "SMI Corporation");
        assert_eq!(PRODUCT_STRING, "USB DISK");
    }

    #[test]
    fn bcd_device_mirrors_pid() {
        // Quirk: RPCS3 stores PID in bcdDevice.
        assert_eq!(bcd_device_from_pid(0x1234), 0x1234);
        assert_eq!(bcd_device_from_pid(0), 0);
        assert_eq!(bcd_device_from_pid(0xFFFF), 0xFFFF);
    }
}
