//! `rpcs3-hle-cellgcm` — GCM (Graphics Command Manager) HLE layer.
//!
//! Ports the shadow-state subset of
//! `rpcs3/Emu/Cell/Modules/cellGcmSys.cpp`. GCM is Sony's low-level
//! GPU interface: games push commands into a ring buffer in main
//! memory, the RSX reads them. The HLE layer doesn't emulate the RSX
//! itself (that's out of scope for this crate) — instead it tracks
//! the *state* games register with it: display buffers (up to 8),
//! tile regions (up to 15), zcull regions (up to 8), IO address
//! table, and the flip mode.
//!
//! ## Entry points covered (iter 1)
//!
//! | HLE function                              | Rust wrapper                        |
//! |-------------------------------------------|-------------------------------------|
//! | `cellGcmInitBody`                         | [`cell_gcm_init_body`]              |
//! | `cellGcmGetConfiguration`                 | [`cell_gcm_get_configuration`]      |
//! | `cellGcmSetDisplayBuffer`                 | [`cell_gcm_set_display_buffer`]     |
//! | `cellGcmAddressToOffset`                  | [`cell_gcm_address_to_offset`]      |
//! | `cellGcmIoOffsetToAddress`                | [`cell_gcm_io_offset_to_address`]   |
//! | `cellGcmSetTileInfo`                      | [`cell_gcm_set_tile_info`]          |
//! | `cellGcmBindTile`                         | [`cell_gcm_bind_tile`]              |
//! | `cellGcmUnbindTile`                       | [`cell_gcm_unbind_tile`]            |
//! | `cellGcmSetZcull`                         | [`cell_gcm_set_zcull`]              |
//! | `cellGcmSetFlipMode`                      | [`cell_gcm_set_flip_mode`]          |
//! | `cellGcmGetCurrentField`                  | [`cell_gcm_get_current_field`]      |
//! | `cellGcmGetReport` / `cellGcmSetReport`   | [`cell_gcm_get_report`] / [`cell_gcm_set_report`] |
//!
//! ## Frozen constants
//!
//! | Const                   | Value  |
//! |-------------------------|-------:|
//! | `MAX_DISPLAY_BUFFERS`   | 8      |
//! | `MAX_TILES`             | 15     |
//! | `MAX_ZCULLS`            | 8      |
//! | `LOCATION_LOCAL`        | 0      |
//! | `LOCATION_MAIN`         | 1      |
//! | `FLIP_MODE_HSYNC`       | 1      |
//! | `FLIP_MODE_VSYNC`       | 2      |

use rpcs3_emu_types::CellError;

// =====================================================================
// Error codes
// =====================================================================

pub mod errors {
    use rpcs3_emu_types::CellError;

    pub const FAILURE: CellError = CellError(0x8021_00FF);
    pub const NO_IO_PAGE_TABLE: CellError = CellError(0x8021_0001);
    pub const INVALID_ENUM: CellError = CellError(0x8021_0002);
    pub const INVALID_VALUE: CellError = CellError(0x8021_0003);
    pub const INVALID_ALIGNMENT: CellError = CellError(0x8021_0004);
    pub const ADDRESS_OVERWRAP: CellError = CellError(0x8021_0005);
}

// =====================================================================
// Frozen limits
// =====================================================================

pub const MAX_DISPLAY_BUFFERS: usize = 8;
pub const MAX_TILES: usize = 15;
pub const MAX_ZCULLS: usize = 8;

pub const LOCATION_LOCAL: u32 = 0;
pub const LOCATION_MAIN: u32 = 1;

pub const FLIP_MODE_HSYNC: u32 = 1;
pub const FLIP_MODE_VSYNC: u32 = 2;

/// Guest local-memory base for RSX video RAM. Constant across models.
pub const DEFAULT_LOCAL_ADDR: u32 = 0xC000_0000;
/// Guest IO base — where the mapped main-memory window starts.
pub const DEFAULT_IO_ADDR: u32 = 0x0F90_0000;

/// 1 MB IO page size (LV2 unit for GCM IO table).
pub const IO_PAGE_SIZE: u32 = 0x100_000;

// =====================================================================
// Data model
// =====================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GcmConfig {
    pub local_address: u32,
    pub io_address: u32,
    pub local_size: u32,
    pub io_size: u32,
    pub memory_frequency: u32,
    pub core_frequency: u32,
}

impl Default for GcmConfig {
    fn default() -> Self {
        Self {
            local_address: DEFAULT_LOCAL_ADDR,
            io_address: DEFAULT_IO_ADDR,
            local_size: 0x0F90_0000,  // 249 MB default video RAM
            io_size: 0,               // set by Init body
            memory_frequency: 650_000_000,
            core_frequency: 500_000_000,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct DisplayBuffer {
    pub offset: u32,
    pub pitch: u32,
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TileInfo {
    pub location: u32,
    pub offset: u32,
    pub size: u32,
    pub pitch: u32,
    pub comp: u32,
    pub base: u16,
    pub bank: u8,
    pub bound: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ZcullInfo {
    pub offset: u32,
    pub width: u32,
    pub height: u32,
    pub cull_start: u32,
    pub z_format: u32,
    pub aa_format: u32,
    pub z_cull_dir: u32,
    pub z_cull_format: u32,
    pub s_func: u32,
    pub s_ref: u32,
    pub s_mask: u32,
    pub bound: bool,
}

// =====================================================================
// Manager
// =====================================================================

#[derive(Debug)]
pub struct GcmManager {
    initialized: bool,
    config: GcmConfig,
    display_buffers: [Option<DisplayBuffer>; MAX_DISPLAY_BUFFERS],
    tiles: [TileInfo; MAX_TILES],
    zculls: [ZcullInfo; MAX_ZCULLS],
    flip_mode: u32,
    current_field: u32,
    reports: std::collections::BTreeMap<u32, u32>,
    /// IO offset → guest EA table. 1 MB granularity.
    io_table: std::collections::BTreeMap<u32, u32>,
}

impl Default for GcmManager {
    fn default() -> Self {
        Self {
            initialized: false,
            config: GcmConfig::default(),
            display_buffers: [None; MAX_DISPLAY_BUFFERS],
            tiles: [TileInfo::default(); MAX_TILES],
            zculls: [ZcullInfo::default(); MAX_ZCULLS],
            flip_mode: FLIP_MODE_VSYNC,
            current_field: 0,
            reports: std::collections::BTreeMap::new(),
            io_table: std::collections::BTreeMap::new(),
        }
    }
}

// =====================================================================
// Syscalls
// =====================================================================

fn ensure_init(m: &GcmManager) -> Result<(), CellError> {
    if m.initialized { Ok(()) } else { Err(errors::FAILURE) }
}

/// `cellGcmInitBody(context_out, cmd_size, io_size, io_address)` —
/// initialise the GCM subsystem. `io_address` must be 1 MB aligned.
#[must_use]
pub fn cell_gcm_init_body(
    m: &mut GcmManager,
    cmd_size: u32,
    io_size: u32,
    io_address: u32,
) -> Result<(), CellError> {
    if m.initialized {
        return Err(errors::FAILURE);
    }
    if io_address & (IO_PAGE_SIZE - 1) != 0 {
        return Err(errors::INVALID_ALIGNMENT);
    }
    if io_size == 0 || (io_size & (IO_PAGE_SIZE - 1)) != 0 {
        return Err(errors::INVALID_VALUE);
    }
    if cmd_size == 0 {
        return Err(errors::INVALID_VALUE);
    }

    m.config.io_address = io_address;
    m.config.io_size = io_size;
    // Register an identity-style IO table: each 1 MB IO page maps to
    // the corresponding guest page just above `io_address`.
    for i in 0..(io_size / IO_PAGE_SIZE) {
        let off = i * IO_PAGE_SIZE;
        m.io_table.insert(off, io_address + off);
    }
    m.initialized = true;
    Ok(())
}

/// `cellGcmGetConfiguration(config_out)`.
#[must_use]
pub fn cell_gcm_get_configuration(m: &GcmManager) -> Result<GcmConfig, CellError> {
    ensure_init(m)?;
    Ok(m.config)
}

/// `cellGcmSetDisplayBuffer(id, offset, pitch, width, height)`.
#[must_use]
pub fn cell_gcm_set_display_buffer(
    m: &mut GcmManager,
    id: u8,
    offset: u32,
    pitch: u32,
    width: u32,
    height: u32,
) -> Result<(), CellError> {
    ensure_init(m)?;
    if (id as usize) >= MAX_DISPLAY_BUFFERS {
        return Err(errors::INVALID_VALUE);
    }
    if pitch == 0 || width == 0 || height == 0 {
        return Err(errors::INVALID_VALUE);
    }
    m.display_buffers[id as usize] = Some(DisplayBuffer { offset, pitch, width, height });
    Ok(())
}

/// Look up a display buffer (test convenience).
#[must_use]
pub fn get_display_buffer(m: &GcmManager, id: u8) -> Option<DisplayBuffer> {
    m.display_buffers.get(id as usize).copied().flatten()
}

/// `cellGcmAddressToOffset(address, offset_out)` — translate guest
/// EA to IO offset via the registered IO table. EA must fall inside
/// either the local memory region or a registered IO page.
#[must_use]
pub fn cell_gcm_address_to_offset(m: &GcmManager, address: u32) -> Result<u32, CellError> {
    ensure_init(m)?;
    // Local-memory addresses are returned unchanged relative to
    // local_address (matches C++ behavior for RSX-local VRAM).
    let lo = m.config.local_address;
    let hi = lo.checked_add(m.config.local_size).ok_or(errors::ADDRESS_OVERWRAP)?;
    if address >= lo && address < hi {
        return Ok(address - lo);
    }
    // IO translation.
    for (&io_off, &ea) in &m.io_table {
        if address >= ea && address < ea + IO_PAGE_SIZE {
            return Ok(io_off + (address - ea));
        }
    }
    Err(errors::FAILURE)
}

/// `cellGcmIoOffsetToAddress(io_offset, address_out)`.
#[must_use]
pub fn cell_gcm_io_offset_to_address(m: &GcmManager, io_offset: u32) -> Result<u32, CellError> {
    ensure_init(m)?;
    let page = io_offset & !(IO_PAGE_SIZE - 1);
    let inner = io_offset & (IO_PAGE_SIZE - 1);
    let ea = m.io_table.get(&page).copied().ok_or(errors::FAILURE)?;
    Ok(ea + inner)
}

/// `cellGcmSetTileInfo(index, location, offset, size, pitch, comp, base, bank)`.
#[must_use]
pub fn cell_gcm_set_tile_info(
    m: &mut GcmManager,
    index: u8,
    location: u32,
    offset: u32,
    size: u32,
    pitch: u32,
    comp: u32,
    base: u16,
    bank: u8,
) -> Result<(), CellError> {
    ensure_init(m)?;
    if (index as usize) >= MAX_TILES {
        return Err(errors::INVALID_VALUE);
    }
    if location != LOCATION_LOCAL && location != LOCATION_MAIN {
        return Err(errors::INVALID_ENUM);
    }
    if pitch != 0 && pitch % 64 != 0 {
        return Err(errors::INVALID_ALIGNMENT);
    }
    let tile = &mut m.tiles[index as usize];
    tile.location = location;
    tile.offset = offset;
    tile.size = size;
    tile.pitch = pitch;
    tile.comp = comp;
    tile.base = base;
    tile.bank = bank;
    Ok(())
}

/// `cellGcmBindTile(index)`.
#[must_use]
pub fn cell_gcm_bind_tile(m: &mut GcmManager, index: u8) -> Result<(), CellError> {
    ensure_init(m)?;
    if (index as usize) >= MAX_TILES {
        return Err(errors::INVALID_VALUE);
    }
    m.tiles[index as usize].bound = true;
    Ok(())
}

/// `cellGcmUnbindTile(index)`.
#[must_use]
pub fn cell_gcm_unbind_tile(m: &mut GcmManager, index: u8) -> Result<(), CellError> {
    ensure_init(m)?;
    if (index as usize) >= MAX_TILES {
        return Err(errors::INVALID_VALUE);
    }
    m.tiles[index as usize].bound = false;
    Ok(())
}

/// Test helper: inspect a tile slot.
#[must_use]
pub fn get_tile(m: &GcmManager, index: u8) -> Option<TileInfo> {
    m.tiles.get(index as usize).copied()
}

/// `cellGcmSetZcull(index, offset, width, height, ...)`.
#[must_use]
pub fn cell_gcm_set_zcull(
    m: &mut GcmManager,
    index: u8,
    info: ZcullInfo,
) -> Result<(), CellError> {
    ensure_init(m)?;
    if (index as usize) >= MAX_ZCULLS {
        return Err(errors::INVALID_VALUE);
    }
    m.zculls[index as usize] = ZcullInfo { bound: true, ..info };
    Ok(())
}

#[must_use]
pub fn get_zcull(m: &GcmManager, index: u8) -> Option<ZcullInfo> {
    m.zculls.get(index as usize).copied()
}

/// `cellGcmSetFlipMode(mode)`.
#[must_use]
pub fn cell_gcm_set_flip_mode(m: &mut GcmManager, mode: u32) -> Result<(), CellError> {
    ensure_init(m)?;
    match mode {
        FLIP_MODE_HSYNC | FLIP_MODE_VSYNC => {
            m.flip_mode = mode;
            Ok(())
        }
        _ => Err(errors::INVALID_ENUM),
    }
}

#[must_use]
pub fn get_flip_mode(m: &GcmManager) -> u32 {
    m.flip_mode
}

/// `cellGcmGetCurrentField()` — returns 0 (top) or 1 (bottom). Our
/// implementation is stateful: callers can bump it via [`advance_field`].
#[must_use]
pub fn cell_gcm_get_current_field(m: &GcmManager) -> u32 {
    m.current_field
}

pub fn advance_field(m: &mut GcmManager) {
    m.current_field ^= 1;
}

/// `cellGcmGetReport(type, index)` — returns stored counter.
#[must_use]
pub fn cell_gcm_get_report(m: &GcmManager, index: u32) -> u32 {
    m.reports.get(&index).copied().unwrap_or(0)
}

/// `cellGcmSetReport(type, index)` — stores counter.
pub fn cell_gcm_set_report(m: &mut GcmManager, index: u32, value: u32) {
    m.reports.insert(index, value);
}

// =====================================================================
// Tests
// =====================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn init_mgr() -> GcmManager {
        let mut m = GcmManager::default();
        cell_gcm_init_body(&mut m, 0x400, 0x1_000_000, DEFAULT_IO_ADDR).unwrap();
        m
    }

    // --- constants ------------------------------------------------

    #[test]
    fn error_codes_byte_exact() {
        assert_eq!(errors::FAILURE.0, 0x8021_00FF);
        assert_eq!(errors::NO_IO_PAGE_TABLE.0, 0x8021_0001);
        assert_eq!(errors::INVALID_ENUM.0, 0x8021_0002);
        assert_eq!(errors::INVALID_VALUE.0, 0x8021_0003);
        assert_eq!(errors::INVALID_ALIGNMENT.0, 0x8021_0004);
        assert_eq!(errors::ADDRESS_OVERWRAP.0, 0x8021_0005);
    }

    #[test]
    fn layout_constants_frozen() {
        assert_eq!(MAX_DISPLAY_BUFFERS, 8);
        assert_eq!(MAX_TILES, 15);
        assert_eq!(MAX_ZCULLS, 8);
        assert_eq!(IO_PAGE_SIZE, 0x100_000);
        assert_eq!(LOCATION_LOCAL, 0);
        assert_eq!(LOCATION_MAIN, 1);
    }

    // --- init -----------------------------------------------------

    #[test]
    fn init_rejects_unaligned_io_address() {
        let mut m = GcmManager::default();
        assert_eq!(
            cell_gcm_init_body(&mut m, 0x400, 0x100_000, 0x0F90_0001).unwrap_err(),
            errors::INVALID_ALIGNMENT,
        );
    }

    #[test]
    fn init_rejects_unaligned_io_size() {
        let mut m = GcmManager::default();
        assert_eq!(
            cell_gcm_init_body(&mut m, 0x400, 0x12345, DEFAULT_IO_ADDR).unwrap_err(),
            errors::INVALID_VALUE,
        );
    }

    #[test]
    fn init_twice_is_failure() {
        let mut m = init_mgr();
        assert_eq!(
            cell_gcm_init_body(&mut m, 0x400, 0x100_000, DEFAULT_IO_ADDR).unwrap_err(),
            errors::FAILURE,
        );
    }

    #[test]
    fn get_configuration_without_init_is_failure() {
        let m = GcmManager::default();
        assert_eq!(cell_gcm_get_configuration(&m).unwrap_err(), errors::FAILURE);
    }

    #[test]
    fn get_configuration_returns_io_size_set_at_init() {
        let m = init_mgr();
        let cfg = cell_gcm_get_configuration(&m).unwrap();
        assert_eq!(cfg.io_size, 0x1_000_000);
        assert_eq!(cfg.io_address, DEFAULT_IO_ADDR);
    }

    // --- display buffers ------------------------------------------

    #[test]
    fn set_display_buffer_round_trips() {
        let mut m = init_mgr();
        cell_gcm_set_display_buffer(&mut m, 0, 0x1000, 1920 * 4, 1920, 1080).unwrap();
        let db = get_display_buffer(&m, 0).unwrap();
        assert_eq!(db.width, 1920);
        assert_eq!(db.height, 1080);
        assert_eq!(db.pitch, 1920 * 4);
    }

    #[test]
    fn set_display_buffer_rejects_id_out_of_range() {
        let mut m = init_mgr();
        assert_eq!(
            cell_gcm_set_display_buffer(&mut m, 8, 0, 1, 1, 1).unwrap_err(),
            errors::INVALID_VALUE,
        );
    }

    #[test]
    fn set_display_buffer_rejects_zero_dims() {
        let mut m = init_mgr();
        assert_eq!(
            cell_gcm_set_display_buffer(&mut m, 0, 0, 0, 100, 100).unwrap_err(),
            errors::INVALID_VALUE,
        );
    }

    // --- address translation --------------------------------------

    #[test]
    fn address_to_offset_inside_local_memory() {
        let m = init_mgr();
        let addr = DEFAULT_LOCAL_ADDR + 0x10_0000;
        let off = cell_gcm_address_to_offset(&m, addr).unwrap();
        assert_eq!(off, 0x10_0000);
    }

    #[test]
    fn address_to_offset_inside_io_region() {
        let m = init_mgr();
        let addr = DEFAULT_IO_ADDR + 0x200_000 + 0x1234;
        let off = cell_gcm_address_to_offset(&m, addr).unwrap();
        assert_eq!(off, 0x200_000 + 0x1234);
    }

    #[test]
    fn address_to_offset_miss_returns_failure() {
        let m = init_mgr();
        assert_eq!(
            cell_gcm_address_to_offset(&m, 0x1000).unwrap_err(),
            errors::FAILURE,
        );
    }

    #[test]
    fn io_offset_to_address_round_trips() {
        let m = init_mgr();
        let addr = DEFAULT_IO_ADDR + 0x300_000 + 0xABCD;
        let off = cell_gcm_address_to_offset(&m, addr).unwrap();
        let back = cell_gcm_io_offset_to_address(&m, off).unwrap();
        assert_eq!(back, addr);
    }

    // --- tiles ----------------------------------------------------

    #[test]
    fn set_tile_info_round_trips() {
        let mut m = init_mgr();
        cell_gcm_set_tile_info(&mut m, 3, LOCATION_LOCAL, 0x1000, 0x10_000, 64 * 8, 0, 0, 0).unwrap();
        let t = get_tile(&m, 3).unwrap();
        assert_eq!(t.offset, 0x1000);
        assert_eq!(t.pitch, 64 * 8);
        assert!(!t.bound);
    }

    #[test]
    fn set_tile_info_rejects_bad_location() {
        let mut m = init_mgr();
        assert_eq!(
            cell_gcm_set_tile_info(&mut m, 0, 99, 0, 0, 64, 0, 0, 0).unwrap_err(),
            errors::INVALID_ENUM,
        );
    }

    #[test]
    fn set_tile_info_rejects_bad_pitch_alignment() {
        let mut m = init_mgr();
        assert_eq!(
            cell_gcm_set_tile_info(&mut m, 0, LOCATION_LOCAL, 0, 0, 63, 0, 0, 0).unwrap_err(),
            errors::INVALID_ALIGNMENT,
        );
    }

    #[test]
    fn bind_and_unbind_tile_flip_state() {
        let mut m = init_mgr();
        cell_gcm_set_tile_info(&mut m, 2, LOCATION_LOCAL, 0, 0x1000, 64, 0, 0, 0).unwrap();
        cell_gcm_bind_tile(&mut m, 2).unwrap();
        assert!(get_tile(&m, 2).unwrap().bound);
        cell_gcm_unbind_tile(&mut m, 2).unwrap();
        assert!(!get_tile(&m, 2).unwrap().bound);
    }

    #[test]
    fn bind_out_of_range_tile_is_invalid_value() {
        let mut m = init_mgr();
        assert_eq!(
            cell_gcm_bind_tile(&mut m, 99).unwrap_err(),
            errors::INVALID_VALUE,
        );
    }

    // --- zculls ---------------------------------------------------

    #[test]
    fn set_zcull_marks_bound() {
        let mut m = init_mgr();
        cell_gcm_set_zcull(&mut m, 1, ZcullInfo::default()).unwrap();
        assert!(get_zcull(&m, 1).unwrap().bound);
    }

    #[test]
    fn set_zcull_out_of_range_is_invalid_value() {
        let mut m = init_mgr();
        assert_eq!(
            cell_gcm_set_zcull(&mut m, 99, ZcullInfo::default()).unwrap_err(),
            errors::INVALID_VALUE,
        );
    }

    // --- flip mode ------------------------------------------------

    #[test]
    fn set_flip_mode_accepts_hsync_and_vsync() {
        let mut m = init_mgr();
        cell_gcm_set_flip_mode(&mut m, FLIP_MODE_HSYNC).unwrap();
        assert_eq!(get_flip_mode(&m), FLIP_MODE_HSYNC);
        cell_gcm_set_flip_mode(&mut m, FLIP_MODE_VSYNC).unwrap();
        assert_eq!(get_flip_mode(&m), FLIP_MODE_VSYNC);
    }

    #[test]
    fn set_flip_mode_rejects_unknown() {
        let mut m = init_mgr();
        assert_eq!(
            cell_gcm_set_flip_mode(&mut m, 99).unwrap_err(),
            errors::INVALID_ENUM,
        );
    }

    // --- fields / reports -----------------------------------------

    #[test]
    fn current_field_toggles_top_bottom() {
        let mut m = init_mgr();
        assert_eq!(cell_gcm_get_current_field(&m), 0);
        advance_field(&mut m);
        assert_eq!(cell_gcm_get_current_field(&m), 1);
        advance_field(&mut m);
        assert_eq!(cell_gcm_get_current_field(&m), 0);
    }

    #[test]
    fn reports_round_trip() {
        let mut m = init_mgr();
        assert_eq!(cell_gcm_get_report(&m, 10), 0, "default is 0");
        cell_gcm_set_report(&mut m, 10, 42);
        assert_eq!(cell_gcm_get_report(&m, 10), 42);
    }
}
