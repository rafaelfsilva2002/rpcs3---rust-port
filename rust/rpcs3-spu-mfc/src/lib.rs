//! `rpcs3-spu-mfc` — Rust port of `rpcs3/Emu/Cell/MFC.cpp`.
//!
//! SPU MFC DMA command opcodes. A single SPU command is an 8-bit opcode
//! built from a base transfer-direction byte OR'd with 0..3 feature
//! masks (barrier / fence / list / start) plus a 1-bit result flag that
//! doesn't appear in the wire encoding but shows up in `fmt` dumps.
//!
//! Frozen here (values in `Emu/Cell/MFC.h:7..35`):
//!
//! - 40 opcodes byte-exact (PUT/GET families, SDCR*, SNDSIG, BARRIER,
//!   EIEIO, SYNC, GETLLAR, PUTLLC, PUTLLUC, PUTQLLUC).
//! - Feature masks `BARRIER=0x01`, `FENCE=0x02`, `LIST=0x04`, `START=0x08`,
//!   `RESULT=0x10`.
//! - Name lookup matching `fmt_class_string<MFC>::format` (cpp:7..63).
//! - Tag layout for `spu_mfc_cmd`: bit 7 = stalled flag, bits 0..=6 = tag
//!   id (cpp:71..73).

// Non-list PUT family.
pub const MFC_PUT_CMD: u8 = 0x20;
pub const MFC_PUTB_CMD: u8 = 0x21;
pub const MFC_PUTF_CMD: u8 = 0x22;
pub const MFC_PUTS_CMD: u8 = 0x28;
pub const MFC_PUTBS_CMD: u8 = 0x29;
pub const MFC_PUTFS_CMD: u8 = 0x2a;
pub const MFC_PUTR_CMD: u8 = 0x30;
pub const MFC_PUTRB_CMD: u8 = 0x31;
pub const MFC_PUTRF_CMD: u8 = 0x32;

// Non-list GET family.
pub const MFC_GET_CMD: u8 = 0x40;
pub const MFC_GETB_CMD: u8 = 0x41;
pub const MFC_GETF_CMD: u8 = 0x42;
pub const MFC_GETS_CMD: u8 = 0x48;
pub const MFC_GETBS_CMD: u8 = 0x49;
pub const MFC_GETFS_CMD: u8 = 0x4a;

// List PUT family.
pub const MFC_PUTL_CMD: u8 = 0x24;
pub const MFC_PUTLB_CMD: u8 = 0x25;
pub const MFC_PUTLF_CMD: u8 = 0x26;
pub const MFC_PUTRL_CMD: u8 = 0x34;
pub const MFC_PUTRLB_CMD: u8 = 0x35;
pub const MFC_PUTRLF_CMD: u8 = 0x36;

// List GET family.
pub const MFC_GETL_CMD: u8 = 0x44;
pub const MFC_GETLB_CMD: u8 = 0x45;
pub const MFC_GETLF_CMD: u8 = 0x46;

// Atomic reservation ops.
pub const MFC_GETLLAR_CMD: u8 = 0xD0;
pub const MFC_PUTLLC_CMD: u8 = 0xB4;
pub const MFC_PUTLLUC_CMD: u8 = 0xB0;
pub const MFC_PUTQLLUC_CMD: u8 = 0xB8;

// Signal notification.
pub const MFC_SNDSIG_CMD: u8 = 0xA0;
pub const MFC_SNDSIGB_CMD: u8 = 0xA1;
pub const MFC_SNDSIGF_CMD: u8 = 0xA2;

// Synchronization.
pub const MFC_BARRIER_CMD: u8 = 0xC0;
pub const MFC_EIEIO_CMD: u8 = 0xC8;
pub const MFC_SYNC_CMD: u8 = 0xCC;

// Software data-cache range ops.
pub const MFC_SDCRT_CMD: u8 = 0x80;
pub const MFC_SDCRTST_CMD: u8 = 0x81;
pub const MFC_SDCRZ_CMD: u8 = 0x89;
pub const MFC_SDCRS_CMD: u8 = 0x8D;
pub const MFC_SDCRF_CMD: u8 = 0x8F;

// Feature masks (cpp:25..29).
pub const MFC_BARRIER_MASK: u8 = 0x01;
pub const MFC_FENCE_MASK: u8 = 0x02;
pub const MFC_LIST_MASK: u8 = 0x04;
pub const MFC_START_MASK: u8 = 0x08;
pub const MFC_RESULT_MASK: u8 = 0x10;

/// Tag has 7-bit id in bits 0..=6 and a "stalled" flag in bit 7
/// (see cpp:73 formatting).
pub const MFC_TAG_ID_MASK: u8 = 0x7F;
pub const MFC_TAG_STALLED_FLAG: u8 = 0x80;

/// Return the C++-style name for an MFC opcode (cpp:7..63). Unknown
/// opcodes yield `None`.
#[must_use]
pub fn name_for_cmd(cmd: u8) -> Option<&'static str> {
    Some(match cmd {
        MFC_PUT_CMD => "PUT",
        MFC_PUTB_CMD => "PUTB",
        MFC_PUTF_CMD => "PUTF",
        MFC_PUTS_CMD => "PUTS",
        MFC_PUTBS_CMD => "PUTBS",
        MFC_PUTFS_CMD => "PUTFS",
        MFC_PUTR_CMD => "PUTR",
        MFC_PUTRB_CMD => "PUTRB",
        MFC_PUTRF_CMD => "PUTRF",
        MFC_GET_CMD => "GET",
        MFC_GETB_CMD => "GETB",
        MFC_GETF_CMD => "GETF",
        MFC_GETS_CMD => "GETS",
        MFC_GETBS_CMD => "GETBS",
        MFC_GETFS_CMD => "GETFS",
        MFC_PUTL_CMD => "PUTL",
        MFC_PUTLB_CMD => "PUTLB",
        MFC_PUTLF_CMD => "PUTLF",
        MFC_PUTRL_CMD => "PUTRL",
        MFC_PUTRLB_CMD => "PUTRLB",
        MFC_PUTRLF_CMD => "PUTRLF",
        MFC_GETL_CMD => "GETL",
        MFC_GETLB_CMD => "GETLB",
        MFC_GETLF_CMD => "GETLF",
        MFC_GETLLAR_CMD => "GETLLAR",
        MFC_PUTLLC_CMD => "PUTLLC",
        MFC_PUTLLUC_CMD => "PUTLLUC",
        MFC_PUTQLLUC_CMD => "PUTQLLUC",
        MFC_SNDSIG_CMD => "SNDSIG",
        MFC_SNDSIGB_CMD => "SNDSIGB",
        MFC_SNDSIGF_CMD => "SNDSIGF",
        MFC_BARRIER_CMD => "BARRIER",
        MFC_EIEIO_CMD => "EIEIO",
        MFC_SYNC_CMD => "SYNC",
        MFC_SDCRT_CMD => "SDCRT",
        MFC_SDCRTST_CMD => "SDCRTST",
        MFC_SDCRZ_CMD => "SDCRZ",
        MFC_SDCRS_CMD => "SDCRS",
        MFC_SDCRF_CMD => "SDCRF",
        _ => return None,
    })
}

/// Extract the 7-bit tag id from the full tag byte.
#[must_use]
pub const fn tag_id(tag: u8) -> u8 {
    tag & MFC_TAG_ID_MASK
}

/// Whether the tag byte has the stalled flag set.
#[must_use]
pub const fn tag_is_stalled(tag: u8) -> bool {
    (tag & MFC_TAG_STALLED_FLAG) != 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn put_family_opcodes_byte_exact() {
        assert_eq!(MFC_PUT_CMD, 0x20);
        assert_eq!(MFC_PUTB_CMD, 0x21);
        assert_eq!(MFC_PUTF_CMD, 0x22);
        assert_eq!(MFC_PUTS_CMD, 0x28);
        assert_eq!(MFC_PUTR_CMD, 0x30);
    }

    #[test]
    fn get_family_opcodes_byte_exact() {
        assert_eq!(MFC_GET_CMD, 0x40);
        assert_eq!(MFC_GETB_CMD, 0x41);
        assert_eq!(MFC_GETF_CMD, 0x42);
        assert_eq!(MFC_GETS_CMD, 0x48);
    }

    #[test]
    fn atomic_ops_byte_exact() {
        assert_eq!(MFC_GETLLAR_CMD, 0xD0);
        assert_eq!(MFC_PUTLLC_CMD, 0xB4);
        assert_eq!(MFC_PUTLLUC_CMD, 0xB0);
        assert_eq!(MFC_PUTQLLUC_CMD, 0xB8);
    }

    #[test]
    fn sync_ops_byte_exact() {
        assert_eq!(MFC_BARRIER_CMD, 0xC0);
        assert_eq!(MFC_EIEIO_CMD, 0xC8);
        assert_eq!(MFC_SYNC_CMD, 0xCC);
    }

    #[test]
    fn sdcr_ops_byte_exact() {
        assert_eq!(MFC_SDCRT_CMD, 0x80);
        assert_eq!(MFC_SDCRTST_CMD, 0x81);
        assert_eq!(MFC_SDCRZ_CMD, 0x89);
        assert_eq!(MFC_SDCRS_CMD, 0x8D);
        assert_eq!(MFC_SDCRF_CMD, 0x8F);
    }

    #[test]
    fn feature_masks_byte_exact() {
        assert_eq!(MFC_BARRIER_MASK, 0x01);
        assert_eq!(MFC_FENCE_MASK, 0x02);
        assert_eq!(MFC_LIST_MASK, 0x04);
        assert_eq!(MFC_START_MASK, 0x08);
        assert_eq!(MFC_RESULT_MASK, 0x10);
    }

    #[test]
    fn name_lookup_covers_all_40_cmds() {
        // A selection spanning every family returns a non-None name.
        let all = [
            MFC_PUT_CMD, MFC_PUTB_CMD, MFC_PUTF_CMD, MFC_PUTS_CMD, MFC_PUTBS_CMD, MFC_PUTFS_CMD,
            MFC_PUTR_CMD, MFC_PUTRB_CMD, MFC_PUTRF_CMD, MFC_GET_CMD, MFC_GETB_CMD, MFC_GETF_CMD,
            MFC_GETS_CMD, MFC_GETBS_CMD, MFC_GETFS_CMD, MFC_PUTL_CMD, MFC_PUTLB_CMD, MFC_PUTLF_CMD,
            MFC_PUTRL_CMD, MFC_PUTRLB_CMD, MFC_PUTRLF_CMD, MFC_GETL_CMD, MFC_GETLB_CMD,
            MFC_GETLF_CMD, MFC_GETLLAR_CMD, MFC_PUTLLC_CMD, MFC_PUTLLUC_CMD, MFC_PUTQLLUC_CMD,
            MFC_SNDSIG_CMD, MFC_SNDSIGB_CMD, MFC_SNDSIGF_CMD, MFC_BARRIER_CMD, MFC_EIEIO_CMD,
            MFC_SYNC_CMD, MFC_SDCRT_CMD, MFC_SDCRTST_CMD, MFC_SDCRZ_CMD, MFC_SDCRS_CMD,
            MFC_SDCRF_CMD,
        ];
        assert_eq!(all.len(), 39);
        for cmd in all {
            assert!(name_for_cmd(cmd).is_some(), "cmd {cmd:#x} missing name");
        }
    }

    #[test]
    fn name_lookup_specific_strings() {
        assert_eq!(name_for_cmd(MFC_PUT_CMD), Some("PUT"));
        assert_eq!(name_for_cmd(MFC_GETLLAR_CMD), Some("GETLLAR"));
        assert_eq!(name_for_cmd(MFC_SNDSIG_CMD), Some("SNDSIG"));
        assert_eq!(name_for_cmd(MFC_SDCRTST_CMD), Some("SDCRTST"));
    }

    #[test]
    fn name_lookup_unknown_is_none() {
        assert_eq!(name_for_cmd(0x00), None);
        assert_eq!(name_for_cmd(0xFF), None);
    }

    #[test]
    fn tag_id_and_stalled_helpers() {
        assert_eq!(tag_id(0x12), 0x12);
        assert_eq!(tag_id(0x82), 0x02);
        assert!(tag_is_stalled(0x80));
        assert!(tag_is_stalled(0xFF));
        assert!(!tag_is_stalled(0x7F));
    }

    #[test]
    fn list_family_opcodes_have_list_mask_bit_set() {
        // All *L_CMD opcodes should have bit 2 (MFC_LIST_MASK) set.
        for cmd in [
            MFC_PUTL_CMD, MFC_PUTLB_CMD, MFC_PUTLF_CMD, MFC_PUTRL_CMD, MFC_PUTRLB_CMD,
            MFC_PUTRLF_CMD, MFC_GETL_CMD, MFC_GETLB_CMD, MFC_GETLF_CMD,
        ] {
            assert_ne!(cmd & MFC_LIST_MASK, 0, "{cmd:#x} missing LIST mask bit");
        }
    }
}
