//! `rpcs3-hle-cellvpost` — video post-processing HLE.
//!
//! Ports `rpcs3/Emu/Cell/Modules/cellVpost.cpp`. Games use cellVpost
//! after the video decoder (cellVdec) emits YUV frames, to convert
//! them to RGBA, deinterlace, and scale to the display resolution.
//! Our HLE manages the lifecycle (Open → Exec → Close) and config
//! queries; actual pixel conversion plugs in via [`VideoPostBackend`].
//!
//! ## Entry points covered
//!
//! | HLE function             | Rust wrapper               |
//! |--------------------------|----------------------------|
//! | `cellVpostQueryAttr`     | [`cell_vpost_query_attr`]  |
//! | `cellVpostOpen`          | [`cell_vpost_open`]        |
//! | `cellVpostOpenEx`        | [`cell_vpost_open_ex`]     |
//! | `cellVpostClose`         | [`cell_vpost_close`]       |
//! | `cellVpostExec`          | [`cell_vpost_exec`]        |

use rpcs3_emu_types::CellError;

// =====================================================================
// Error codes — byte-exact with cellVpost.h:22-52
// =====================================================================

pub mod errors {
    use rpcs3_emu_types::CellError;

    pub const Q_ARG_CFG_NULL: CellError = CellError(0x8061_0410);
    pub const Q_ARG_CFG_INVALID: CellError = CellError(0x8061_0411);
    pub const Q_ARG_ATTR_NULL: CellError = CellError(0x8061_0412);
    pub const O_ARG_CFG_NULL: CellError = CellError(0x8061_0440);
    pub const O_ARG_CFG_INVALID: CellError = CellError(0x8061_0441);
    pub const O_ARG_RSRC_NULL: CellError = CellError(0x8061_0442);
    pub const O_ARG_RSRC_INVALID: CellError = CellError(0x8061_0443);
    pub const O_ARG_HDL_NULL: CellError = CellError(0x8061_0444);
    pub const O_FATAL_QUERY_FAIL: CellError = CellError(0x8061_0460);
    pub const O_FATAL_CREATEMON_FAIL: CellError = CellError(0x8061_0461);
    pub const O_FATAL_INITSPURS_FAIL: CellError = CellError(0x8061_0462);
    pub const C_ARG_HDL_NULL: CellError = CellError(0x8061_0470);
    pub const C_ARG_HDL_INVALID: CellError = CellError(0x8061_0471);
    pub const E_ARG_HDL_NULL: CellError = CellError(0x8061_04A0);
    pub const E_ARG_HDL_INVALID: CellError = CellError(0x8061_04A1);
    pub const E_ARG_INPICBUF_NULL: CellError = CellError(0x8061_04A2);
    pub const E_ARG_INPICBUF_INVALID: CellError = CellError(0x8061_04A3);
    pub const E_ARG_CTRL_NULL: CellError = CellError(0x8061_04A4);
    pub const E_ARG_CTRL_INVALID: CellError = CellError(0x8061_04A5);
    pub const E_ARG_OUTPICBUF_NULL: CellError = CellError(0x8061_04A6);
    pub const E_ARG_OUTPICBUF_INVALID: CellError = CellError(0x8061_04A7);
    pub const E_ARG_PICINFO_NULL: CellError = CellError(0x8061_04A8);
}

// =====================================================================
// Enums (subset, byte-exact)
// =====================================================================

pub const PIC_DEPTH_8: i32 = 0;

pub const PIC_FMT_IN_YUV420_PLANAR: i32 = 0;

pub const PIC_FMT_OUT_RGBA_ILV: i32 = 0;
pub const PIC_FMT_OUT_YUV420_PLANAR: i32 = 1;

pub const PIC_STRUCT_PFRM: i32 = 0;
pub const PIC_STRUCT_IFRM: i32 = 1;
pub const PIC_STRUCT_ITOP: i32 = 2;
pub const PIC_STRUCT_IBTM: i32 = 3;

pub const EXEC_TYPE_PFRM_PFRM: i32 = 0;
pub const EXEC_TYPE_PTOP_ITOP: i32 = 1;
pub const EXEC_TYPE_PBTM_IBTM: i32 = 2;
pub const EXEC_TYPE_ITOP_PFRM: i32 = 3;
pub const EXEC_TYPE_IBTM_PFRM: i32 = 4;
pub const EXEC_TYPE_IFRM_IFRM: i32 = 5;
pub const EXEC_TYPE_ITOP_ITOP: i32 = 6;
pub const EXEC_TYPE_IBTM_IBTM: i32 = 7;

pub const CHROMA_POS_TYPE_A: i32 = 0;
pub const CHROMA_POS_TYPE_B: i32 = 1;

pub const SCAN_TYPE_P: i32 = 0;
pub const SCAN_TYPE_I: i32 = 1;

pub const QUANT_RANGE_FULL: i32 = 0;
pub const QUANT_RANGE_BROADCAST: i32 = 1;

pub const COLOR_MATRIX_BT601: i32 = 0;
pub const COLOR_MATRIX_BT709: i32 = 1;

pub const SCALER_TYPE_BILINEAR: i32 = 0;
pub const SCALER_TYPE_LINEAR_SHARP: i32 = 1;
pub const SCALER_TYPE_2X4TAP: i32 = 2;
pub const SCALER_TYPE_8X4TAP: i32 = 3;

pub const IPC_TYPE_DOUBLING: i32 = 0;
pub const IPC_TYPE_LINEAR: i32 = 1;
pub const IPC_TYPE_MAVG: i32 = 2;

// =====================================================================
// Data model
// =====================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CfgParam {
    pub in_max_width: u32,
    pub in_max_height: u32,
    pub in_depth: i32,
    pub out_max_width: u32,
    pub out_max_height: u32,
    pub out_depth: i32,
    pub out_fmt: i32,
}

impl Default for CfgParam {
    fn default() -> Self {
        Self {
            in_max_width: 1920,
            in_max_height: 1080,
            in_depth: PIC_DEPTH_8,
            out_max_width: 1920,
            out_max_height: 1080,
            out_depth: PIC_DEPTH_8,
            out_fmt: PIC_FMT_OUT_RGBA_ILV,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Attr {
    pub mem_size: u32,
    pub delay: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CtrlParam {
    pub exec_type: i32,
    pub scaler_type: i32,
    pub ipc_type: i32,
    pub in_width: u32,
    pub in_height: u32,
    pub in_window_x: u32,
    pub in_window_y: u32,
    pub in_window_w: u32,
    pub in_window_h: u32,
    pub out_width: u32,
    pub out_height: u32,
    pub out_window_x: u32,
    pub out_window_y: u32,
    pub out_window_w: u32,
    pub out_window_h: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PictureInfo {
    pub in_width: u32,
    pub in_height: u32,
    pub out_width: u32,
    pub out_height: u32,
    pub out_pitch: u32,
    pub processed_lines: u32,
}

// =====================================================================
// Backend trait
// =====================================================================

pub trait VideoPostBackend {
    fn exec(
        &mut self,
        ctrl: &CtrlParam,
        in_pic: &[u8],
        out_pic: &mut [u8],
    ) -> Result<PictureInfo, CellError>;
}

/// Stub backend — copies raw bytes over (size-matched).
#[derive(Debug, Default)]
pub struct StubVideoPostBackend;

impl VideoPostBackend for StubVideoPostBackend {
    fn exec(
        &mut self,
        ctrl: &CtrlParam,
        in_pic: &[u8],
        out_pic: &mut [u8],
    ) -> Result<PictureInfo, CellError> {
        if in_pic.is_empty() {
            return Err(errors::E_ARG_INPICBUF_INVALID);
        }
        if out_pic.is_empty() {
            return Err(errors::E_ARG_OUTPICBUF_INVALID);
        }
        let n = in_pic.len().min(out_pic.len());
        out_pic[..n].copy_from_slice(&in_pic[..n]);
        Ok(PictureInfo {
            in_width: ctrl.in_width,
            in_height: ctrl.in_height,
            out_width: ctrl.out_width,
            out_height: ctrl.out_height,
            out_pitch: ctrl.out_width * 4,
            processed_lines: ctrl.out_height,
        })
    }
}

// =====================================================================
// Manager
// =====================================================================

#[derive(Debug, Default)]
pub struct VpostManager {
    handles: std::collections::BTreeMap<u32, Handle>,
    next_handle: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Handle {
    cfg: CfgParam,
}

// =====================================================================
// Validation
// =====================================================================

fn validate_cfg(cfg: &CfgParam) -> Result<(), CellError> {
    if cfg.in_max_width == 0 || cfg.in_max_height == 0
        || cfg.out_max_width == 0 || cfg.out_max_height == 0
    {
        return Err(errors::O_ARG_CFG_INVALID);
    }
    if cfg.in_depth != PIC_DEPTH_8 || cfg.out_depth != PIC_DEPTH_8 {
        return Err(errors::O_ARG_CFG_INVALID);
    }
    match cfg.out_fmt {
        PIC_FMT_OUT_RGBA_ILV | PIC_FMT_OUT_YUV420_PLANAR => Ok(()),
        _ => Err(errors::O_ARG_CFG_INVALID),
    }
}

fn validate_ctrl(ctrl: &CtrlParam) -> Result<(), CellError> {
    if ctrl.in_width == 0 || ctrl.in_height == 0
        || ctrl.out_width == 0 || ctrl.out_height == 0
    {
        return Err(errors::E_ARG_CTRL_INVALID);
    }
    if !matches!(
        ctrl.exec_type,
        EXEC_TYPE_PFRM_PFRM | EXEC_TYPE_PTOP_ITOP | EXEC_TYPE_PBTM_IBTM
        | EXEC_TYPE_ITOP_PFRM | EXEC_TYPE_IBTM_PFRM | EXEC_TYPE_IFRM_IFRM
        | EXEC_TYPE_ITOP_ITOP | EXEC_TYPE_IBTM_IBTM,
    ) {
        return Err(errors::E_ARG_CTRL_INVALID);
    }
    if !matches!(
        ctrl.scaler_type,
        SCALER_TYPE_BILINEAR | SCALER_TYPE_LINEAR_SHARP
        | SCALER_TYPE_2X4TAP | SCALER_TYPE_8X4TAP,
    ) {
        return Err(errors::E_ARG_CTRL_INVALID);
    }
    if !matches!(
        ctrl.ipc_type,
        IPC_TYPE_DOUBLING | IPC_TYPE_LINEAR | IPC_TYPE_MAVG,
    ) {
        return Err(errors::E_ARG_CTRL_INVALID);
    }
    if ctrl.in_window_x + ctrl.in_window_w > ctrl.in_width
        || ctrl.in_window_y + ctrl.in_window_h > ctrl.in_height
    {
        return Err(errors::E_ARG_CTRL_INVALID);
    }
    if ctrl.out_window_x + ctrl.out_window_w > ctrl.out_width
        || ctrl.out_window_y + ctrl.out_window_h > ctrl.out_height
    {
        return Err(errors::E_ARG_CTRL_INVALID);
    }
    Ok(())
}

// =====================================================================
// Syscalls
// =====================================================================

/// `cellVpostQueryAttr(cfg, attr_out)`.
#[must_use]
pub fn cell_vpost_query_attr(cfg: &CfgParam) -> Result<Attr, CellError> {
    validate_cfg(cfg)?;
    // Memory size heuristic matches C++: max-in + max-out + ~8 MB overhead.
    let in_bytes = cfg.in_max_width * cfg.in_max_height * 2;  // YUV420 1.5x, padded
    let out_bpp = match cfg.out_fmt {
        PIC_FMT_OUT_RGBA_ILV => 4,
        PIC_FMT_OUT_YUV420_PLANAR => 2,
        _ => return Err(errors::Q_ARG_CFG_INVALID),
    };
    let out_bytes = cfg.out_max_width * cfg.out_max_height * out_bpp;
    Ok(Attr {
        mem_size: in_bytes + out_bytes + 0x80_0000,
        delay: 16000,  // typical, in microseconds
    })
}

/// `cellVpostOpen(cfg, rsrc, handle_out)`.
#[must_use]
pub fn cell_vpost_open(m: &mut VpostManager, cfg: CfgParam) -> Result<u32, CellError> {
    validate_cfg(&cfg)?;
    m.next_handle += 1;
    let h = m.next_handle;
    m.handles.insert(h, Handle { cfg });
    Ok(h)
}

/// `cellVpostOpenEx(cfg, rsrc_ex, handle_out)` — same semantics in our port.
#[must_use]
pub fn cell_vpost_open_ex(m: &mut VpostManager, cfg: CfgParam) -> Result<u32, CellError> {
    cell_vpost_open(m, cfg)
}

/// `cellVpostClose(handle)`.
#[must_use]
pub fn cell_vpost_close(m: &mut VpostManager, handle: u32) -> Result<(), CellError> {
    if m.handles.remove(&handle).is_none() {
        return Err(errors::C_ARG_HDL_INVALID);
    }
    Ok(())
}

/// `cellVpostExec(handle, in_pic, ctrl, out_pic, pic_info_out)`.
#[must_use]
pub fn cell_vpost_exec<B: VideoPostBackend + ?Sized>(
    m: &VpostManager,
    backend: &mut B,
    handle: u32,
    in_pic: &[u8],
    ctrl: &CtrlParam,
    out_pic: &mut [u8],
) -> Result<PictureInfo, CellError> {
    if !m.handles.contains_key(&handle) {
        return Err(errors::E_ARG_HDL_INVALID);
    }
    if in_pic.is_empty() {
        return Err(errors::E_ARG_INPICBUF_NULL);
    }
    if out_pic.is_empty() {
        return Err(errors::E_ARG_OUTPICBUF_NULL);
    }
    validate_ctrl(ctrl)?;
    backend.exec(ctrl, in_pic, out_pic)
}

// =====================================================================
// Tests
// =====================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn default_ctrl() -> CtrlParam {
        CtrlParam {
            exec_type: EXEC_TYPE_PFRM_PFRM,
            scaler_type: SCALER_TYPE_BILINEAR,
            ipc_type: IPC_TYPE_LINEAR,
            in_width: 1280,
            in_height: 720,
            in_window_x: 0, in_window_y: 0, in_window_w: 1280, in_window_h: 720,
            out_width: 1920,
            out_height: 1080,
            out_window_x: 0, out_window_y: 0, out_window_w: 1920, out_window_h: 1080,
        }
    }

    // --- constants ------------------------------------------------

    #[test]
    fn error_codes_byte_exact() {
        assert_eq!(errors::Q_ARG_CFG_NULL.0, 0x8061_0410);
        assert_eq!(errors::O_ARG_CFG_INVALID.0, 0x8061_0441);
        assert_eq!(errors::C_ARG_HDL_INVALID.0, 0x8061_0471);
        assert_eq!(errors::E_ARG_INPICBUF_INVALID.0, 0x8061_04A3);
        assert_eq!(errors::E_ARG_PICINFO_NULL.0, 0x8061_04A8);
    }

    #[test]
    fn enum_constants_stable() {
        assert_eq!(PIC_FMT_OUT_RGBA_ILV, 0);
        assert_eq!(PIC_FMT_OUT_YUV420_PLANAR, 1);
        assert_eq!(SCALER_TYPE_BILINEAR, 0);
        assert_eq!(SCALER_TYPE_8X4TAP, 3);
        assert_eq!(COLOR_MATRIX_BT709, 1);
    }

    // --- query ----------------------------------------------------

    #[test]
    fn query_attr_happy_path() {
        let attr = cell_vpost_query_attr(&CfgParam::default()).unwrap();
        assert!(attr.mem_size >= 1920 * 1080 * 4);
    }

    #[test]
    fn query_attr_zero_dims_rejected() {
        let cfg = CfgParam { in_max_width: 0, ..CfgParam::default() };
        assert_eq!(
            cell_vpost_query_attr(&cfg).unwrap_err(),
            errors::O_ARG_CFG_INVALID,
        );
    }

    #[test]
    fn query_attr_bad_out_fmt_rejected() {
        let cfg = CfgParam { out_fmt: 99, ..CfgParam::default() };
        assert_eq!(
            cell_vpost_query_attr(&cfg).unwrap_err(),
            errors::O_ARG_CFG_INVALID,
        );
    }

    // --- open / close ---------------------------------------------

    #[test]
    fn open_allocates_handle() {
        let mut m = VpostManager::default();
        let h = cell_vpost_open(&mut m, CfgParam::default()).unwrap();
        assert_eq!(h, 1);
    }

    #[test]
    fn open_ex_is_same_semantics() {
        let mut m = VpostManager::default();
        cell_vpost_open_ex(&mut m, CfgParam::default()).unwrap();
        assert_eq!(m.handles.len(), 1);
    }

    #[test]
    fn open_with_invalid_cfg_fails() {
        let mut m = VpostManager::default();
        let cfg = CfgParam { out_max_height: 0, ..CfgParam::default() };
        assert_eq!(
            cell_vpost_open(&mut m, cfg).unwrap_err(),
            errors::O_ARG_CFG_INVALID,
        );
    }

    #[test]
    fn close_unknown_handle_is_invalid() {
        let mut m = VpostManager::default();
        assert_eq!(
            cell_vpost_close(&mut m, 999).unwrap_err(),
            errors::C_ARG_HDL_INVALID,
        );
    }

    #[test]
    fn close_releases_handle() {
        let mut m = VpostManager::default();
        let h = cell_vpost_open(&mut m, CfgParam::default()).unwrap();
        cell_vpost_close(&mut m, h).unwrap();
        assert_eq!(m.handles.len(), 0);
    }

    // --- exec -----------------------------------------------------

    #[test]
    fn exec_unknown_handle_is_invalid() {
        let m = VpostManager::default();
        let mut b = StubVideoPostBackend;
        let in_pic = vec![0u8; 128];
        let mut out_pic = vec![0u8; 128];
        let ctrl = default_ctrl();
        assert_eq!(
            cell_vpost_exec(&m, &mut b, 999, &in_pic, &ctrl, &mut out_pic).unwrap_err(),
            errors::E_ARG_HDL_INVALID,
        );
    }

    #[test]
    fn exec_empty_in_buffer_is_inpicbuf_null() {
        let mut m = VpostManager::default();
        let h = cell_vpost_open(&mut m, CfgParam::default()).unwrap();
        let mut b = StubVideoPostBackend;
        let mut out_pic = vec![0u8; 128];
        let ctrl = default_ctrl();
        assert_eq!(
            cell_vpost_exec(&m, &mut b, h, &[], &ctrl, &mut out_pic).unwrap_err(),
            errors::E_ARG_INPICBUF_NULL,
        );
    }

    #[test]
    fn exec_empty_out_buffer_is_outpicbuf_null() {
        let mut m = VpostManager::default();
        let h = cell_vpost_open(&mut m, CfgParam::default()).unwrap();
        let mut b = StubVideoPostBackend;
        let in_pic = vec![0u8; 128];
        let mut out_pic = Vec::new();
        let ctrl = default_ctrl();
        assert_eq!(
            cell_vpost_exec(&m, &mut b, h, &in_pic, &ctrl, &mut out_pic).unwrap_err(),
            errors::E_ARG_OUTPICBUF_NULL,
        );
    }

    #[test]
    fn exec_bad_scaler_type_is_ctrl_invalid() {
        let mut m = VpostManager::default();
        let h = cell_vpost_open(&mut m, CfgParam::default()).unwrap();
        let mut b = StubVideoPostBackend;
        let in_pic = vec![0u8; 128];
        let mut out_pic = vec![0u8; 128];
        let ctrl = CtrlParam { scaler_type: 99, ..default_ctrl() };
        assert_eq!(
            cell_vpost_exec(&m, &mut b, h, &in_pic, &ctrl, &mut out_pic).unwrap_err(),
            errors::E_ARG_CTRL_INVALID,
        );
    }

    #[test]
    fn exec_window_overflow_is_ctrl_invalid() {
        let mut m = VpostManager::default();
        let h = cell_vpost_open(&mut m, CfgParam::default()).unwrap();
        let mut b = StubVideoPostBackend;
        let in_pic = vec![0u8; 128];
        let mut out_pic = vec![0u8; 128];
        // Window extends past in_width.
        let ctrl = CtrlParam { in_window_x: 100, in_window_w: 9999, ..default_ctrl() };
        assert_eq!(
            cell_vpost_exec(&m, &mut b, h, &in_pic, &ctrl, &mut out_pic).unwrap_err(),
            errors::E_ARG_CTRL_INVALID,
        );
    }

    #[test]
    fn exec_happy_path_copies_bytes_and_returns_pic_info() {
        let mut m = VpostManager::default();
        let h = cell_vpost_open(&mut m, CfgParam::default()).unwrap();
        let mut b = StubVideoPostBackend;
        let in_pic = (0..128).map(|i| i as u8).collect::<Vec<_>>();
        let mut out_pic = vec![0u8; 128];
        let ctrl = default_ctrl();
        let info = cell_vpost_exec(&m, &mut b, h, &in_pic, &ctrl, &mut out_pic).unwrap();
        assert_eq!(info.out_width, 1920);
        assert_eq!(info.out_pitch, 1920 * 4);
        assert_eq!(&out_pic[..4], &[0, 1, 2, 3]);
    }

    #[test]
    fn full_lifecycle_open_exec_close() {
        let mut m = VpostManager::default();
        let h = cell_vpost_open(&mut m, CfgParam::default()).unwrap();
        let mut b = StubVideoPostBackend;
        let in_pic = vec![0x80u8; 1280 * 720 * 3 / 2];
        let mut out_pic = vec![0u8; 1920 * 1080 * 4];
        let ctrl = default_ctrl();
        cell_vpost_exec(&m, &mut b, h, &in_pic, &ctrl, &mut out_pic).unwrap();
        cell_vpost_close(&mut m, h).unwrap();
    }

    #[test]
    fn multiple_handles_independent() {
        let mut m = VpostManager::default();
        let h1 = cell_vpost_open(&mut m, CfgParam::default()).unwrap();
        let h2 = cell_vpost_open(&mut m, CfgParam::default()).unwrap();
        assert_ne!(h1, h2);
        cell_vpost_close(&mut m, h1).unwrap();
        assert!(m.handles.contains_key(&h2));
    }

    #[test]
    fn ctrl_bad_exec_type_is_invalid() {
        let mut m = VpostManager::default();
        let h = cell_vpost_open(&mut m, CfgParam::default()).unwrap();
        let mut b = StubVideoPostBackend;
        let in_pic = vec![0u8; 128];
        let mut out_pic = vec![0u8; 128];
        let ctrl = CtrlParam { exec_type: 99, ..default_ctrl() };
        assert_eq!(
            cell_vpost_exec(&m, &mut b, h, &in_pic, &ctrl, &mut out_pic).unwrap_err(),
            errors::E_ARG_CTRL_INVALID,
        );
    }

    #[test]
    fn ctrl_bad_ipc_type_is_invalid() {
        let mut m = VpostManager::default();
        let h = cell_vpost_open(&mut m, CfgParam::default()).unwrap();
        let mut b = StubVideoPostBackend;
        let in_pic = vec![0u8; 128];
        let mut out_pic = vec![0u8; 128];
        let ctrl = CtrlParam { ipc_type: 99, ..default_ctrl() };
        assert_eq!(
            cell_vpost_exec(&m, &mut b, h, &in_pic, &ctrl, &mut out_pic).unwrap_err(),
            errors::E_ARG_CTRL_INVALID,
        );
    }
}
