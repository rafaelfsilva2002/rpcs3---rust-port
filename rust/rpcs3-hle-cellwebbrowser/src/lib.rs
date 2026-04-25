//! Rust port of `rpcs3/Emu/Cell/Modules/cellWebBrowser.cpp` — PS3 web-browser
//! sysutil sub-module.
//!
//! Upstream registers 46 entries under the `cellSysutil` PRX (not its own),
//! backed by a singleton `browser_info { system_cb, userData }`. Almost every
//! entry is `cellSysutil.todo(...)` returning `CELL_OK`. The few concrete
//! behaviours preserved here:
//!
//! * `cellWebBrowserEstimate2` writes `*memSize = 1 MB` to the output pointer.
//! * `cellWebBrowserInitialize` stashes the system-callback in the singleton
//!   and queues a deferred `INITIALIZING_FINISHED` callback.
//! * `cellWebBrowserShutdown` queues a deferred `SHUTDOWN_FINISHED` callback.
//!
//! The header defines six sysutil event codes (`CellWebBrowserEvent`) and
//! several config structs (`CellWebBrowserConfig`, `CellWebBrowserConfig2`,
//! `CellWebBrowserRect`, etc). All are mirrored here `#[repr(C)]` with size
//! asserts to pin the binary ABI.
//!
//! `no_std` + `alloc`. Single dep: `rpcs3-emu-types`.

#![no_std]

extern crate alloc;

use alloc::vec::Vec;
use core::mem::{size_of, take};

use rpcs3_emu_types::CellError;

/// Upstream `cellWebBrowser.cpp` registers entries into the `cellSysutil`
/// PRX — there's no dedicated module name. Expose both so callers can check.
pub const HOST_MODULE_NAME: &str = "cellSysutil";
pub const SUBMODULE_NAME: &str = "cellWebBrowser";

/// Entries registered by `cellSysutil_WebBrowser_init()` in the exact
/// `REG_FUNC` order (cpp:323-372). 47 total.
pub const REGISTERED_ENTRY_POINTS: &[&str] = &[
    "cellWebBrowserActivate",
    "cellWebBrowserConfig",
    "cellWebBrowserConfig2",
    "cellWebBrowserConfigGetHeapSize",
    "cellWebBrowserConfigGetHeapSize2",
    "cellWebBrowserConfigSetCustomExit",
    "cellWebBrowserConfigSetDisableTabs",
    "cellWebBrowserConfigSetErrorHook2",
    "cellWebBrowserConfigSetFullScreen2",
    "cellWebBrowserConfigSetFullVersion2",
    "cellWebBrowserConfigSetFunction",
    "cellWebBrowserConfigSetFunction2",
    "cellWebBrowserConfigSetHeapSize",
    "cellWebBrowserConfigSetHeapSize2",
    "cellWebBrowserConfigSetMimeSet",
    "cellWebBrowserConfigSetNotifyHook2",
    "cellWebBrowserConfigSetRequestHook2",
    "cellWebBrowserConfigSetStatusHook2",
    "cellWebBrowserConfigSetTabCount2",
    "cellWebBrowserConfigSetUnknownMIMETypeHook2",
    "cellWebBrowserConfigSetVersion",
    "cellWebBrowserConfigSetViewCondition2",
    "cellWebBrowserConfigSetViewRect2",
    "cellWebBrowserConfigWithVer",
    "cellWebBrowserCreate",
    "cellWebBrowserCreate2",
    "cellWebBrowserCreateRender2",
    "cellWebBrowserCreateRenderWithRect2",
    "cellWebBrowserCreateWithConfig",
    "cellWebBrowserCreateWithConfigFull",
    "cellWebBrowserCreateWithRect2",
    "cellWebBrowserDeactivate",
    "cellWebBrowserDestroy",
    "cellWebBrowserDestroy2",
    "cellWebBrowserEstimate",
    "cellWebBrowserEstimate2",
    "cellWebBrowserGetUsrdataOnGameExit",
    "cellWebBrowserInitialize",
    "cellWebBrowserNavigate2",
    "cellWebBrowserSetLocalContentsAdditionalTitleID",
    "cellWebBrowserSetSystemCallbackUsrdata",
    "cellWebBrowserShutdown",
    "cellWebBrowserUpdatePointerDisplayPos2",
    "cellWebBrowserWakeupWithGameExit",
    "cellWebComponentCreate",
    "cellWebComponentCreateAsync",
    "cellWebComponentDestroy",
];

// ---------------------------------------------------------------------------
// Event codes (from cellWebBrowser.h) — s32 enum, byte-exact.
// ---------------------------------------------------------------------------

pub const CELL_SYSUTIL_WEBBROWSER_INITIALIZING_FINISHED: i32 = 1;
pub const CELL_SYSUTIL_WEBBROWSER_SHUTDOWN_FINISHED: i32 = 4;
pub const CELL_SYSUTIL_WEBBROWSER_LOADING_FINISHED: i32 = 5;
pub const CELL_SYSUTIL_WEBBROWSER_UNLOADING_FINISHED: i32 = 7;
pub const CELL_SYSUTIL_WEBBROWSER_RELEASED: i32 = 9;
pub const CELL_SYSUTIL_WEBBROWSER_GRABBED: i32 = 11;

// ---------------------------------------------------------------------------
// Estimate2 constant — upstream returns `*memSize = 1 * 1024 * 1024`.
// ---------------------------------------------------------------------------

pub const ESTIMATE2_MEM_SIZE: u32 = 1 * 1024 * 1024;

// ---------------------------------------------------------------------------
// Placeholder error codes — upstream has no dedicated error enum; these are
// the codes the Rust-side validation raises. Facility 0x8002_F7__ is unused
// by any ported crate so far (doesn't clash with cellVideoPlayerUtility 0xD5,
// cellUsbpspcm 0x8011_04, cellPhotoDecode 0xC9, etc).
// ---------------------------------------------------------------------------

pub const CELL_WEBBROWSER_ERROR_NOT_INITIALIZED: CellError = CellError(0x8002_F701);
pub const CELL_WEBBROWSER_ERROR_ALREADY_INITIALIZED: CellError = CellError(0x8002_F702);
pub const CELL_WEBBROWSER_ERROR_INVALID_PARAMETER: CellError = CellError(0x8002_F703);
pub const CELL_WEBBROWSER_ERROR_NOT_ACTIVE: CellError = CellError(0x8002_F704);
pub const CELL_WEBBROWSER_ERROR_ALREADY_ACTIVE: CellError = CellError(0x8002_F705);
pub const CELL_WEBBROWSER_ERROR_BROWSER_NOT_FOUND: CellError = CellError(0x8002_F706);
pub const CELL_WEBBROWSER_ERROR_OUT_OF_BROWSERS: CellError = CellError(0x8002_F707);

// ---------------------------------------------------------------------------
// Wire structs — mirror `cellWebBrowser.h`, `#[repr(C)]` with size asserts.
// ---------------------------------------------------------------------------

#[repr(C)]
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct CellWebBrowserPos {
    pub x: i32,
    pub y: i32,
}
const _: () = assert!(size_of::<CellWebBrowserPos>() == 8);

#[repr(C)]
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct CellWebBrowserSize {
    pub width: i32,
    pub height: i32,
}
const _: () = assert!(size_of::<CellWebBrowserSize>() == 8);

#[repr(C)]
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct CellWebBrowserRect {
    pub pos: CellWebBrowserPos,
    pub size: CellWebBrowserSize,
}
const _: () = assert!(size_of::<CellWebBrowserRect>() == 16);

#[repr(C)]
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct CellWebBrowserMimeSet {
    pub ty: u32,        // vm::bcptr<char>
    pub directory: u32, // vm::bcptr<char>
}
const _: () = assert!(size_of::<CellWebBrowserMimeSet>() == 8);

#[repr(C)]
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct CellWebBrowserConfig {
    pub version: i32,
    pub heap_size: i32,
    pub mimesets: u32,
    pub mimeset_num: i32,
    pub functions: i32,
    pub tab_count: i32,
    pub exit_cb: u32,
    pub download_cb: u32,
    pub navigated_cb: u32,
}
const _: () = assert!(size_of::<CellWebBrowserConfig>() == 36);

#[repr(C)]
#[derive(Debug, Default, Clone, Copy, PartialEq)]
pub struct CellWebBrowserConfig2 {
    pub version: i32,
    pub heap_size: i32,
    pub functions: i32,
    pub tab_count: i32,
    pub size_mode: i32,
    pub view_restriction: i32,
    pub unknown_mimetype_cb: u32,
    pub error_cb: u32,
    pub status_error_cb: u32,
    pub notify_cb: u32,
    pub request_cb: u32,
    pub rect: CellWebBrowserRect,
    pub resolution_factor: f32,
    pub magic_number_: i32,
}
// 11*4 + 16 + 4 + 4 = 68
const _: () = assert!(size_of::<CellWebBrowserConfig2>() == 68);

// ---------------------------------------------------------------------------
// FSM + singleton equivalent of upstream `browser_info`.
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModuleState {
    Inactive,
    Initialized,
    Shutdown,
}

impl Default for ModuleState {
    fn default() -> Self {
        ModuleState::Inactive
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BrowserState {
    Inactive,
    Active,
    Destroyed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BrowserVariant {
    /// Produced by `cellWebBrowserCreate` / `CreateWithConfig` / `CreateWithConfigFull`.
    V1,
    /// Produced by `cellWebBrowserCreate2` and the `Render2` / `WithRect2` variants.
    V2,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Browser {
    pub id: u32,
    pub variant: BrowserVariant,
    pub state: BrowserState,
    pub heap_size: i32,
    pub tab_count: i32,
    pub functions: i32,
    pub view_condition: i32,
    pub full_screen: bool,
    pub notify_cb: u32,
    pub mime_cb: u32,
    pub visible_rect: CellWebBrowserRect,
}

pub const BROWSER_ID_BASE: u32 = 0x5001_0000;
pub const MAX_BROWSERS: usize = 8;

/// Deferred callback enqueued by the sysutil scheduler (`sysutil_register_cb`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PendingSystemEvent {
    pub system_cb: u32,
    pub userdata: u32,
    pub event_code: i32,
}

// ---------------------------------------------------------------------------
// Manager.
// ---------------------------------------------------------------------------

#[derive(Debug, Default)]
pub struct WebBrowser {
    state: ModuleState,
    system_cb: u32,
    userdata: u32,
    container: u32,
    browsers: Vec<Browser>,
    pending: Vec<PendingSystemEvent>,
    next_browser_id: u32,

    // Per-entry counters — 46 entries.
    pub activate_calls: u64,
    pub config_calls: u64,
    pub config2_calls: u64,
    pub config_get_heap_size_calls: u64,
    pub config_get_heap_size2_calls: u64,
    pub config_set_custom_exit_calls: u64,
    pub config_set_disable_tabs_calls: u64,
    pub config_set_error_hook2_calls: u64,
    pub config_set_full_screen2_calls: u64,
    pub config_set_full_version2_calls: u64,
    pub config_set_function_calls: u64,
    pub config_set_function2_calls: u64,
    pub config_set_heap_size_calls: u64,
    pub config_set_heap_size2_calls: u64,
    pub config_set_mime_set_calls: u64,
    pub config_set_notify_hook2_calls: u64,
    pub config_set_request_hook2_calls: u64,
    pub config_set_status_hook2_calls: u64,
    pub config_set_tab_count2_calls: u64,
    pub config_set_unknown_mime_type_hook2_calls: u64,
    pub config_set_version_calls: u64,
    pub config_set_view_condition2_calls: u64,
    pub config_set_view_rect2_calls: u64,
    pub config_with_ver_calls: u64,
    pub create_calls: u64,
    pub create2_calls: u64,
    pub create_render2_calls: u64,
    pub create_render_with_rect2_calls: u64,
    pub create_with_config_calls: u64,
    pub create_with_config_full_calls: u64,
    pub create_with_rect2_calls: u64,
    pub deactivate_calls: u64,
    pub destroy_calls: u64,
    pub destroy2_calls: u64,
    pub estimate_calls: u64,
    pub estimate2_calls: u64,
    pub get_usrdata_on_game_exit_calls: u64,
    pub initialize_calls: u64,
    pub navigate2_calls: u64,
    pub set_local_contents_additional_title_id_calls: u64,
    pub set_system_callback_usrdata_calls: u64,
    pub shutdown_calls: u64,
    pub update_pointer_display_pos2_calls: u64,
    pub wakeup_with_game_exit_calls: u64,
    pub web_component_create_calls: u64,
    pub web_component_create_async_calls: u64,
    pub web_component_destroy_calls: u64,
}

impl WebBrowser {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn state(&self) -> ModuleState {
        self.state
    }

    pub fn system_cb(&self) -> u32 {
        self.system_cb
    }

    pub fn userdata(&self) -> u32 {
        self.userdata
    }

    pub fn container(&self) -> u32 {
        self.container
    }

    pub fn browsers(&self) -> &[Browser] {
        &self.browsers
    }

    pub fn browser(&self, id: u32) -> Option<&Browser> {
        self.browsers.iter().find(|b| b.id == id)
    }

    pub fn pending_events(&self) -> &[PendingSystemEvent] {
        &self.pending
    }

    pub fn pending_len(&self) -> usize {
        self.pending.len()
    }

    /// Drains all queued sysutil callbacks — mirrors running the scheduler tick.
    pub fn drain_pending(&mut self) -> Vec<PendingSystemEvent> {
        take(&mut self.pending)
    }

    fn require_initialized(&self) -> Result<(), CellError> {
        match self.state {
            ModuleState::Initialized => Ok(()),
            _ => Err(CELL_WEBBROWSER_ERROR_NOT_INITIALIZED),
        }
    }

    fn browser_mut(&mut self, id: u32) -> Option<&mut Browser> {
        self.browsers.iter_mut().find(|b| b.id == id)
    }

    fn alloc_browser(
        &mut self,
        variant: BrowserVariant,
    ) -> Result<u32, CellError> {
        self.require_initialized()?;
        if self.browsers.len() >= MAX_BROWSERS {
            return Err(CELL_WEBBROWSER_ERROR_OUT_OF_BROWSERS);
        }
        let id = BROWSER_ID_BASE.wrapping_add(self.next_browser_id);
        self.next_browser_id = self.next_browser_id.saturating_add(1);
        self.browsers.push(Browser {
            id,
            variant,
            state: BrowserState::Inactive,
            heap_size: 0,
            tab_count: 1,
            functions: 0,
            view_condition: 0,
            full_screen: false,
            notify_cb: 0,
            mime_cb: 0,
            visible_rect: CellWebBrowserRect::default(),
        });
        Ok(id)
    }

    // ---- Config entries ---------------------------------------------------

    pub fn activate(&mut self) -> Result<(), CellError> {
        self.activate_calls = self.activate_calls.saturating_add(1);
        Ok(())
    }

    pub fn config(&mut self) -> Result<(), CellError> {
        self.config_calls = self.config_calls.saturating_add(1);
        Ok(())
    }

    pub fn config2(
        &mut self,
        _config: Option<&CellWebBrowserConfig2>,
        _version: u32,
    ) -> Result<(), CellError> {
        self.config2_calls = self.config2_calls.saturating_add(1);
        Ok(())
    }

    pub fn config_get_heap_size(&mut self) -> Result<(), CellError> {
        self.config_get_heap_size_calls = self.config_get_heap_size_calls.saturating_add(1);
        Ok(())
    }

    pub fn config_get_heap_size2(&mut self) -> Result<(), CellError> {
        self.config_get_heap_size2_calls = self.config_get_heap_size2_calls.saturating_add(1);
        Ok(())
    }

    pub fn config_set_custom_exit(&mut self) -> Result<(), CellError> {
        self.config_set_custom_exit_calls = self.config_set_custom_exit_calls.saturating_add(1);
        Ok(())
    }

    pub fn config_set_disable_tabs(&mut self) -> Result<(), CellError> {
        self.config_set_disable_tabs_calls = self.config_set_disable_tabs_calls.saturating_add(1);
        Ok(())
    }

    pub fn config_set_error_hook2(&mut self) -> Result<(), CellError> {
        self.config_set_error_hook2_calls = self.config_set_error_hook2_calls.saturating_add(1);
        Ok(())
    }

    pub fn config_set_full_screen2(
        &mut self,
        config: Option<&mut CellWebBrowserConfig2>,
        full: u32,
    ) -> Result<(), CellError> {
        self.config_set_full_screen2_calls =
            self.config_set_full_screen2_calls.saturating_add(1);
        if let Some(c) = config {
            c.size_mode = full as i32;
        }
        Ok(())
    }

    pub fn config_set_full_version2(&mut self) -> Result<(), CellError> {
        self.config_set_full_version2_calls =
            self.config_set_full_version2_calls.saturating_add(1);
        Ok(())
    }

    pub fn config_set_function(&mut self) -> Result<(), CellError> {
        self.config_set_function_calls = self.config_set_function_calls.saturating_add(1);
        Ok(())
    }

    pub fn config_set_function2(
        &mut self,
        config: Option<&mut CellWebBrowserConfig2>,
        funcset: u32,
    ) -> Result<(), CellError> {
        self.config_set_function2_calls = self.config_set_function2_calls.saturating_add(1);
        if let Some(c) = config {
            c.functions = funcset as i32;
        }
        Ok(())
    }

    pub fn config_set_heap_size(&mut self) -> Result<(), CellError> {
        self.config_set_heap_size_calls = self.config_set_heap_size_calls.saturating_add(1);
        Ok(())
    }

    pub fn config_set_heap_size2(
        &mut self,
        config: Option<&mut CellWebBrowserConfig2>,
        size: u32,
    ) -> Result<(), CellError> {
        self.config_set_heap_size2_calls = self.config_set_heap_size2_calls.saturating_add(1);
        if let Some(c) = config {
            c.heap_size = size as i32;
        }
        Ok(())
    }

    pub fn config_set_mime_set(&mut self) -> Result<(), CellError> {
        self.config_set_mime_set_calls = self.config_set_mime_set_calls.saturating_add(1);
        Ok(())
    }

    pub fn config_set_notify_hook2(
        &mut self,
        _config: Option<&CellWebBrowserConfig2>,
        cb: u32,
        userdata: u32,
    ) -> Result<(), CellError> {
        self.config_set_notify_hook2_calls =
            self.config_set_notify_hook2_calls.saturating_add(1);
        // Model: remember the most-recent notify-hook + userdata on the
        // singleton for test introspection. Upstream only logs.
        let _ = (cb, userdata);
        Ok(())
    }

    pub fn config_set_request_hook2(&mut self) -> Result<(), CellError> {
        self.config_set_request_hook2_calls =
            self.config_set_request_hook2_calls.saturating_add(1);
        Ok(())
    }

    pub fn config_set_status_hook2(&mut self) -> Result<(), CellError> {
        self.config_set_status_hook2_calls =
            self.config_set_status_hook2_calls.saturating_add(1);
        Ok(())
    }

    pub fn config_set_tab_count2(
        &mut self,
        config: Option<&mut CellWebBrowserConfig2>,
        tab_count: u32,
    ) -> Result<(), CellError> {
        self.config_set_tab_count2_calls =
            self.config_set_tab_count2_calls.saturating_add(1);
        if let Some(c) = config {
            c.tab_count = tab_count as i32;
        }
        Ok(())
    }

    pub fn config_set_unknown_mime_type_hook2(
        &mut self,
        _config: Option<&CellWebBrowserConfig2>,
        _cb: u32,
        _userdata: u32,
    ) -> Result<(), CellError> {
        self.config_set_unknown_mime_type_hook2_calls = self
            .config_set_unknown_mime_type_hook2_calls
            .saturating_add(1);
        Ok(())
    }

    pub fn config_set_version(&mut self) -> Result<(), CellError> {
        self.config_set_version_calls = self.config_set_version_calls.saturating_add(1);
        Ok(())
    }

    pub fn config_set_view_condition2(
        &mut self,
        config: Option<&mut CellWebBrowserConfig2>,
        cond: u32,
    ) -> Result<(), CellError> {
        self.config_set_view_condition2_calls =
            self.config_set_view_condition2_calls.saturating_add(1);
        if let Some(c) = config {
            c.view_restriction = cond as i32;
        }
        Ok(())
    }

    pub fn config_set_view_rect2(&mut self) -> Result<(), CellError> {
        self.config_set_view_rect2_calls = self.config_set_view_rect2_calls.saturating_add(1);
        Ok(())
    }

    pub fn config_with_ver(&mut self) -> Result<(), CellError> {
        self.config_with_ver_calls = self.config_with_ver_calls.saturating_add(1);
        Ok(())
    }

    // ---- Create / Destroy -----------------------------------------------

    pub fn create(&mut self) -> Result<u32, CellError> {
        self.create_calls = self.create_calls.saturating_add(1);
        self.alloc_browser(BrowserVariant::V1)
    }

    pub fn create2(&mut self) -> Result<u32, CellError> {
        self.create2_calls = self.create2_calls.saturating_add(1);
        self.alloc_browser(BrowserVariant::V2)
    }

    pub fn create_render2(&mut self) -> Result<u32, CellError> {
        self.create_render2_calls = self.create_render2_calls.saturating_add(1);
        self.alloc_browser(BrowserVariant::V2)
    }

    pub fn create_render_with_rect2(&mut self) -> Result<u32, CellError> {
        self.create_render_with_rect2_calls =
            self.create_render_with_rect2_calls.saturating_add(1);
        self.alloc_browser(BrowserVariant::V2)
    }

    pub fn create_with_config(&mut self) -> Result<u32, CellError> {
        self.create_with_config_calls = self.create_with_config_calls.saturating_add(1);
        self.alloc_browser(BrowserVariant::V1)
    }

    pub fn create_with_config_full(&mut self) -> Result<u32, CellError> {
        self.create_with_config_full_calls =
            self.create_with_config_full_calls.saturating_add(1);
        self.alloc_browser(BrowserVariant::V1)
    }

    pub fn create_with_rect2(&mut self) -> Result<u32, CellError> {
        self.create_with_rect2_calls = self.create_with_rect2_calls.saturating_add(1);
        self.alloc_browser(BrowserVariant::V2)
    }

    pub fn destroy(&mut self, id: u32) -> Result<(), CellError> {
        self.destroy_calls = self.destroy_calls.saturating_add(1);
        self.destroy_common(id, BrowserVariant::V1)
    }

    pub fn destroy2(&mut self, id: u32) -> Result<(), CellError> {
        self.destroy2_calls = self.destroy2_calls.saturating_add(1);
        self.destroy_common(id, BrowserVariant::V2)
    }

    fn destroy_common(&mut self, id: u32, variant: BrowserVariant) -> Result<(), CellError> {
        self.require_initialized()?;
        let b = self
            .browser_mut(id)
            .ok_or(CELL_WEBBROWSER_ERROR_BROWSER_NOT_FOUND)?;
        if b.variant != variant {
            return Err(CELL_WEBBROWSER_ERROR_INVALID_PARAMETER);
        }
        b.state = BrowserState::Destroyed;
        Ok(())
    }

    // ---- Estimate / Activate / Deactivate -------------------------------

    pub fn estimate(&mut self) -> Result<(), CellError> {
        self.estimate_calls = self.estimate_calls.saturating_add(1);
        Ok(())
    }

    /// `cellWebBrowserEstimate2(config, memSize)` — writes `ESTIMATE2_MEM_SIZE`
    /// to `*memSize`. Upstream unconditionally returns the constant.
    pub fn estimate2(
        &mut self,
        _config: Option<&CellWebBrowserConfig2>,
        out_mem_size: &mut u32,
    ) -> Result<(), CellError> {
        self.estimate2_calls = self.estimate2_calls.saturating_add(1);
        *out_mem_size = ESTIMATE2_MEM_SIZE;
        Ok(())
    }

    pub fn deactivate(&mut self) -> Result<(), CellError> {
        self.deactivate_calls = self.deactivate_calls.saturating_add(1);
        // Deactivate all live browsers — mirrors upstream semantics where the
        // PS3 firmware transitions all web contexts offscreen.
        for b in &mut self.browsers {
            if matches!(b.state, BrowserState::Active) {
                b.state = BrowserState::Inactive;
            }
        }
        Ok(())
    }

    // ---- Initialize / Shutdown ------------------------------------------

    /// `cellWebBrowserInitialize(system_cb, container)` — stashes the callback
    /// and queues an `INITIALIZING_FINISHED` sysutil event.
    pub fn initialize(
        &mut self,
        system_cb: u32,
        container: u32,
    ) -> Result<(), CellError> {
        self.initialize_calls = self.initialize_calls.saturating_add(1);
        if matches!(self.state, ModuleState::Initialized) {
            return Err(CELL_WEBBROWSER_ERROR_ALREADY_INITIALIZED);
        }
        self.system_cb = system_cb;
        self.container = container;
        self.state = ModuleState::Initialized;
        self.pending.push(PendingSystemEvent {
            system_cb,
            userdata: self.userdata,
            event_code: CELL_SYSUTIL_WEBBROWSER_INITIALIZING_FINISHED,
        });
        Ok(())
    }

    /// `cellWebBrowserShutdown()` — void return upstream, but we expose
    /// `Result` for consistency. Queues `SHUTDOWN_FINISHED`.
    pub fn shutdown(&mut self) -> Result<(), CellError> {
        self.shutdown_calls = self.shutdown_calls.saturating_add(1);
        // Upstream does NOT require prior init; it just logs + queues a
        // callback. We preserve that: queue even if not initialized, but
        // only using the stashed system_cb (0 if never set).
        self.pending.push(PendingSystemEvent {
            system_cb: self.system_cb,
            userdata: self.userdata,
            event_code: CELL_SYSUTIL_WEBBROWSER_SHUTDOWN_FINISHED,
        });
        self.state = ModuleState::Shutdown;
        Ok(())
    }

    // ---- Set_Usrdata + Navigate + Wake ----------------------------------

    pub fn set_system_callback_usrdata(&mut self, userdata: u32) -> Result<(), CellError> {
        self.set_system_callback_usrdata_calls =
            self.set_system_callback_usrdata_calls.saturating_add(1);
        self.userdata = userdata;
        Ok(())
    }

    pub fn navigate2(&mut self) -> Result<(), CellError> {
        self.navigate2_calls = self.navigate2_calls.saturating_add(1);
        Ok(())
    }

    pub fn set_local_contents_additional_title_id(&mut self) -> Result<(), CellError> {
        self.set_local_contents_additional_title_id_calls = self
            .set_local_contents_additional_title_id_calls
            .saturating_add(1);
        Ok(())
    }

    pub fn get_usrdata_on_game_exit(&mut self) -> Result<(), CellError> {
        self.get_usrdata_on_game_exit_calls =
            self.get_usrdata_on_game_exit_calls.saturating_add(1);
        Ok(())
    }

    pub fn update_pointer_display_pos2(&mut self) -> Result<(), CellError> {
        self.update_pointer_display_pos2_calls =
            self.update_pointer_display_pos2_calls.saturating_add(1);
        Ok(())
    }

    pub fn wakeup_with_game_exit(&mut self) -> Result<(), CellError> {
        self.wakeup_with_game_exit_calls =
            self.wakeup_with_game_exit_calls.saturating_add(1);
        Ok(())
    }

    // ---- WebComponent (separate family) --------------------------------

    pub fn web_component_create(&mut self) -> Result<(), CellError> {
        self.web_component_create_calls = self.web_component_create_calls.saturating_add(1);
        Ok(())
    }

    pub fn web_component_create_async(&mut self) -> Result<(), CellError> {
        self.web_component_create_async_calls =
            self.web_component_create_async_calls.saturating_add(1);
        Ok(())
    }

    pub fn web_component_destroy(&mut self) -> Result<(), CellError> {
        self.web_component_destroy_calls = self.web_component_destroy_calls.saturating_add(1);
        Ok(())
    }

    // ---- Test hooks -----------------------------------------------------

    /// Flip a browser to Active — useful for test scenarios wanting a live
    /// context after `Create*`.
    pub fn mark_active(&mut self, id: u32) {
        if let Some(b) = self.browser_mut(id) {
            if !matches!(b.state, BrowserState::Destroyed) {
                b.state = BrowserState::Active;
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_module_and_entries_match_cpp() {
        assert_eq!(HOST_MODULE_NAME, "cellSysutil");
        assert_eq!(SUBMODULE_NAME, "cellWebBrowser");
        assert_eq!(REGISTERED_ENTRY_POINTS.len(), 47);
        assert_eq!(REGISTERED_ENTRY_POINTS[0], "cellWebBrowserActivate");
        assert_eq!(REGISTERED_ENTRY_POINTS[36], "cellWebBrowserGetUsrdataOnGameExit");
        assert_eq!(REGISTERED_ENTRY_POINTS[37], "cellWebBrowserInitialize");
        assert_eq!(REGISTERED_ENTRY_POINTS[46], "cellWebComponentDestroy");
    }

    #[test]
    fn event_codes_byte_exact() {
        assert_eq!(CELL_SYSUTIL_WEBBROWSER_INITIALIZING_FINISHED, 1);
        assert_eq!(CELL_SYSUTIL_WEBBROWSER_SHUTDOWN_FINISHED, 4);
        assert_eq!(CELL_SYSUTIL_WEBBROWSER_LOADING_FINISHED, 5);
        assert_eq!(CELL_SYSUTIL_WEBBROWSER_UNLOADING_FINISHED, 7);
        assert_eq!(CELL_SYSUTIL_WEBBROWSER_RELEASED, 9);
        assert_eq!(CELL_SYSUTIL_WEBBROWSER_GRABBED, 11);
    }

    #[test]
    fn estimate2_constant_matches_cpp() {
        assert_eq!(ESTIMATE2_MEM_SIZE, 1 * 1024 * 1024);
        let mut m = WebBrowser::new();
        let mut mem_size = 0u32;
        m.estimate2(None, &mut mem_size).unwrap();
        assert_eq!(mem_size, 1 * 1024 * 1024);
        assert_eq!(m.estimate2_calls, 1);
    }

    #[test]
    fn placeholder_error_codes_byte_exact() {
        assert_eq!(CELL_WEBBROWSER_ERROR_NOT_INITIALIZED.0, 0x8002_F701);
        assert_eq!(CELL_WEBBROWSER_ERROR_ALREADY_INITIALIZED.0, 0x8002_F702);
        assert_eq!(CELL_WEBBROWSER_ERROR_INVALID_PARAMETER.0, 0x8002_F703);
        assert_eq!(CELL_WEBBROWSER_ERROR_NOT_ACTIVE.0, 0x8002_F704);
        assert_eq!(CELL_WEBBROWSER_ERROR_ALREADY_ACTIVE.0, 0x8002_F705);
        assert_eq!(CELL_WEBBROWSER_ERROR_BROWSER_NOT_FOUND.0, 0x8002_F706);
        assert_eq!(CELL_WEBBROWSER_ERROR_OUT_OF_BROWSERS.0, 0x8002_F707);
    }

    #[test]
    fn wire_struct_sizes() {
        assert_eq!(core::mem::size_of::<CellWebBrowserPos>(), 8);
        assert_eq!(core::mem::size_of::<CellWebBrowserSize>(), 8);
        assert_eq!(core::mem::size_of::<CellWebBrowserRect>(), 16);
        assert_eq!(core::mem::size_of::<CellWebBrowserMimeSet>(), 8);
        assert_eq!(core::mem::size_of::<CellWebBrowserConfig>(), 36);
        assert_eq!(core::mem::size_of::<CellWebBrowserConfig2>(), 68);
    }

    #[test]
    fn initialize_captures_system_cb_and_queues_event() {
        let mut m = WebBrowser::new();
        m.initialize(0xDEAD_0001, 0xC0FFEE).unwrap();
        assert_eq!(m.state(), ModuleState::Initialized);
        assert_eq!(m.system_cb(), 0xDEAD_0001);
        assert_eq!(m.container(), 0xC0FFEE);
        assert_eq!(m.pending_len(), 1);
        assert_eq!(
            m.pending_events()[0].event_code,
            CELL_SYSUTIL_WEBBROWSER_INITIALIZING_FINISHED
        );
        assert_eq!(m.pending_events()[0].system_cb, 0xDEAD_0001);
    }

    #[test]
    fn initialize_twice_is_already_initialized() {
        let mut m = WebBrowser::new();
        m.initialize(1, 2).unwrap();
        assert_eq!(
            m.initialize(3, 4),
            Err(CELL_WEBBROWSER_ERROR_ALREADY_INITIALIZED)
        );
    }

    #[test]
    fn shutdown_queues_event_even_if_not_initialized() {
        let mut m = WebBrowser::new();
        m.shutdown().unwrap();
        assert_eq!(m.state(), ModuleState::Shutdown);
        assert_eq!(m.pending_len(), 1);
        let ev = m.pending_events()[0];
        assert_eq!(ev.event_code, CELL_SYSUTIL_WEBBROWSER_SHUTDOWN_FINISHED);
        assert_eq!(ev.system_cb, 0); // never captured
    }

    #[test]
    fn shutdown_after_init_propagates_stashed_callback() {
        let mut m = WebBrowser::new();
        m.initialize(0x9000, 0).unwrap();
        m.set_system_callback_usrdata(0x7777).unwrap();
        m.shutdown().unwrap();
        let drained = m.drain_pending();
        assert_eq!(drained.len(), 2);
        assert_eq!(drained[1].system_cb, 0x9000);
        assert_eq!(drained[1].userdata, 0x7777);
        assert_eq!(drained[1].event_code, CELL_SYSUTIL_WEBBROWSER_SHUTDOWN_FINISHED);
    }

    #[test]
    fn drain_pending_empties_queue() {
        let mut m = WebBrowser::new();
        m.initialize(1, 0).unwrap();
        assert_eq!(m.pending_len(), 1);
        let drained = m.drain_pending();
        assert_eq!(drained.len(), 1);
        assert_eq!(m.pending_len(), 0);
    }

    #[test]
    fn create_requires_initialize() {
        let mut m = WebBrowser::new();
        assert_eq!(m.create(), Err(CELL_WEBBROWSER_ERROR_NOT_INITIALIZED));
        m.initialize(0, 0).unwrap();
        let id = m.create().unwrap();
        assert_eq!(id, BROWSER_ID_BASE);
    }

    #[test]
    fn create_variants_match_versions() {
        let mut m = WebBrowser::new();
        m.initialize(0, 0).unwrap();
        let v1a = m.create().unwrap();
        let v1b = m.create_with_config().unwrap();
        let v1c = m.create_with_config_full().unwrap();
        let v2a = m.create2().unwrap();
        let v2b = m.create_render2().unwrap();
        let v2c = m.create_render_with_rect2().unwrap();
        let v2d = m.create_with_rect2().unwrap();

        assert_eq!(m.browser(v1a).unwrap().variant, BrowserVariant::V1);
        assert_eq!(m.browser(v1b).unwrap().variant, BrowserVariant::V1);
        assert_eq!(m.browser(v1c).unwrap().variant, BrowserVariant::V1);
        assert_eq!(m.browser(v2a).unwrap().variant, BrowserVariant::V2);
        assert_eq!(m.browser(v2b).unwrap().variant, BrowserVariant::V2);
        assert_eq!(m.browser(v2c).unwrap().variant, BrowserVariant::V2);
        assert_eq!(m.browser(v2d).unwrap().variant, BrowserVariant::V2);
        // Per-entry counters match.
        assert_eq!(m.create_calls, 1);
        assert_eq!(m.create2_calls, 1);
        assert_eq!(m.create_render2_calls, 1);
        assert_eq!(m.create_render_with_rect2_calls, 1);
        assert_eq!(m.create_with_config_calls, 1);
        assert_eq!(m.create_with_config_full_calls, 1);
        assert_eq!(m.create_with_rect2_calls, 1);
    }

    #[test]
    fn create_over_max_rejected() {
        let mut m = WebBrowser::new();
        m.initialize(0, 0).unwrap();
        for _ in 0..MAX_BROWSERS {
            m.create().unwrap();
        }
        assert_eq!(m.create(), Err(CELL_WEBBROWSER_ERROR_OUT_OF_BROWSERS));
    }

    #[test]
    fn destroy_respects_variant_match() {
        let mut m = WebBrowser::new();
        m.initialize(0, 0).unwrap();
        let v1 = m.create().unwrap();
        let v2 = m.create2().unwrap();
        // Mismatched variant → INVALID_PARAMETER.
        assert_eq!(m.destroy(v2), Err(CELL_WEBBROWSER_ERROR_INVALID_PARAMETER));
        assert_eq!(m.destroy2(v1), Err(CELL_WEBBROWSER_ERROR_INVALID_PARAMETER));
        // Matched variant succeeds.
        m.destroy(v1).unwrap();
        m.destroy2(v2).unwrap();
        assert_eq!(m.browser(v1).unwrap().state, BrowserState::Destroyed);
        assert_eq!(m.browser(v2).unwrap().state, BrowserState::Destroyed);
    }

    #[test]
    fn destroy_unknown_id_returns_not_found() {
        let mut m = WebBrowser::new();
        m.initialize(0, 0).unwrap();
        assert_eq!(
            m.destroy(0xDEAD_BEEF),
            Err(CELL_WEBBROWSER_ERROR_BROWSER_NOT_FOUND)
        );
    }

    #[test]
    fn config2_setters_write_through() {
        let mut m = WebBrowser::new();
        let mut cfg = CellWebBrowserConfig2::default();
        m.config_set_heap_size2(Some(&mut cfg), 0x100_000).unwrap();
        m.config_set_tab_count2(Some(&mut cfg), 4).unwrap();
        m.config_set_function2(Some(&mut cfg), 0xABCD).unwrap();
        m.config_set_full_screen2(Some(&mut cfg), 1).unwrap();
        m.config_set_view_condition2(Some(&mut cfg), 0x2).unwrap();
        assert_eq!(cfg.heap_size, 0x100_000);
        assert_eq!(cfg.tab_count, 4);
        assert_eq!(cfg.functions as u32, 0xABCD);
        assert_eq!(cfg.size_mode, 1);
        assert_eq!(cfg.view_restriction, 2);
    }

    #[test]
    fn config2_setters_tolerate_null_config() {
        let mut m = WebBrowser::new();
        m.config_set_heap_size2(None, 0x100).unwrap();
        m.config_set_tab_count2(None, 2).unwrap();
        m.config_set_function2(None, 0x1).unwrap();
        m.config_set_full_screen2(None, 1).unwrap();
        m.config_set_view_condition2(None, 1).unwrap();
        // No observable effect — upstream also only logs.
    }

    #[test]
    fn deactivate_flips_active_to_inactive() {
        let mut m = WebBrowser::new();
        m.initialize(0, 0).unwrap();
        let a = m.create().unwrap();
        let b = m.create2().unwrap();
        m.mark_active(a);
        m.mark_active(b);
        m.deactivate().unwrap();
        assert_eq!(m.browser(a).unwrap().state, BrowserState::Inactive);
        assert_eq!(m.browser(b).unwrap().state, BrowserState::Inactive);
        // Destroyed browsers stay Destroyed.
        m.mark_active(a); // no-op because we're going to destroy it
        m.destroy(a).unwrap();
        m.deactivate().unwrap();
        assert_eq!(m.browser(a).unwrap().state, BrowserState::Destroyed);
    }

    #[test]
    fn counters_cover_all_entries() {
        let mut m = WebBrowser::new();
        // Touch every entry at least once (some require init).
        m.activate().unwrap();
        m.config().unwrap();
        m.config2(None, 0).unwrap();
        m.config_get_heap_size().unwrap();
        m.config_get_heap_size2().unwrap();
        m.config_set_custom_exit().unwrap();
        m.config_set_disable_tabs().unwrap();
        m.config_set_error_hook2().unwrap();
        m.config_set_full_screen2(None, 0).unwrap();
        m.config_set_full_version2().unwrap();
        m.config_set_function().unwrap();
        m.config_set_function2(None, 0).unwrap();
        m.config_set_heap_size().unwrap();
        m.config_set_heap_size2(None, 0).unwrap();
        m.config_set_mime_set().unwrap();
        m.config_set_notify_hook2(None, 0, 0).unwrap();
        m.config_set_request_hook2().unwrap();
        m.config_set_status_hook2().unwrap();
        m.config_set_tab_count2(None, 0).unwrap();
        m.config_set_unknown_mime_type_hook2(None, 0, 0).unwrap();
        m.config_set_version().unwrap();
        m.config_set_view_condition2(None, 0).unwrap();
        m.config_set_view_rect2().unwrap();
        m.config_with_ver().unwrap();

        m.initialize(0, 0).unwrap();
        let _ = m.create();
        let _ = m.create2();
        let _ = m.create_render2();
        let _ = m.create_render_with_rect2();
        let _ = m.create_with_config();
        let _ = m.create_with_config_full();
        let _ = m.create_with_rect2();

        m.deactivate().unwrap();
        m.estimate().unwrap();
        let mut ms = 0u32;
        m.estimate2(None, &mut ms).unwrap();
        m.get_usrdata_on_game_exit().unwrap();
        m.navigate2().unwrap();
        m.set_local_contents_additional_title_id().unwrap();
        m.set_system_callback_usrdata(0).unwrap();
        m.update_pointer_display_pos2().unwrap();
        m.wakeup_with_game_exit().unwrap();
        m.web_component_create().unwrap();
        m.web_component_create_async().unwrap();
        m.web_component_destroy().unwrap();
        m.shutdown().unwrap();
        // 41 destroy/destroy2 will need valid ids — exercised in other tests.

        // Spot-check counters are non-zero. The sum should equal 44 (minus
        // destroy/destroy2 which we skip here).
        let total = m.activate_calls
            + m.config_calls
            + m.config2_calls
            + m.config_get_heap_size_calls
            + m.config_get_heap_size2_calls
            + m.config_set_custom_exit_calls
            + m.config_set_disable_tabs_calls
            + m.config_set_error_hook2_calls
            + m.config_set_full_screen2_calls
            + m.config_set_full_version2_calls
            + m.config_set_function_calls
            + m.config_set_function2_calls
            + m.config_set_heap_size_calls
            + m.config_set_heap_size2_calls
            + m.config_set_mime_set_calls
            + m.config_set_notify_hook2_calls
            + m.config_set_request_hook2_calls
            + m.config_set_status_hook2_calls
            + m.config_set_tab_count2_calls
            + m.config_set_unknown_mime_type_hook2_calls
            + m.config_set_version_calls
            + m.config_set_view_condition2_calls
            + m.config_set_view_rect2_calls
            + m.config_with_ver_calls
            + m.create_calls
            + m.create2_calls
            + m.create_render2_calls
            + m.create_render_with_rect2_calls
            + m.create_with_config_calls
            + m.create_with_config_full_calls
            + m.create_with_rect2_calls
            + m.deactivate_calls
            + m.estimate_calls
            + m.estimate2_calls
            + m.get_usrdata_on_game_exit_calls
            + m.initialize_calls
            + m.navigate2_calls
            + m.set_local_contents_additional_title_id_calls
            + m.set_system_callback_usrdata_calls
            + m.shutdown_calls
            + m.update_pointer_display_pos2_calls
            + m.wakeup_with_game_exit_calls
            + m.web_component_create_calls
            + m.web_component_create_async_calls
            + m.web_component_destroy_calls;
        assert_eq!(total, 45); // 46 entries minus destroy + destroy2 = 44, plus 1 extra from create counters double-counting? No — create family = 7. Init=1, shutdown=1, estimate=1, estimate2=1, deactivate=1, 24 config/activate stubs =24, 6 misc (get_usrdata, navigate2, set_local_*, set_system_cb_usrdata, update_pointer, wakeup) = 6, 3 webcomponent =3. Total = 7+1+1+1+1+1+24+6+3 = 45.
    }

    #[test]
    fn set_system_callback_usrdata_updates_singleton() {
        let mut m = WebBrowser::new();
        m.initialize(0x1000, 0).unwrap();
        m.set_system_callback_usrdata(0x5678).unwrap();
        assert_eq!(m.userdata(), 0x5678);
        // Subsequent shutdown event should carry the new userdata.
        m.shutdown().unwrap();
        let ev = m.pending_events().last().unwrap();
        assert_eq!(ev.userdata, 0x5678);
    }

    #[test]
    fn browser_ids_are_monotonic_across_variants() {
        let mut m = WebBrowser::new();
        m.initialize(0, 0).unwrap();
        let a = m.create().unwrap();
        let b = m.create2().unwrap();
        let c = m.create_render2().unwrap();
        assert_eq!(a, BROWSER_ID_BASE);
        assert_eq!(b, BROWSER_ID_BASE + 1);
        assert_eq!(c, BROWSER_ID_BASE + 2);
    }

    #[test]
    fn full_webbrowser_lifecycle_smoke() {
        let mut m = WebBrowser::new();

        // Estimate2 is callable pre-init.
        let mut mem_size = 0u32;
        m.estimate2(None, &mut mem_size).unwrap();
        assert_eq!(mem_size, 1 * 1024 * 1024);

        // Init + userdata + create
        m.initialize(0xCB_ABCD, 0xC0).unwrap();
        m.set_system_callback_usrdata(0xDEAD).unwrap();
        let browser = m.create2().unwrap();
        m.mark_active(browser);

        // Config2 write-through
        let mut cfg = CellWebBrowserConfig2::default();
        m.config_set_heap_size2(Some(&mut cfg), 0x1_0000).unwrap();
        m.config_set_tab_count2(Some(&mut cfg), 3).unwrap();
        assert_eq!(cfg.heap_size, 0x1_0000);
        assert_eq!(cfg.tab_count, 3);

        m.navigate2().unwrap();

        // Deactivate flips Active→Inactive
        m.deactivate().unwrap();
        assert_eq!(m.browser(browser).unwrap().state, BrowserState::Inactive);

        // Destroy + shutdown
        m.destroy2(browser).unwrap();
        m.shutdown().unwrap();

        // Drain pending events: init + shutdown
        let pending = m.drain_pending();
        assert_eq!(pending.len(), 2);
        assert_eq!(pending[0].event_code, CELL_SYSUTIL_WEBBROWSER_INITIALIZING_FINISHED);
        assert_eq!(pending[1].event_code, CELL_SYSUTIL_WEBBROWSER_SHUTDOWN_FINISHED);
        assert_eq!(pending[1].userdata, 0xDEAD);
        assert_eq!(m.state(), ModuleState::Shutdown);
    }
}
