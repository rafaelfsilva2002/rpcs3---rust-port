//! `rpcs3-lv2-raw-spu` — `sys_raw_spu_*` syscalls (standalone SPUs).
//!
//! Ports the raw-SPU subset of `rpcs3/Emu/Cell/lv2/sys_spu.cpp`.
//! Raw SPUs differ from SPURS-managed SPUs in two ways:
//!
//! * **No thread group**: the caller owns the SPU directly.
//! * **Memory-mapped registers**: the LS + problem-state registers are
//!   mapped at a fixed host address (`RAW_SPU_BASE_ADDR`), making
//!   each raw SPU exposed as a simple device.
//!
//! Most games use SPURS instead, but raw SPUs are the escape hatch for
//! latency-sensitive workloads (audio decoders, graphics helpers).
//!
//! ## Syscalls covered
//!
//! | LV2 syscall                          | Rust wrapper                       |
//! |--------------------------------------|------------------------------------|
//! | `sys_raw_spu_create`                 | [`sys_raw_spu_create`]             |
//! | `sys_raw_spu_destroy`                | [`sys_raw_spu_destroy`]            |
//! | `sys_raw_spu_create_interrupt_tag`   | [`sys_raw_spu_create_interrupt_tag`]|
//! | `sys_raw_spu_set_int_mask`           | [`sys_raw_spu_set_int_mask`]       |
//! | `sys_raw_spu_get_int_mask`           | [`sys_raw_spu_get_int_mask`]       |
//! | `sys_raw_spu_set_int_stat`           | [`sys_raw_spu_set_int_stat`]       |
//! | `sys_raw_spu_get_int_stat`           | [`sys_raw_spu_get_int_stat`]       |
//! | `sys_raw_spu_set_spu_cfg`            | [`sys_raw_spu_set_spu_cfg`]        |
//! | `sys_raw_spu_get_spu_cfg`            | [`sys_raw_spu_get_spu_cfg`]        |
//!
//! ## Frozen constants
//!
//! * `MAX_RAW_SPU = 5` — five slots, byte-exact with
//!   `spu_thread::g_raw_spu_id[5]` in C++.
//! * `RAW_SPU_BASE_ADDR = 0xE000_0000`.
//! * `RAW_SPU_OFFSET = 0x10_0000` (1 MB per slot).
//! * Class IDs: 0 / 1 / 2 (three interrupt class tags per SPU).

use rpcs3_emu_types::CellError;

// =====================================================================
// Frozen constants
// =====================================================================

pub const MAX_RAW_SPU: usize = 5;
pub const RAW_SPU_BASE_ADDR: u32 = 0xE000_0000;
/// Per-SPU address stride. Each slot occupies 1 MB of guest VA.
pub const RAW_SPU_OFFSET: u32 = 0x0010_0000;

/// LS lives at `base + LS_OFFSET`.
pub const LS_OFFSET: u32 = 0x0000;
/// Problem-state registers at `base + PROB_OFFSET`.
pub const PROB_OFFSET: u32 = 0x0004_0000;

pub const NUM_INTERRUPT_CLASSES: u32 = 3;

// =====================================================================
// Data model
// =====================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct InterruptState {
    pub mask: u64,
    pub stat: u64,
}

#[derive(Debug, Clone)]
pub struct RawSpu {
    pub slot_index: u32,
    pub id: u32,
    pub spu_cfg: u32,
    pub classes: [InterruptState; NUM_INTERRUPT_CLASSES as usize],
    pub interrupt_tag: [Option<u32>; NUM_INTERRUPT_CLASSES as usize],
}

impl RawSpu {
    /// Base VA for this raw SPU's mapped region.
    #[must_use]
    pub const fn base_addr(&self) -> u32 {
        RAW_SPU_BASE_ADDR + self.slot_index * RAW_SPU_OFFSET
    }
}

// =====================================================================
// Registry
// =====================================================================

pub trait RawSpuRegistry {
    fn raw_spu_create(&mut self) -> Result<u32, CellError>;
    fn raw_spu_destroy(&mut self, id: u32) -> Result<(), CellError>;
    fn raw_spu_get(&self, id: u32) -> Result<&RawSpu, CellError>;
    fn raw_spu_get_mut(&mut self, id: u32) -> Result<&mut RawSpu, CellError>;
}

fn check_class(class_id: u32) -> Result<(), CellError> {
    if class_id < NUM_INTERRUPT_CLASSES {
        Ok(())
    } else {
        Err(CellError::EINVAL)
    }
}

// =====================================================================
// Syscalls
// =====================================================================

#[must_use]
pub fn sys_raw_spu_create<R: RawSpuRegistry + ?Sized>(
    reg: &mut R,
) -> Result<u32, CellError> {
    reg.raw_spu_create()
}

#[must_use]
pub fn sys_raw_spu_destroy<R: RawSpuRegistry + ?Sized>(
    reg: &mut R,
    id: u32,
) -> Result<(), CellError> {
    reg.raw_spu_destroy(id)
}

#[must_use]
pub fn sys_raw_spu_create_interrupt_tag<R: RawSpuRegistry + ?Sized>(
    reg: &mut R,
    id: u32,
    class_id: u32,
    _hwthread: u32,
) -> Result<u32, CellError> {
    check_class(class_id)?;
    let spu = reg.raw_spu_get_mut(id)?;
    if spu.interrupt_tag[class_id as usize].is_some() {
        return Err(CellError::EAGAIN);
    }
    // Tag id scheme: MSB=slot index, low 2 bits = class. Fits in 32b.
    let tag = 0xA000_0000u32 | (spu.slot_index << 8) | class_id;
    spu.interrupt_tag[class_id as usize] = Some(tag);
    Ok(tag)
}

#[must_use]
pub fn sys_raw_spu_set_int_mask<R: RawSpuRegistry + ?Sized>(
    reg: &mut R,
    id: u32,
    class_id: u32,
    mask: u64,
) -> Result<(), CellError> {
    check_class(class_id)?;
    reg.raw_spu_get_mut(id)?.classes[class_id as usize].mask = mask;
    Ok(())
}

#[must_use]
pub fn sys_raw_spu_get_int_mask<R: RawSpuRegistry + ?Sized>(
    reg: &R,
    id: u32,
    class_id: u32,
) -> Result<u64, CellError> {
    check_class(class_id)?;
    Ok(reg.raw_spu_get(id)?.classes[class_id as usize].mask)
}

#[must_use]
pub fn sys_raw_spu_set_int_stat<R: RawSpuRegistry + ?Sized>(
    reg: &mut R,
    id: u32,
    class_id: u32,
    stat: u64,
) -> Result<(), CellError> {
    check_class(class_id)?;
    // "set_int_stat" in LV2 actually **clears** the bits specified by
    // `stat` (write-1-to-clear semantics, matching real hardware).
    let spu = reg.raw_spu_get_mut(id)?;
    spu.classes[class_id as usize].stat &= !stat;
    Ok(())
}

#[must_use]
pub fn sys_raw_spu_get_int_stat<R: RawSpuRegistry + ?Sized>(
    reg: &R,
    id: u32,
    class_id: u32,
) -> Result<u64, CellError> {
    check_class(class_id)?;
    Ok(reg.raw_spu_get(id)?.classes[class_id as usize].stat)
}

#[must_use]
pub fn sys_raw_spu_set_spu_cfg<R: RawSpuRegistry + ?Sized>(
    reg: &mut R,
    id: u32,
    value: u32,
) -> Result<(), CellError> {
    reg.raw_spu_get_mut(id)?.spu_cfg = value;
    Ok(())
}

#[must_use]
pub fn sys_raw_spu_get_spu_cfg<R: RawSpuRegistry + ?Sized>(
    reg: &R,
    id: u32,
) -> Result<u32, CellError> {
    Ok(reg.raw_spu_get(id)?.spu_cfg)
}

// =====================================================================
// Reference registry
// =====================================================================

#[derive(Debug, Default)]
pub struct TestRawSpuRegistry {
    next_id: u32,
    slots: [Option<u32>; MAX_RAW_SPU],
    spus: std::collections::BTreeMap<u32, RawSpu>,
}

impl TestRawSpuRegistry {
    fn alloc_id(&mut self) -> u32 {
        self.next_id += 1;
        // id_base — arbitrary but consistent.
        0xAA00_0000 | self.next_id
    }

    /// Test helper: test how many SPUs are allocated.
    #[must_use]
    pub fn active_count(&self) -> usize {
        self.slots.iter().filter(|s| s.is_some()).count()
    }

    /// Test helper: fire an interrupt (set bit in stat).
    pub fn fire_interrupt(&mut self, id: u32, class_id: u32, bit: u64) -> Result<(), CellError> {
        check_class(class_id)?;
        let spu = self.raw_spu_get_mut(id)?;
        spu.classes[class_id as usize].stat |= bit;
        Ok(())
    }
}

impl RawSpuRegistry for TestRawSpuRegistry {
    fn raw_spu_create(&mut self) -> Result<u32, CellError> {
        let slot_idx = self
            .slots
            .iter()
            .position(|s| s.is_none())
            .ok_or(CellError::EAGAIN)?;
        let id = self.alloc_id();
        self.slots[slot_idx] = Some(id);
        self.spus.insert(
            id,
            RawSpu {
                slot_index: slot_idx as u32,
                id,
                spu_cfg: 0,
                classes: [InterruptState::default(); NUM_INTERRUPT_CLASSES as usize],
                interrupt_tag: [None; NUM_INTERRUPT_CLASSES as usize],
            },
        );
        Ok(id)
    }

    fn raw_spu_destroy(&mut self, id: u32) -> Result<(), CellError> {
        let spu = self.spus.remove(&id).ok_or(CellError::ESRCH)?;
        self.slots[spu.slot_index as usize] = None;
        Ok(())
    }

    fn raw_spu_get(&self, id: u32) -> Result<&RawSpu, CellError> {
        self.spus.get(&id).ok_or(CellError::ESRCH)
    }

    fn raw_spu_get_mut(&mut self, id: u32) -> Result<&mut RawSpu, CellError> {
        self.spus.get_mut(&id).ok_or(CellError::ESRCH)
    }
}

// =====================================================================
// Tests
// =====================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // --- constants ------------------------------------------------

    #[test]
    fn layout_constants_match_cpp() {
        assert_eq!(MAX_RAW_SPU, 5);
        assert_eq!(RAW_SPU_BASE_ADDR, 0xE000_0000);
        assert_eq!(RAW_SPU_OFFSET, 0x10_0000);
        assert_eq!(NUM_INTERRUPT_CLASSES, 3);
    }

    // --- create / destroy -----------------------------------------

    #[test]
    fn create_allocates_first_free_slot_with_base_address_derived() {
        let mut reg = TestRawSpuRegistry::default();
        let id = sys_raw_spu_create(&mut reg).unwrap();
        let spu = reg.raw_spu_get(id).unwrap();
        assert_eq!(spu.slot_index, 0);
        assert_eq!(spu.base_addr(), RAW_SPU_BASE_ADDR);

        let id2 = sys_raw_spu_create(&mut reg).unwrap();
        let spu2 = reg.raw_spu_get(id2).unwrap();
        assert_eq!(spu2.slot_index, 1);
        assert_eq!(spu2.base_addr(), RAW_SPU_BASE_ADDR + RAW_SPU_OFFSET);
    }

    #[test]
    fn create_fills_all_five_slots_then_eagain() {
        let mut reg = TestRawSpuRegistry::default();
        for _ in 0..MAX_RAW_SPU {
            sys_raw_spu_create(&mut reg).unwrap();
        }
        assert_eq!(reg.active_count(), 5);
        assert_eq!(sys_raw_spu_create(&mut reg).unwrap_err(), CellError::EAGAIN);
    }

    #[test]
    fn destroy_frees_slot_for_reuse() {
        let mut reg = TestRawSpuRegistry::default();
        let id = sys_raw_spu_create(&mut reg).unwrap();
        sys_raw_spu_destroy(&mut reg, id).unwrap();
        let id2 = sys_raw_spu_create(&mut reg).unwrap();
        assert_eq!(reg.raw_spu_get(id2).unwrap().slot_index, 0, "slot reused");
    }

    #[test]
    fn destroy_unknown_is_esrch() {
        let mut reg = TestRawSpuRegistry::default();
        assert_eq!(
            sys_raw_spu_destroy(&mut reg, 0xDEAD_BEEF).unwrap_err(),
            CellError::ESRCH,
        );
    }

    // --- interrupt tags -------------------------------------------

    #[test]
    fn create_interrupt_tag_assigns_unique_id_per_class() {
        let mut reg = TestRawSpuRegistry::default();
        let id = sys_raw_spu_create(&mut reg).unwrap();
        let tag0 = sys_raw_spu_create_interrupt_tag(&mut reg, id, 0, 0).unwrap();
        let tag1 = sys_raw_spu_create_interrupt_tag(&mut reg, id, 1, 0).unwrap();
        let tag2 = sys_raw_spu_create_interrupt_tag(&mut reg, id, 2, 0).unwrap();
        assert_ne!(tag0, tag1);
        assert_ne!(tag1, tag2);
    }

    #[test]
    fn create_interrupt_tag_twice_same_class_is_eagain() {
        let mut reg = TestRawSpuRegistry::default();
        let id = sys_raw_spu_create(&mut reg).unwrap();
        sys_raw_spu_create_interrupt_tag(&mut reg, id, 0, 0).unwrap();
        assert_eq!(
            sys_raw_spu_create_interrupt_tag(&mut reg, id, 0, 0).unwrap_err(),
            CellError::EAGAIN,
        );
    }

    #[test]
    fn bad_class_id_is_einval() {
        let mut reg = TestRawSpuRegistry::default();
        let id = sys_raw_spu_create(&mut reg).unwrap();
        assert_eq!(
            sys_raw_spu_create_interrupt_tag(&mut reg, id, 3, 0).unwrap_err(),
            CellError::EINVAL,
        );
        assert_eq!(
            sys_raw_spu_set_int_mask(&mut reg, id, 99, 0).unwrap_err(),
            CellError::EINVAL,
        );
    }

    // --- int mask / stat ------------------------------------------

    #[test]
    fn int_mask_round_trips_per_class() {
        let mut reg = TestRawSpuRegistry::default();
        let id = sys_raw_spu_create(&mut reg).unwrap();
        sys_raw_spu_set_int_mask(&mut reg, id, 1, 0xCAFE_BEEF_DEAD_BABE).unwrap();
        assert_eq!(
            sys_raw_spu_get_int_mask(&reg, id, 1).unwrap(),
            0xCAFE_BEEF_DEAD_BABE,
        );
        // Other classes unaffected.
        assert_eq!(sys_raw_spu_get_int_mask(&reg, id, 0).unwrap(), 0);
        assert_eq!(sys_raw_spu_get_int_mask(&reg, id, 2).unwrap(), 0);
    }

    #[test]
    fn int_stat_read_clears_via_set_int_stat() {
        let mut reg = TestRawSpuRegistry::default();
        let id = sys_raw_spu_create(&mut reg).unwrap();
        reg.fire_interrupt(id, 0, 0xF0F0).unwrap();
        assert_eq!(sys_raw_spu_get_int_stat(&reg, id, 0).unwrap(), 0xF0F0);
        // Clear the high nibble bits.
        sys_raw_spu_set_int_stat(&mut reg, id, 0, 0xF000).unwrap();
        assert_eq!(sys_raw_spu_get_int_stat(&reg, id, 0).unwrap(), 0xF0);
    }

    // --- cfg ------------------------------------------------------

    #[test]
    fn spu_cfg_round_trips() {
        let mut reg = TestRawSpuRegistry::default();
        let id = sys_raw_spu_create(&mut reg).unwrap();
        assert_eq!(sys_raw_spu_get_spu_cfg(&reg, id).unwrap(), 0);
        sys_raw_spu_set_spu_cfg(&mut reg, id, 0x1234_5678).unwrap();
        assert_eq!(sys_raw_spu_get_spu_cfg(&reg, id).unwrap(), 0x1234_5678);
    }

    #[test]
    fn cfg_ops_on_unknown_id_are_esrch() {
        let mut reg = TestRawSpuRegistry::default();
        assert_eq!(
            sys_raw_spu_set_spu_cfg(&mut reg, 42, 1).unwrap_err(),
            CellError::ESRCH,
        );
        assert_eq!(
            sys_raw_spu_get_spu_cfg(&reg, 42).unwrap_err(),
            CellError::ESRCH,
        );
    }

    // --- base address layout --------------------------------------

    #[test]
    fn slot_addresses_are_1MB_apart() {
        let mut reg = TestRawSpuRegistry::default();
        let mut addrs = Vec::new();
        for _ in 0..MAX_RAW_SPU {
            let id = sys_raw_spu_create(&mut reg).unwrap();
            addrs.push(reg.raw_spu_get(id).unwrap().base_addr());
        }
        for w in addrs.windows(2) {
            assert_eq!(w[1] - w[0], 0x10_0000);
        }
    }
}
