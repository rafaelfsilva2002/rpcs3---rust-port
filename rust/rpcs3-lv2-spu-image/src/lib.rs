//! `rpcs3-lv2-spu-image` — SPU ELF image loader + `sys_spu_image_*`
//! syscalls.
//!
//! Ports the image-loading subset of `rpcs3/Emu/Cell/lv2/sys_spu.cpp`
//! / `sys_spu.h`. An SPU image is a parsed program laid out as a
//! vector of [`SpuSegment`] records (COPY / FILL / INFO), ready to
//! be deployed into a 256 KB local store.
//!
//! ## Syscalls covered
//!
//! | LV2 syscall                        | Rust wrapper                   |
//! |------------------------------------|--------------------------------|
//! | `sys_spu_image_open`               | [`sys_spu_image_open`]         |
//! | `_sys_spu_image_import`            | [`sys_spu_image_import`]       |
//! | `_sys_spu_image_close`             | [`sys_spu_image_close`]        |
//! | `_sys_spu_image_get_information`   | [`sys_spu_image_get_info`]     |
//! | `_sys_spu_image_get_segments`      | [`sys_spu_image_get_segments`] |
//!
//! ## Frozen constants (from `sys_spu.h`)
//!
//! | Const                           | Value |
//! |---------------------------------|------:|
//! | `SYS_SPU_SEGMENT_TYPE_COPY`     | 1     |
//! | `SYS_SPU_SEGMENT_TYPE_FILL`     | 2     |
//! | `SYS_SPU_SEGMENT_TYPE_INFO`     | 4     |
//! | `SYS_SPU_IMAGE_TYPE_USER`       | 0     |
//! | `SYS_SPU_IMAGE_TYPE_KERNEL`     | 1     |
//! | `SYS_SPU_IMAGE_PROTECT`         | 0     |
//! | `SYS_SPU_IMAGE_DIRECT`          | 1     |
//! | `id_base (lv2_spu_image)`       | `0x22000000` |

use rpcs3_emu_types::CellError;

// =====================================================================
// Frozen constants
// =====================================================================

pub const SPU_SEGMENT_TYPE_COPY: u32 = 1;
pub const SPU_SEGMENT_TYPE_FILL: u32 = 2;
pub const SPU_SEGMENT_TYPE_INFO: u32 = 4;

pub const SPU_IMAGE_TYPE_USER: u32 = 0;
pub const SPU_IMAGE_TYPE_KERNEL: u32 = 1;

pub const SPU_IMAGE_PROTECT: u32 = 0;
pub const SPU_IMAGE_DIRECT: u32 = 1;

pub const LS_SIZE: u32 = 256 * 1024;

/// ELF program header p_type values the loader recognises.
pub const PT_LOAD: u32 = 1;
pub const PT_NOTE: u32 = 4;

// =====================================================================
// Data shapes — all sizes frozen byte-exact
// =====================================================================

/// One 24-byte SPU segment record. Mirrors `sys_spu_segment` which
/// `CHECK_SIZE(sys_spu_segment, 0x18)` enforces in C++.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(C)]
pub struct SpuSegment {
    pub seg_type: u32,
    pub ls: u32,
    pub size: u32,
    pub pad: u32,
    pub addr: u32,
    pub addr_pad: u32,
}

const _: () = {
    assert!(core::mem::size_of::<SpuSegment>() == 0x18);
};

impl SpuSegment {
    #[must_use]
    pub const fn copy(ls: u32, size: u32, addr: u32) -> Self {
        Self { seg_type: SPU_SEGMENT_TYPE_COPY, ls, size, pad: 0, addr, addr_pad: 0 }
    }
    #[must_use]
    pub const fn fill(ls: u32, size: u32) -> Self {
        Self { seg_type: SPU_SEGMENT_TYPE_FILL, ls, size, pad: 0, addr: 0, addr_pad: 0 }
    }
    #[must_use]
    pub const fn info(size: u32, addr: u32) -> Self {
        Self { seg_type: SPU_SEGMENT_TYPE_INFO, ls: 0, size, pad: 0, addr, addr_pad: 0 }
    }
}

/// Parsed image handle — what `sys_spu_image_*` syscalls surface.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpuImage {
    pub image_type: u32,
    pub entry_point: u32,
    pub segments: Vec<SpuSegment>,
}

/// A single PPC ELF program header relevant to SPU loading. The host
/// side produces this by parsing the ELF; the loader fills the
/// `SpuSegment` list from it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SpuPhdr {
    pub p_type: u32,
    pub p_offset: u32,
    pub p_vaddr: u32,
    pub p_filesz: u32,
    pub p_memsz: u32,
}

// =====================================================================
// Fill / counting (matches the C++ `fill<WriteInfo=true>` branch)
// =====================================================================

/// Count how many [`SpuSegment`] records `phdrs` will produce.
///
/// Returns `-1` if any program header has an unrecognised `p_type`.
#[must_use]
pub fn count_segments(phdrs: &[SpuPhdr], count_info: bool) -> i32 {
    let mut n: i32 = 0;
    for p in phdrs {
        if p.p_type != PT_LOAD && p.p_type != PT_NOTE {
            return -1;
        }
        if p.p_type == PT_LOAD && p.p_filesz != p.p_memsz && p.p_filesz != 0 {
            n += 2;
        } else if p.p_type == PT_LOAD || count_info {
            n += 1;
        }
    }
    n
}

/// Populate `segs` from ELF program headers. `src` is the guest-side
/// base address where the ELF file bytes sit — COPY / INFO segments
/// reference offsets within that buffer.
///
/// Mirrors C++ `sys_spu_image::fill<WriteInfo=true>(...)`.
/// Returns the number of segments written, or a negative error:
/// `-1` = bad `p_type`, `-2` = out of capacity.
pub fn fill_segments(
    segs: &mut [SpuSegment],
    phdrs: &[SpuPhdr],
    src: u32,
    write_info: bool,
) -> i32 {
    let mut n = 0usize;
    for p in phdrs {
        match p.p_type {
            PT_LOAD => {
                if p.p_filesz != 0 {
                    if n >= segs.len() {
                        return -2;
                    }
                    segs[n] = SpuSegment::copy(
                        p.p_vaddr,
                        p.p_filesz,
                        p.p_offset.wrapping_add(src),
                    );
                    n += 1;
                }
                if p.p_memsz > p.p_filesz {
                    if n >= segs.len() {
                        return -2;
                    }
                    segs[n] = SpuSegment::fill(
                        p.p_vaddr + p.p_filesz,
                        p.p_memsz - p.p_filesz,
                    );
                    n += 1;
                }
            }
            PT_NOTE if write_info => {
                if n >= segs.len() {
                    return -2;
                }
                // C++ note: `seg->addr = static_cast<u32>(phdr.p_offset + 0x14 + src)`.
                segs[n] = SpuSegment::info(0x20, p.p_offset.wrapping_add(0x14).wrapping_add(src));
                n += 1;
            }
            PT_NOTE => {
                // WriteInfo=false drops PT_NOTE silently.
            }
            _ => return -1,
        }
    }
    n as i32
}

/// Convenience: build a complete [`SpuImage`] from ELF program headers
/// in one shot. `entry_point` is the `e_entry` from the SPU ELF header.
#[must_use]
pub fn build_image(entry_point: u32, phdrs: &[SpuPhdr], src: u32) -> Result<SpuImage, CellError> {
    let n = count_segments(phdrs, true);
    if n < 0 {
        return Err(CellError::EINVAL);
    }
    let mut segs = vec![SpuSegment::copy(0, 0, 0); n as usize];
    let written = fill_segments(&mut segs, phdrs, src, true);
    if written < 0 {
        return Err(CellError::EINVAL);
    }
    assert_eq!(written as usize, segs.len());
    Ok(SpuImage { image_type: SPU_IMAGE_TYPE_USER, entry_point, segments: segs })
}

// =====================================================================
// Deployer: write COPY/FILL segments into a 256 KB LS buffer.
// =====================================================================

/// Copy/fill a parsed image into a simulated SPU local store.
///
/// * COPY segments need their source bytes; caller supplies them via
///   `fetch(addr, size)` — typically a read from guest memory.
/// * FILL segments zero the corresponding LS range.
/// * INFO segments are metadata only (left alone by the deployer).
///
/// Returns `EINVAL` if any segment would overflow the 256 KB LS.
pub fn deploy<F>(image: &SpuImage, ls: &mut [u8], mut fetch: F) -> Result<(), CellError>
where
    F: FnMut(u32, u32) -> Option<Vec<u8>>,
{
    assert_eq!(ls.len(), LS_SIZE as usize, "local store must be 256 KB");
    for seg in &image.segments {
        match seg.seg_type {
            SPU_SEGMENT_TYPE_COPY => {
                let end = seg.ls.checked_add(seg.size).ok_or(CellError::EINVAL)?;
                if end as usize > ls.len() {
                    return Err(CellError::EINVAL);
                }
                let bytes = fetch(seg.addr, seg.size).ok_or(CellError::EFAULT)?;
                if bytes.len() != seg.size as usize {
                    return Err(CellError::EFAULT);
                }
                ls[seg.ls as usize..end as usize].copy_from_slice(&bytes);
            }
            SPU_SEGMENT_TYPE_FILL => {
                let end = seg.ls.checked_add(seg.size).ok_or(CellError::EINVAL)?;
                if end as usize > ls.len() {
                    return Err(CellError::EINVAL);
                }
                for b in &mut ls[seg.ls as usize..end as usize] {
                    *b = 0;
                }
            }
            SPU_SEGMENT_TYPE_INFO => {}
            _ => return Err(CellError::EINVAL),
        }
    }
    Ok(())
}

// =====================================================================
// Registry (sys_spu_image_* syscalls)
// =====================================================================

pub trait SpuImageRegistry {
    fn image_open(&mut self, path: &str, image: SpuImage) -> Result<u32, CellError>;
    fn image_import(&mut self, src: u32, size: u32, image: SpuImage) -> Result<u32, CellError>;
    fn image_close(&mut self, id: u32) -> Result<(), CellError>;
    fn image_get(&self, id: u32) -> Result<&SpuImage, CellError>;
}

#[must_use]
pub fn sys_spu_image_open<R: SpuImageRegistry + ?Sized>(
    reg: &mut R,
    path: &str,
    image: SpuImage,
) -> Result<u32, CellError> {
    if path.is_empty() {
        return Err(CellError::EINVAL);
    }
    reg.image_open(path, image)
}

#[must_use]
pub fn sys_spu_image_import<R: SpuImageRegistry + ?Sized>(
    reg: &mut R,
    src: u32,
    size: u32,
    image: SpuImage,
) -> Result<u32, CellError> {
    if size == 0 {
        return Err(CellError::EINVAL);
    }
    reg.image_import(src, size, image)
}

#[must_use]
pub fn sys_spu_image_close<R: SpuImageRegistry + ?Sized>(
    reg: &mut R,
    id: u32,
) -> Result<(), CellError> {
    reg.image_close(id)
}

#[must_use]
pub fn sys_spu_image_get_info<R: SpuImageRegistry + ?Sized>(
    reg: &R,
    id: u32,
) -> Result<(u32, u32), CellError> {
    let img = reg.image_get(id)?;
    Ok((img.entry_point, img.segments.len() as u32))
}

#[must_use]
pub fn sys_spu_image_get_segments<R: SpuImageRegistry + ?Sized>(
    reg: &R,
    id: u32,
    dst: &mut [SpuSegment],
) -> Result<u32, CellError> {
    let img = reg.image_get(id)?;
    let n = img.segments.len().min(dst.len());
    dst[..n].copy_from_slice(&img.segments[..n]);
    Ok(n as u32)
}

// =====================================================================
// Reference registry
// =====================================================================

#[derive(Debug, Default)]
pub struct TestSpuImageRegistry {
    next_id: u32,
    images: std::collections::BTreeMap<u32, SpuImage>,
}

impl TestSpuImageRegistry {
    fn alloc_id(&mut self) -> u32 {
        self.next_id += 1;
        // Match C++ `lv2_spu_image::id_base = 0x22000000`.
        0x2200_0000 | self.next_id
    }
}

impl SpuImageRegistry for TestSpuImageRegistry {
    fn image_open(&mut self, _path: &str, image: SpuImage) -> Result<u32, CellError> {
        let id = self.alloc_id();
        self.images.insert(id, image);
        Ok(id)
    }
    fn image_import(&mut self, _src: u32, _size: u32, image: SpuImage) -> Result<u32, CellError> {
        let id = self.alloc_id();
        self.images.insert(id, image);
        Ok(id)
    }
    fn image_close(&mut self, id: u32) -> Result<(), CellError> {
        self.images.remove(&id).ok_or(CellError::ESRCH)?;
        Ok(())
    }
    fn image_get(&self, id: u32) -> Result<&SpuImage, CellError> {
        self.images.get(&id).ok_or(CellError::ESRCH)
    }
}

// =====================================================================
// Tests
// =====================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn load_phdr() -> SpuPhdr {
        SpuPhdr { p_type: PT_LOAD, p_offset: 0x100, p_vaddr: 0x2000, p_filesz: 0x400, p_memsz: 0x400 }
    }

    // --- constants / ABI -----------------------------------------

    #[test]
    fn segment_struct_is_0x18_bytes() {
        assert_eq!(core::mem::size_of::<SpuSegment>(), 0x18);
    }

    #[test]
    fn frozen_segment_type_ordinals() {
        assert_eq!(SPU_SEGMENT_TYPE_COPY, 1);
        assert_eq!(SPU_SEGMENT_TYPE_FILL, 2);
        assert_eq!(SPU_SEGMENT_TYPE_INFO, 4);
    }

    #[test]
    fn frozen_image_type_ordinals() {
        assert_eq!(SPU_IMAGE_TYPE_USER, 0);
        assert_eq!(SPU_IMAGE_TYPE_KERNEL, 1);
        assert_eq!(SPU_IMAGE_PROTECT, 0);
        assert_eq!(SPU_IMAGE_DIRECT, 1);
    }

    // --- count_segments ------------------------------------------

    #[test]
    fn count_loads_with_equal_filesz_memsz_gives_one_each() {
        let phdrs = vec![load_phdr(), load_phdr()];
        assert_eq!(count_segments(&phdrs, true), 2);
    }

    #[test]
    fn count_loads_with_bss_gives_two_per_header() {
        let phdr = SpuPhdr { p_memsz: 0x800, ..load_phdr() };
        assert_eq!(count_segments(&[phdr], true), 2);
    }

    #[test]
    fn count_note_adds_info_when_write_info_true() {
        let phdrs = vec![
            load_phdr(),
            SpuPhdr { p_type: PT_NOTE, p_offset: 0, p_vaddr: 0, p_filesz: 0x20, p_memsz: 0x20 },
        ];
        assert_eq!(count_segments(&phdrs, true), 2);
        assert_eq!(count_segments(&phdrs, false), 1);
    }

    #[test]
    fn count_rejects_unknown_ptype() {
        let phdrs = vec![SpuPhdr { p_type: 0xAA, ..load_phdr() }];
        assert_eq!(count_segments(&phdrs, true), -1);
    }

    // --- fill_segments -------------------------------------------

    #[test]
    fn fill_emits_copy_then_fill_for_bss_load() {
        let phdr = SpuPhdr { p_memsz: 0x600, ..load_phdr() };
        let mut segs = vec![SpuSegment::copy(0, 0, 0); 2];
        assert_eq!(fill_segments(&mut segs, &[phdr], 0x1000, true), 2);
        assert_eq!(segs[0].seg_type, SPU_SEGMENT_TYPE_COPY);
        assert_eq!(segs[0].ls, 0x2000);
        assert_eq!(segs[0].size, 0x400);
        assert_eq!(segs[0].addr, 0x1000 + 0x100);
        assert_eq!(segs[1].seg_type, SPU_SEGMENT_TYPE_FILL);
        assert_eq!(segs[1].ls, 0x2000 + 0x400);
        assert_eq!(segs[1].size, 0x200);
    }

    #[test]
    fn fill_emits_info_for_note_with_0x14_offset() {
        let phdr = SpuPhdr { p_type: PT_NOTE, p_offset: 0x200, p_vaddr: 0, p_filesz: 0x20, p_memsz: 0x20 };
        let mut segs = vec![SpuSegment::copy(0, 0, 0); 1];
        assert_eq!(fill_segments(&mut segs, &[phdr], 0x1000, true), 1);
        assert_eq!(segs[0].seg_type, SPU_SEGMENT_TYPE_INFO);
        assert_eq!(segs[0].size, 0x20);
        assert_eq!(segs[0].addr, 0x1000 + 0x200 + 0x14);
    }

    #[test]
    fn fill_skips_load_with_zero_filesz() {
        let phdr = SpuPhdr { p_filesz: 0, p_memsz: 0x400, ..load_phdr() };
        let mut segs = vec![SpuSegment::copy(0, 0, 0); 1];
        assert_eq!(fill_segments(&mut segs, &[phdr], 0, true), 1);
        assert_eq!(segs[0].seg_type, SPU_SEGMENT_TYPE_FILL);
    }

    #[test]
    fn fill_returns_minus_two_when_capacity_exceeded() {
        let phdr = load_phdr();
        let mut segs = vec![SpuSegment::copy(0, 0, 0); 0];
        assert_eq!(fill_segments(&mut segs, &[phdr], 0, true), -2);
    }

    // --- build_image + deploy ------------------------------------

    #[test]
    fn build_image_round_trip() {
        let phdrs = vec![
            SpuPhdr { p_type: PT_LOAD, p_offset: 0, p_vaddr: 0, p_filesz: 8, p_memsz: 12 },
            SpuPhdr { p_type: PT_NOTE, p_offset: 0x30, p_vaddr: 0, p_filesz: 0x20, p_memsz: 0x20 },
        ];
        let img = build_image(0xDEAD, &phdrs, 0x4000).unwrap();
        assert_eq!(img.image_type, SPU_IMAGE_TYPE_USER);
        assert_eq!(img.entry_point, 0xDEAD);
        assert_eq!(img.segments.len(), 3);
        assert_eq!(img.segments[0].seg_type, SPU_SEGMENT_TYPE_COPY);
        assert_eq!(img.segments[1].seg_type, SPU_SEGMENT_TYPE_FILL);
        assert_eq!(img.segments[2].seg_type, SPU_SEGMENT_TYPE_INFO);
    }

    #[test]
    fn deploy_applies_copy_and_fill_to_local_store() {
        let image = SpuImage {
            image_type: SPU_IMAGE_TYPE_USER,
            entry_point: 0,
            segments: vec![
                SpuSegment::copy(0, 4, 0x4000),
                SpuSegment::fill(0x100, 4),
                SpuSegment::info(0x20, 0),
            ],
        };
        let mut ls = vec![0xAAu8; LS_SIZE as usize];
        // Pre-dirty the FILL range so we can see it being zeroed.
        ls[0x100..0x104].copy_from_slice(&[0xFF, 0xFF, 0xFF, 0xFF]);

        deploy(&image, &mut ls, |addr, size| {
            assert_eq!(addr, 0x4000);
            assert_eq!(size, 4);
            Some(vec![0x11, 0x22, 0x33, 0x44])
        })
        .unwrap();

        assert_eq!(&ls[0..4], &[0x11, 0x22, 0x33, 0x44]);
        assert_eq!(&ls[0x100..0x104], &[0, 0, 0, 0]);
    }

    #[test]
    fn deploy_rejects_segment_past_ls_end() {
        let image = SpuImage {
            image_type: SPU_IMAGE_TYPE_USER,
            entry_point: 0,
            segments: vec![SpuSegment::fill(LS_SIZE - 3, 8)],
        };
        let mut ls = vec![0u8; LS_SIZE as usize];
        assert_eq!(deploy(&image, &mut ls, |_, _| None).unwrap_err(), CellError::EINVAL);
    }

    #[test]
    fn deploy_returns_efault_when_fetch_fails() {
        let image = SpuImage {
            image_type: SPU_IMAGE_TYPE_USER,
            entry_point: 0,
            segments: vec![SpuSegment::copy(0, 4, 0x100)],
        };
        let mut ls = vec![0u8; LS_SIZE as usize];
        assert_eq!(deploy(&image, &mut ls, |_, _| None).unwrap_err(), CellError::EFAULT);
    }

    // --- registry / syscalls -------------------------------------

    #[test]
    fn id_base_is_0x22000000() {
        let mut reg = TestSpuImageRegistry::default();
        let img = SpuImage { image_type: 0, entry_point: 0, segments: vec![] };
        let id = sys_spu_image_open(&mut reg, "/app_home/main.spu", img).unwrap();
        assert_eq!(id & 0xFF00_0000, 0x2200_0000);
    }

    #[test]
    fn open_with_empty_path_is_einval() {
        let mut reg = TestSpuImageRegistry::default();
        let img = SpuImage { image_type: 0, entry_point: 0, segments: vec![] };
        assert_eq!(sys_spu_image_open(&mut reg, "", img).unwrap_err(), CellError::EINVAL);
    }

    #[test]
    fn import_with_zero_size_is_einval() {
        let mut reg = TestSpuImageRegistry::default();
        let img = SpuImage { image_type: 0, entry_point: 0, segments: vec![] };
        assert_eq!(sys_spu_image_import(&mut reg, 0x1000, 0, img).unwrap_err(), CellError::EINVAL);
    }

    #[test]
    fn close_unknown_id_is_esrch() {
        let mut reg = TestSpuImageRegistry::default();
        assert_eq!(sys_spu_image_close(&mut reg, 0x2200_0042).unwrap_err(), CellError::ESRCH);
    }

    #[test]
    fn get_info_returns_entry_and_segment_count() {
        let mut reg = TestSpuImageRegistry::default();
        let img = SpuImage {
            image_type: 0,
            entry_point: 0xBEEF,
            segments: vec![SpuSegment::copy(0, 4, 0x100); 3],
        };
        let id = sys_spu_image_import(&mut reg, 0x100, 16, img).unwrap();
        let (entry, nsegs) = sys_spu_image_get_info(&reg, id).unwrap();
        assert_eq!(entry, 0xBEEF);
        assert_eq!(nsegs, 3);
    }

    #[test]
    fn get_segments_copies_up_to_dst_capacity() {
        let mut reg = TestSpuImageRegistry::default();
        let img = SpuImage {
            image_type: 0,
            entry_point: 0,
            segments: vec![
                SpuSegment::copy(0, 4, 0x100),
                SpuSegment::fill(0x100, 8),
                SpuSegment::info(0x20, 0x200),
            ],
        };
        let id = sys_spu_image_import(&mut reg, 0, 1, img).unwrap();
        let mut dst = vec![SpuSegment::copy(0, 0, 0); 2];
        assert_eq!(sys_spu_image_get_segments(&reg, id, &mut dst).unwrap(), 2);
        assert_eq!(dst[0].seg_type, SPU_SEGMENT_TYPE_COPY);
        assert_eq!(dst[1].seg_type, SPU_SEGMENT_TYPE_FILL);
    }
}
