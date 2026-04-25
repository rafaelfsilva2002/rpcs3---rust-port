//! Rust port of `rpcs3/Emu/Cell/lv2/sys_overlay.cpp` — PS3 LV2 overlay PRX
//! syscalls (3 entries, 204 lines C++).
//!
//! Overlays são módulos PRX dinamicamente carregados em runtime, comuns em
//! jogos AAA grandes. Surface:
//!
//! * `sys_overlay_load_module(ovlmid, path, flags, entry)` — carrega de path.
//! * `sys_overlay_load_module_by_fd(ovlmid, fd, offset, flags, entry)` —
//!   carrega de fd já aberto + offset (para PRX dentro de container PSARC).
//! * `sys_overlay_unload_module(ovlmid)` — descarrega + libera memória.
//!
//! Validation cascade preservada cpp:133-176:
//! * `!ppc_seg` → CELL_ENOSYS (processo não tem permissão para overlays)
//! * `!path` → CELL_EFAULT
//! * `offset < 0` (signed s64) → CELL_EINVAL
//! * `fd not found` → CELL_EBADF
//! * `unload unknown id` → CELL_ESRCH
//!
//! O carregamento real é plugado via `OverlayLoader` trait — a decifração
//! self/edat + parse ELF + map PT_LOAD segments fica fora do escopo desta
//! crate (mora em rpcs3-loader-elf-self + rpcs3-crypto).
//!
//! `no_std` + `alloc`. Single dep: `rpcs3-emu-types`.

#![no_std]

extern crate alloc;

use alloc::string::String;
use alloc::vec::Vec;
use rpcs3_emu_types::CellError;

pub const MODULE_NAME: &str = "sys_overlay";

pub const REGISTERED_ENTRY_POINTS: &[&str] = &[
    "sys_overlay_load_module",
    "sys_overlay_load_module_by_fd",
    "sys_overlay_unload_module",
];

pub const CELL_ENOSYS: CellError = CellError(0x8001_0001);
pub const CELL_EINVAL: CellError = CellError(0x8001_0002);
pub const CELL_EFAULT: CellError = CellError(0x8001_000D);
pub const CELL_EBADF: CellError = CellError(0x8001_0009);
pub const CELL_ESRCH: CellError = CellError(0x8001_0005);
pub const CELL_ENOEXEC: CellError = CellError(0x8001_0008);
pub const CELL_CANCEL: CellError = CellError(0x8001_0028);

/// `lv2_overlay` segment record (mirror of cpp:196-199 dealloc loop).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OverlaySegment {
    pub addr: u32,
    pub size: u32,
}

/// `lv2_overlay` mirror — minimal public face.
#[derive(Debug, Clone)]
pub struct LoadedOverlay {
    pub id: u32,
    pub vpath: String,
    pub file_offset: i64,
    pub entry: u32,
    pub segs: Vec<OverlaySegment>,
}

/// `lv2_file` mirror for the by-fd variant.
#[derive(Debug, Clone)]
pub struct LoadedFile {
    pub fd: u32,
    pub name: String,
    /// Whether `file` is open; cpp:171 returns CELL_EBADF if closed.
    pub open: bool,
}

/// Pluggable overlay loader — wraps `ppu_load_overlay` in production.
pub trait OverlayLoader {
    /// Mirror of `overlay_load_module`'s success path. Returns the loaded
    /// overlay (caller assigns ID).
    fn load_overlay(
        &mut self,
        vpath: &str,
        file_offset: i64,
    ) -> Result<LoadedOverlay, CellError>;
}

/// Test loader: loads a single canned overlay or fails as configured.
#[derive(Debug, Default)]
pub struct MockLoader {
    pub next_entry: u32,
    pub segs_per_load: Vec<OverlaySegment>,
    pub fail_with: Option<CellError>,
    pub load_calls: Vec<(String, i64)>,
}

impl OverlayLoader for MockLoader {
    fn load_overlay(
        &mut self,
        vpath: &str,
        file_offset: i64,
    ) -> Result<LoadedOverlay, CellError> {
        self.load_calls.push((vpath.into(), file_offset));
        if let Some(e) = self.fail_with {
            return Err(e);
        }
        Ok(LoadedOverlay {
            id: 0, // SysOverlay assigns
            vpath: vpath.into(),
            file_offset,
            entry: self.next_entry,
            segs: self.segs_per_load.clone(),
        })
    }
}

#[derive(Debug, Default)]
pub struct SysOverlay {
    pub overlays: Vec<LoadedOverlay>,
    pub files: Vec<LoadedFile>,
    pub next_id: u32,
    pub ppc_seg_permitted: bool,

    pub load_calls: u64,
    pub load_by_fd_calls: u64,
    pub unload_calls: u64,
}

impl SysOverlay {
    pub fn new() -> Self {
        Self {
            ppc_seg_permitted: true,
            next_id: 1,
            ..Default::default()
        }
    }

    /// Test/scaffold helper.
    pub fn register_file(&mut self, fd: u32, name: &str, open: bool) {
        self.files.push(LoadedFile {
            fd,
            name: name.into(),
            open,
        });
    }

    fn find_file(&self, fd: u32) -> Option<&LoadedFile> {
        self.files.iter().find(|f| f.fd == fd)
    }

    /// `sys_overlay_load_module(ovlmid, path, flags, entry)` — cpp:129-145.
    pub fn load_module<L: OverlayLoader>(
        &mut self,
        loader: &mut L,
        path: Option<&str>,
        _flags: u64,
        ovlmid_out: Option<&mut u32>,
        entry_out: Option<&mut u32>,
    ) -> Result<(), CellError> {
        self.load_calls = self.load_calls.saturating_add(1);
        if !self.ppc_seg_permitted {
            return Err(CELL_ENOSYS);
        }
        let path = path.ok_or(CELL_EFAULT)?;
        let mut ovlm = loader.load_overlay(path, 0)?;
        let id = self.next_id;
        self.next_id = self.next_id.wrapping_add(1);
        ovlm.id = id;
        let entry = ovlm.entry;
        self.overlays.push(ovlm);
        if let Some(slot) = ovlmid_out {
            *slot = id;
        }
        if let Some(slot) = entry_out {
            *slot = entry;
        }
        Ok(())
    }

    /// `sys_overlay_load_module_by_fd(ovlmid, fd, offset, flags, entry)` —
    /// cpp:147-177. Validation: ppc_seg → offset≥0 → fd lookup → fd open.
    pub fn load_module_by_fd<L: OverlayLoader>(
        &mut self,
        loader: &mut L,
        fd: u32,
        offset: u64,
        _flags: u64,
        ovlmid_out: Option<&mut u32>,
        entry_out: Option<&mut u32>,
    ) -> Result<(), CellError> {
        self.load_by_fd_calls = self.load_by_fd_calls.saturating_add(1);
        if !self.ppc_seg_permitted {
            return Err(CELL_ENOSYS);
        }
        // cpp:157-160 — signed cast check.
        if (offset as i64) < 0 {
            return Err(CELL_EINVAL);
        }
        let file = self.find_file(fd).ok_or(CELL_EBADF)?.clone();
        if !file.open {
            return Err(CELL_EBADF);
        }
        // cpp:176 — virtual path is "<name>_x<offset>" if offset != 0, else just name.
        let vpath = if offset != 0 {
            let mut s = file.name.clone();
            // Use upper-hex with `_x` prefix matching cpp fmt::format.
            s.push_str("_x");
            push_hex(&mut s, offset);
            s
        } else {
            file.name.clone()
        };
        let mut ovlm = loader.load_overlay(&vpath, offset as i64)?;
        let id = self.next_id;
        self.next_id = self.next_id.wrapping_add(1);
        ovlm.id = id;
        let entry = ovlm.entry;
        self.overlays.push(ovlm);
        if let Some(slot) = ovlmid_out {
            *slot = id;
        }
        if let Some(slot) = entry_out {
            *slot = entry;
        }
        Ok(())
    }

    /// `sys_overlay_unload_module(ovlmid)` — cpp:179-204.
    /// Returns the segments that need to be deallocated (caller wires
    /// `vm::dealloc(seg.addr)` per the cpp:196-199 loop).
    pub fn unload_module(&mut self, ovlmid: u32) -> Result<Vec<OverlaySegment>, CellError> {
        self.unload_calls = self.unload_calls.saturating_add(1);
        if !self.ppc_seg_permitted {
            return Err(CELL_ENOSYS);
        }
        let pos = self
            .overlays
            .iter()
            .position(|o| o.id == ovlmid)
            .ok_or(CELL_ESRCH)?;
        let removed = self.overlays.remove(pos);
        Ok(removed.segs)
    }
}

/// Append `value` as lowercase hex (no leading zeros) to `s`.
/// Mirror of `fmt::format("%x", value)` cpp:176.
fn push_hex(s: &mut String, value: u64) {
    if value == 0 {
        s.push('0');
        return;
    }
    let hex_chars = b"0123456789abcdef";
    let mut buf = [0u8; 16];
    let mut len = 0;
    let mut v = value;
    while v != 0 {
        buf[len] = hex_chars[(v & 0xF) as usize];
        v >>= 4;
        len += 1;
    }
    // Reverse and push.
    for i in (0..len).rev() {
        s.push(buf[i] as char);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    #[test]
    fn module_and_entries() {
        assert_eq!(MODULE_NAME, "sys_overlay");
        assert_eq!(REGISTERED_ENTRY_POINTS.len(), 3);
    }

    #[test]
    fn errors_byte_exact() {
        assert_eq!(CELL_ENOSYS.0, 0x8001_0001);
        assert_eq!(CELL_EINVAL.0, 0x8001_0002);
        assert_eq!(CELL_EBADF.0, 0x8001_0009);
        assert_eq!(CELL_EFAULT.0, 0x8001_000D);
        assert_eq!(CELL_ESRCH.0, 0x8001_0005);
    }

    #[test]
    fn load_no_ppc_seg_enosys() {
        let mut m = SysOverlay::new();
        m.ppc_seg_permitted = false;
        let mut loader = MockLoader::default();
        assert_eq!(
            m.load_module(&mut loader, Some("/dev_hdd0/game.prx"), 0, None, None),
            Err(CELL_ENOSYS)
        );
    }

    #[test]
    fn load_null_path_efault() {
        let mut m = SysOverlay::new();
        let mut loader = MockLoader::default();
        assert_eq!(
            m.load_module(&mut loader, None, 0, None, None),
            Err(CELL_EFAULT)
        );
    }

    #[test]
    fn load_succeeds_assigns_id_and_entry() {
        let mut m = SysOverlay::new();
        let mut loader = MockLoader {
            next_entry: 0x4000_1000,
            segs_per_load: vec![OverlaySegment {
                addr: 0x4000_0000,
                size: 0x10000,
            }],
            ..Default::default()
        };
        let mut ovlmid = 0u32;
        let mut entry = 0u32;
        m.load_module(
            &mut loader,
            Some("/dev_hdd0/game/USRDIR/audio.prx"),
            0,
            Some(&mut ovlmid),
            Some(&mut entry),
        )
        .unwrap();
        assert_eq!(ovlmid, 1);
        assert_eq!(entry, 0x4000_1000);
        assert_eq!(loader.load_calls.len(), 1);
        assert_eq!(loader.load_calls[0].0, "/dev_hdd0/game/USRDIR/audio.prx");
        assert_eq!(loader.load_calls[0].1, 0);
        assert_eq!(m.overlays.len(), 1);
        assert_eq!(m.overlays[0].segs.len(), 1);
    }

    #[test]
    fn load_by_fd_no_ppc_seg_enosys() {
        let mut m = SysOverlay::new();
        m.ppc_seg_permitted = false;
        let mut loader = MockLoader::default();
        assert_eq!(
            m.load_module_by_fd(&mut loader, 5, 0, 0, None, None),
            Err(CELL_ENOSYS)
        );
    }

    #[test]
    fn load_by_fd_negative_offset_einval() {
        let mut m = SysOverlay::new();
        let mut loader = MockLoader::default();
        // cpp:157 `static_cast<s64>(offset) < 0` — negative when treated as signed.
        let neg = u64::MAX; // -1 as i64
        assert_eq!(
            m.load_module_by_fd(&mut loader, 5, neg, 0, None, None),
            Err(CELL_EINVAL)
        );
    }

    #[test]
    fn load_by_fd_unknown_fd_ebadf() {
        let mut m = SysOverlay::new();
        let mut loader = MockLoader::default();
        assert_eq!(
            m.load_module_by_fd(&mut loader, 99, 0, 0, None, None),
            Err(CELL_EBADF)
        );
    }

    #[test]
    fn load_by_fd_closed_file_ebadf() {
        let mut m = SysOverlay::new();
        m.register_file(5, "/dev_hdd0/data.psarc", false); // closed
        let mut loader = MockLoader::default();
        assert_eq!(
            m.load_module_by_fd(&mut loader, 5, 0, 0, None, None),
            Err(CELL_EBADF)
        );
    }

    #[test]
    fn load_by_fd_offset_zero_uses_filename_only() {
        let mut m = SysOverlay::new();
        m.register_file(5, "/dev_hdd0/data.prx", true);
        let mut loader = MockLoader {
            next_entry: 0x1000,
            ..Default::default()
        };
        let mut id = 0u32;
        m.load_module_by_fd(&mut loader, 5, 0, 0, Some(&mut id), None).unwrap();
        // cpp:176 — offset==0 path is just file.name (no _x suffix).
        assert_eq!(loader.load_calls[0].0, "/dev_hdd0/data.prx");
        assert_eq!(loader.load_calls[0].1, 0);
    }

    #[test]
    fn load_by_fd_nonzero_offset_appends_hex_suffix() {
        let mut m = SysOverlay::new();
        m.register_file(5, "/dev_hdd0/pkg.psarc", true);
        let mut loader = MockLoader::default();
        let mut id = 0u32;
        m.load_module_by_fd(&mut loader, 5, 0xABCD, 0, Some(&mut id), None)
            .unwrap();
        // cpp:176 — `<name>_x<offset_hex>` with lowercase hex.
        assert_eq!(loader.load_calls[0].0, "/dev_hdd0/pkg.psarc_xabcd");
        assert_eq!(loader.load_calls[0].1, 0xABCD);
    }

    #[test]
    fn unload_unknown_id_esrch() {
        let mut m = SysOverlay::new();
        assert_eq!(m.unload_module(99), Err(CELL_ESRCH));
    }

    #[test]
    fn unload_no_ppc_seg_enosys() {
        let mut m = SysOverlay::new();
        m.ppc_seg_permitted = false;
        assert_eq!(m.unload_module(1), Err(CELL_ENOSYS));
    }

    #[test]
    fn unload_returns_segments_for_dealloc() {
        let mut m = SysOverlay::new();
        let mut loader = MockLoader {
            next_entry: 0x1000,
            segs_per_load: vec![
                OverlaySegment {
                    addr: 0x4000_0000,
                    size: 0x10000,
                },
                OverlaySegment {
                    addr: 0x4001_0000,
                    size: 0x4000,
                },
            ],
            ..Default::default()
        };
        let mut id = 0u32;
        m.load_module(&mut loader, Some("/test.prx"), 0, Some(&mut id), None).unwrap();
        let segs = m.unload_module(id).unwrap();
        assert_eq!(segs.len(), 2);
        assert_eq!(segs[0].addr, 0x4000_0000);
        assert_eq!(segs[1].size, 0x4000);
        // Re-unload fails.
        assert_eq!(m.unload_module(id), Err(CELL_ESRCH));
    }

    #[test]
    fn loader_error_propagates() {
        let mut m = SysOverlay::new();
        let mut loader = MockLoader {
            fail_with: Some(CELL_ENOEXEC),
            ..Default::default()
        };
        assert_eq!(
            m.load_module(&mut loader, Some("/bad.prx"), 0, None, None),
            Err(CELL_ENOEXEC)
        );
        // No overlay registered when load failed.
        assert!(m.overlays.is_empty());
    }

    #[test]
    fn push_hex_basic() {
        let mut s = String::new();
        push_hex(&mut s, 0);
        assert_eq!(s, "0");
        s.clear();
        push_hex(&mut s, 0xABCD);
        assert_eq!(s, "abcd");
        s.clear();
        push_hex(&mut s, 0x1000);
        assert_eq!(s, "1000");
    }

    #[test]
    fn full_overlay_lifecycle_smoke() {
        let mut m = SysOverlay::new();
        m.register_file(10, "/dev_hdd0/game/PS3_GAME/USRDIR/data.psarc", true);
        let mut loader = MockLoader {
            next_entry: 0x4000_F000,
            segs_per_load: vec![OverlaySegment {
                addr: 0x4000_0000,
                size: 0x100000,
            }],
            ..Default::default()
        };

        // Load by path.
        let mut id1 = 0u32;
        let mut entry1 = 0u32;
        m.load_module(
            &mut loader,
            Some("/dev_hdd0/game/PS3_GAME/USRDIR/audio.prx"),
            0,
            Some(&mut id1),
            Some(&mut entry1),
        )
        .unwrap();
        assert_eq!(id1, 1);
        assert_eq!(entry1, 0x4000_F000);

        // Load by fd with offset.
        let mut id2 = 0u32;
        m.load_module_by_fd(&mut loader, 10, 0x100, 0, Some(&mut id2), None).unwrap();
        assert_eq!(id2, 2);
        assert_eq!(loader.load_calls[1].0, "/dev_hdd0/game/PS3_GAME/USRDIR/data.psarc_x100");

        // Unload first.
        let segs1 = m.unload_module(id1).unwrap();
        assert_eq!(segs1.len(), 1);

        // Second still alive.
        assert_eq!(m.overlays.len(), 1);
        assert_eq!(m.overlays[0].id, 2);

        // Unload second.
        m.unload_module(id2).unwrap();
        assert!(m.overlays.is_empty());
    }
}
