//! `rpcs3-hle-cellsearch` — XMB media search HLE.
//!
//! Ports `rpcs3/Emu/Cell/Modules/cellSearch.cpp`. cellSearch indexes the
//! media libraries (Music/Photo/Video) backed by the XMB content DB on
//! the PS3 hard drive. It is async-event-driven in the real lib (callbacks
//! fire into a user-supplied handler). We model:
//!
//! 1. Lifecycle FSM: Uninitialized → Initializing → Ready → Finalizing.
//! 2. An active search context identified by `SearchId` — only one at a
//!    time (`BUSY` otherwise).
//! 3. A content index (id → `ContentInfo`) so games can iterate results.
//!
//! ## Entry points covered
//!
//! | HLE function                                 | Rust wrapper                          |
//! |----------------------------------------------|---------------------------------------|
//! | `cellSearchInitialize`                       | [`SearchManager::initialize`]         |
//! | `cellSearchFinalize`                         | [`SearchManager::finalize`]           |
//! | `cellSearchStart`                            | [`SearchManager::start`]              |
//! | `cellSearchCancel`                           | [`SearchManager::cancel`]             |
//! | `cellSearchEnd`                              | [`SearchManager::end`]                |
//! | `cellSearchNotificationOpen` / `Close`       | [`SearchManager::notification_open`] |
//! | `cellSearchGetContentInfoByOffset`           | [`SearchManager::content_info_by_offset`] |
//! | `cellSearchGetContentInfoByContentId`        | [`SearchManager::content_info_by_id`] |
//! | `cellSearchGetContentInfoGameComment`        | [`SearchManager::content_info_game_comment`] |

use rpcs3_emu_types::CellError;

// =====================================================================
// Error codes — byte-exact with cellSearch.h:7-28
// =====================================================================

pub const CANCELED: i32 = 1;

pub mod errors {
    use rpcs3_emu_types::CellError;

    pub const PARAM: CellError = CellError(0x8002_C801);
    pub const BUSY: CellError = CellError(0x8002_C802);
    pub const NO_MEMORY: CellError = CellError(0x8002_C803);
    pub const UNKNOWN_MODE: CellError = CellError(0x8002_C804);
    pub const ALREADY_INITIALIZED: CellError = CellError(0x8002_C805);
    pub const NOT_INITIALIZED: CellError = CellError(0x8002_C806);
    pub const FINALIZING: CellError = CellError(0x8002_C807);
    pub const NOT_SUPPORTED_SEARCH: CellError = CellError(0x8002_C808);
    pub const CONTENT_OBSOLETE: CellError = CellError(0x8002_C809);
    pub const CONTENT_NOT_FOUND: CellError = CellError(0x8002_C80A);
    pub const NOT_LIST: CellError = CellError(0x8002_C80B);
    pub const OUT_OF_RANGE: CellError = CellError(0x8002_C80C);
    pub const INVALID_SEARCHID: CellError = CellError(0x8002_C80D);
    pub const ALREADY_GOT_RESULT: CellError = CellError(0x8002_C80E);
    pub const NOT_SUPPORTED_CONTEXT: CellError = CellError(0x8002_C80F);
    pub const INVALID_CONTENTTYPE: CellError = CellError(0x8002_C810);
    pub const DRM: CellError = CellError(0x8002_C811);
    pub const TAG: CellError = CellError(0x8002_C812);
    pub const GENERIC: CellError = CellError(0x8002_C8FF);
}

// =====================================================================
// Constants
// =====================================================================

pub const CONTENT_ID_SIZE: usize = 16;
pub const TITLE_LEN_MAX: usize = 384;
pub const TAG_NUM_MAX: usize = 6;
pub const TAG_LEN_MAX: usize = 63;
pub const PATH_LEN_MAX: usize = 63;
pub const MTOPTION_LEN_MAX: usize = 63;
pub const DEVELOPERDATA_LEN_MAX: usize = 64;
pub const GAMECOMMENT_SIZE_MAX: usize = 1024;
pub const CONTENT_BUFFER_SIZE_MAX: usize = 2048;

// =====================================================================
// Search mode / content type / list type / event enums
// =====================================================================

pub const MODE_NORMAL: i32 = 0;

#[must_use]
pub fn is_known_mode(mode: i32) -> bool {
    matches!(mode, MODE_NORMAL)
}

// Content types (cellSearch.h:69-79)
pub const CONTENTTYPE_NONE: i32 = 0;
pub const CONTENTTYPE_MUSIC: i32 = 1;
pub const CONTENTTYPE_MUSICLIST: i32 = 2;
pub const CONTENTTYPE_PHOTO: i32 = 3;
pub const CONTENTTYPE_PHOTOLIST: i32 = 4;
pub const CONTENTTYPE_VIDEO: i32 = 5;
pub const CONTENTTYPE_VIDEOLIST: i32 = 6;
pub const CONTENTTYPE_SCENE: i32 = 7;

// Content-search type (top-level buckets for ContentSearch API)
pub const CONTENTSEARCHTYPE_NONE: i32 = 0;
pub const CONTENTSEARCHTYPE_MUSIC_ALL: i32 = 1;
pub const CONTENTSEARCHTYPE_PHOTO_ALL: i32 = 2;
pub const CONTENTSEARCHTYPE_VIDEO_ALL: i32 = 3;

#[must_use]
pub fn is_known_content_search_type(t: i32) -> bool {
    (CONTENTSEARCHTYPE_NONE..=CONTENTSEARCHTYPE_VIDEO_ALL).contains(&t)
}

// Event types (cellSearch.h:165-175)
pub const EVENT_NOTIFICATION: i32 = 0;
pub const EVENT_INITIALIZE_RESULT: i32 = 1;
pub const EVENT_FINALIZE_RESULT: i32 = 2;
pub const EVENT_LISTSEARCH_RESULT: i32 = 3;
pub const EVENT_CONTENTSEARCH_INLIST_RESULT: i32 = 4;
pub const EVENT_CONTENTSEARCH_RESULT: i32 = 5;
pub const EVENT_SCENESEARCH_INVIDEO_RESULT: i32 = 6;
pub const EVENT_SCENESEARCH_RESULT: i32 = 7;

// Sort key/order (cellSearch.h:44-66)
pub const SORTKEY_NONE: i32 = 0;
pub const SORTKEY_DEFAULT: i32 = 1;
pub const SORTKEY_TITLE: i32 = 2;
pub const SORTKEY_IMPORTEDDATE: i32 = 6;
pub const SORTKEY_MODIFIEDDATE: i32 = 10;

pub const SORTORDER_NONE: i32 = 0;
pub const SORTORDER_ASCENDING: i32 = 1;
pub const SORTORDER_DESCENDING: i32 = 2;

#[must_use]
pub fn is_known_sort_key(key: i32) -> bool {
    (SORTKEY_NONE..=SORTKEY_MODIFIEDDATE).contains(&key)
}

#[must_use]
pub fn is_known_sort_order(order: i32) -> bool {
    (SORTORDER_NONE..=SORTORDER_DESCENDING).contains(&order)
}

// Content status (cellSearch.h:140-146)
pub const CONTENTSTATUS_NONE: i32 = 0;
pub const CONTENTSTATUS_AVAILABLE: i32 = 1;
pub const CONTENTSTATUS_NOT_SUPPORTED: i32 = 2;
pub const CONTENTSTATUS_BROKEN: i32 = 3;

// =====================================================================
// Types
// =====================================================================

pub type SearchId = i32;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContentId(pub [u8; CONTENT_ID_SIZE]);

impl ContentId {
    #[must_use]
    pub const fn new(data: [u8; CONTENT_ID_SIZE]) -> Self {
        Self(data)
    }

    /// Deterministic construction from an arbitrary id (test + reference
    /// backends can use this without PRNG).
    #[must_use]
    pub fn from_u64(v: u64) -> Self {
        let mut buf = [0u8; CONTENT_ID_SIZE];
        buf[..8].copy_from_slice(&v.to_be_bytes());
        buf[8..].copy_from_slice(&v.wrapping_mul(0x9E37_79B1_185E_B4A5).to_be_bytes());
        Self(buf)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContentInfo {
    pub id: ContentId,
    pub content_type: i32,
    pub status: i32,
    pub path: String, // ≤ PATH_LEN_MAX
    pub title: String,
    pub tags: Vec<String>,
    pub game_comment: String,
    pub duration: i64,  // µs for music/video; 0 for photo
    pub size_bytes: u64,
    pub is_drm: bool,
}

impl ContentInfo {
    #[must_use]
    pub fn new(id: ContentId, content_type: i32, path: impl Into<String>) -> Self {
        Self {
            id,
            content_type,
            status: CONTENTSTATUS_AVAILABLE,
            path: path.into(),
            title: String::new(),
            tags: Vec::new(),
            game_comment: String::new(),
            duration: 0,
            size_bytes: 0,
            is_drm: false,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SearchState {
    Uninitialized,
    Initializing,
    Ready,
    Finalizing,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SearchSession {
    pub id: SearchId,
    pub search_type: i32,
    pub sort_key: i32,
    pub sort_order: i32,
    pub results: Vec<ContentId>,
    pub result_consumed: bool,
}

// =====================================================================
// SearchManager — behaves like the cellSearch FXO
// =====================================================================

#[derive(Clone, Debug)]
pub struct SearchManager {
    state: SearchState,
    mode: i32,
    active: Option<SearchSession>,
    next_id: SearchId,
    notification_open: bool,
    index: Vec<ContentInfo>,
}

impl SearchManager {
    #[must_use]
    pub fn new() -> Self {
        Self {
            state: SearchState::Uninitialized,
            mode: MODE_NORMAL,
            active: None,
            next_id: 1,
            notification_open: false,
            index: Vec::new(),
        }
    }

    #[must_use]
    pub fn state(&self) -> &SearchState {
        &self.state
    }

    #[must_use]
    pub fn mode(&self) -> i32 {
        self.mode
    }

    /// Pre-populate the content index with a fixture set. Real emu-core
    /// integration walks dev_hdd0 and builds this at init.
    pub fn add_content(&mut self, info: ContentInfo) {
        self.index.push(info);
    }

    #[must_use]
    pub fn index_len(&self) -> usize {
        self.index.len()
    }

    // ----------------- Lifecycle -----------------

    /// `cellSearchInitialize(mode, container, callback, userData)`.
    /// Validates mode, transitions Uninitialized → Initializing → Ready.
    pub fn initialize(&mut self, mode: i32) -> Result<(), CellError> {
        match self.state {
            SearchState::Uninitialized => {}
            SearchState::Ready | SearchState::Initializing => return Err(errors::ALREADY_INITIALIZED),
            SearchState::Finalizing => return Err(errors::FINALIZING),
        }
        if !is_known_mode(mode) {
            return Err(errors::UNKNOWN_MODE);
        }
        self.mode = mode;
        self.state = SearchState::Initializing;
        // In real HW the callback fires async; here we snap to Ready on
        // success — games poll `state()` / get `EVENT_INITIALIZE_RESULT`.
        self.state = SearchState::Ready;
        Ok(())
    }

    /// `cellSearchFinalize(callback, userData)`.
    pub fn finalize(&mut self) -> Result<(), CellError> {
        match self.state {
            SearchState::Uninitialized => return Err(errors::NOT_INITIALIZED),
            SearchState::Ready => {}
            SearchState::Initializing => return Err(errors::BUSY),
            SearchState::Finalizing => return Err(errors::FINALIZING),
        }
        if self.active.is_some() {
            return Err(errors::BUSY);
        }
        self.state = SearchState::Finalizing;
        self.notification_open = false;
        self.state = SearchState::Uninitialized;
        Ok(())
    }

    // ----------------- Notifications -----------------

    /// `cellSearchNotificationOpen(callback, userData)`.
    pub fn notification_open(&mut self) -> Result<(), CellError> {
        self.require_ready()?;
        if self.notification_open {
            return Err(errors::BUSY);
        }
        self.notification_open = true;
        Ok(())
    }

    pub fn notification_close(&mut self) -> Result<(), CellError> {
        self.require_ready()?;
        if !self.notification_open {
            return Err(errors::NOT_INITIALIZED);
        }
        self.notification_open = false;
        Ok(())
    }

    #[must_use]
    pub fn is_notification_open(&self) -> bool {
        self.notification_open
    }

    // ----------------- Search sessions -----------------

    /// `cellSearchStartContentSearch(searchType, sortKey, sortOrder,
    /// callback, userData)`. Only one session active at a time.
    pub fn start_content_search(
        &mut self,
        search_type: i32,
        sort_key: i32,
        sort_order: i32,
    ) -> Result<SearchId, CellError> {
        self.require_ready()?;
        if self.active.is_some() {
            return Err(errors::BUSY);
        }
        if !is_known_content_search_type(search_type) || search_type == CONTENTSEARCHTYPE_NONE {
            return Err(errors::NOT_SUPPORTED_SEARCH);
        }
        if !is_known_sort_key(sort_key) {
            return Err(errors::PARAM);
        }
        if !is_known_sort_order(sort_order) {
            return Err(errors::PARAM);
        }
        let id = self.next_id;
        self.next_id += 1;
        let content_type = match search_type {
            CONTENTSEARCHTYPE_MUSIC_ALL => CONTENTTYPE_MUSIC,
            CONTENTSEARCHTYPE_PHOTO_ALL => CONTENTTYPE_PHOTO,
            CONTENTSEARCHTYPE_VIDEO_ALL => CONTENTTYPE_VIDEO,
            _ => CONTENTTYPE_NONE,
        };
        let mut results: Vec<ContentId> = self
            .index
            .iter()
            .filter(|info| info.content_type == content_type)
            .map(|info| info.id.clone())
            .collect();
        self.sort_results(&mut results, sort_key, sort_order);
        self.active = Some(SearchSession {
            id,
            search_type,
            sort_key,
            sort_order,
            results,
            result_consumed: false,
        });
        Ok(id)
    }

    fn sort_results(&self, ids: &mut [ContentId], sort_key: i32, sort_order: i32) {
        if sort_key == SORTKEY_NONE || sort_order == SORTORDER_NONE {
            return;
        }
        let index = &self.index;
        ids.sort_by(|a, b| {
            let ea = index.iter().find(|i| i.id == *a);
            let eb = index.iter().find(|i| i.id == *b);
            let ord = match (ea, eb) {
                (Some(ia), Some(ib)) => match sort_key {
                    SORTKEY_TITLE => ia.title.cmp(&ib.title),
                    SORTKEY_IMPORTEDDATE | SORTKEY_MODIFIEDDATE => ia.duration.cmp(&ib.duration),
                    _ => ia.path.cmp(&ib.path),
                },
                _ => std::cmp::Ordering::Equal,
            };
            if sort_order == SORTORDER_DESCENDING { ord.reverse() } else { ord }
        });
    }

    /// `cellSearchCancel(searchId)`.
    pub fn cancel(&mut self, id: SearchId) -> Result<(), CellError> {
        self.require_ready()?;
        let active = self.active.as_ref().ok_or(errors::INVALID_SEARCHID)?;
        if active.id != id {
            return Err(errors::INVALID_SEARCHID);
        }
        self.active = None;
        Ok(())
    }

    /// `cellSearchEnd(searchId)`. Releases the session; idempotent-ish:
    /// calling twice returns INVALID_SEARCHID.
    pub fn end(&mut self, id: SearchId) -> Result<u32, CellError> {
        self.require_ready()?;
        let active = self.active.as_ref().ok_or(errors::INVALID_SEARCHID)?;
        if active.id != id {
            return Err(errors::INVALID_SEARCHID);
        }
        let count = u32::try_from(active.results.len()).unwrap_or(u32::MAX);
        self.active = None;
        Ok(count)
    }

    #[must_use]
    pub fn active_id(&self) -> Option<SearchId> {
        self.active.as_ref().map(|s| s.id)
    }

    // ----------------- Result lookups -----------------

    /// `cellSearchGetContentInfoByOffset(searchId, offset, contentType,
    /// contentId, searchType)`.
    pub fn content_info_by_offset(&self, id: SearchId, offset: u32) -> Result<&ContentInfo, CellError> {
        self.require_ready_const()?;
        let session = self.active.as_ref().ok_or(errors::INVALID_SEARCHID)?;
        if session.id != id {
            return Err(errors::INVALID_SEARCHID);
        }
        let idx = offset as usize;
        let content_id = session.results.get(idx).ok_or(errors::OUT_OF_RANGE)?;
        self.index.iter().find(|c| c.id == *content_id).ok_or(errors::CONTENT_NOT_FOUND)
    }

    /// `cellSearchGetContentInfoByContentId(contentId, contentType, ...)`.
    /// Does *not* require an active session — any id previously seen is
    /// looked up directly.
    pub fn content_info_by_id(&self, id: &ContentId) -> Result<&ContentInfo, CellError> {
        self.require_ready_const()?;
        self.index.iter().find(|c| c.id == *id).ok_or(errors::CONTENT_NOT_FOUND)
    }

    /// `cellSearchGetContentInfoGameComment(contentId, outComment)`.
    pub fn content_info_game_comment(&self, id: &ContentId) -> Result<&str, CellError> {
        self.require_ready_const()?;
        let info = self.index.iter().find(|c| c.id == *id).ok_or(errors::CONTENT_NOT_FOUND)?;
        if info.game_comment.is_empty() {
            return Err(errors::TAG);
        }
        if info.game_comment.len() > GAMECOMMENT_SIZE_MAX {
            return Err(errors::NO_MEMORY);
        }
        Ok(&info.game_comment)
    }

    fn require_ready(&self) -> Result<(), CellError> {
        match self.state {
            SearchState::Ready => Ok(()),
            SearchState::Uninitialized => Err(errors::NOT_INITIALIZED),
            SearchState::Initializing => Err(errors::BUSY),
            SearchState::Finalizing => Err(errors::FINALIZING),
        }
    }

    fn require_ready_const(&self) -> Result<(), CellError> {
        self.require_ready()
    }
}

impl Default for SearchManager {
    fn default() -> Self {
        Self::new()
    }
}

// =====================================================================
// Tests
// =====================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn initialized_manager_with_three() -> SearchManager {
        let mut m = SearchManager::new();
        m.initialize(MODE_NORMAL).unwrap();
        let mut a = ContentInfo::new(ContentId::from_u64(1), CONTENTTYPE_MUSIC, "/dev_hdd0/music/a.mp3");
        a.title = "Alpha".into();
        a.duration = 10;
        a.game_comment = "My comment A".into();
        let mut b = ContentInfo::new(ContentId::from_u64(2), CONTENTTYPE_MUSIC, "/dev_hdd0/music/b.mp3");
        b.title = "Bravo".into();
        b.duration = 20;
        let mut c = ContentInfo::new(ContentId::from_u64(3), CONTENTTYPE_PHOTO, "/dev_hdd0/photo/c.jpg");
        c.title = "Charlie".into();
        m.add_content(a);
        m.add_content(b);
        m.add_content(c);
        m
    }

    #[test]
    fn error_codes_byte_exact() {
        assert_eq!(errors::PARAM.0, 0x8002_C801);
        assert_eq!(errors::BUSY.0, 0x8002_C802);
        assert_eq!(errors::ALREADY_INITIALIZED.0, 0x8002_C805);
        assert_eq!(errors::NOT_INITIALIZED.0, 0x8002_C806);
        assert_eq!(errors::CONTENT_NOT_FOUND.0, 0x8002_C80A);
        assert_eq!(errors::INVALID_SEARCHID.0, 0x8002_C80D);
        assert_eq!(errors::DRM.0, 0x8002_C811);
        assert_eq!(errors::GENERIC.0, 0x8002_C8FF);
    }

    #[test]
    fn canceled_sentinel_is_1() {
        assert_eq!(CANCELED, 1);
    }

    #[test]
    fn size_constants_stable() {
        assert_eq!(CONTENT_ID_SIZE, 16);
        assert_eq!(TITLE_LEN_MAX, 384);
        assert_eq!(TAG_NUM_MAX, 6);
        assert_eq!(TAG_LEN_MAX, 63);
        assert_eq!(GAMECOMMENT_SIZE_MAX, 1024);
        assert_eq!(CONTENT_BUFFER_SIZE_MAX, 2048);
    }

    #[test]
    fn content_type_constants_stable() {
        assert_eq!(CONTENTTYPE_MUSIC, 1);
        assert_eq!(CONTENTTYPE_PHOTO, 3);
        assert_eq!(CONTENTTYPE_VIDEO, 5);
        assert_eq!(CONTENTTYPE_SCENE, 7);
    }

    #[test]
    fn event_constants_stable() {
        assert_eq!(EVENT_NOTIFICATION, 0);
        assert_eq!(EVENT_INITIALIZE_RESULT, 1);
        assert_eq!(EVENT_CONTENTSEARCH_RESULT, 5);
        assert_eq!(EVENT_SCENESEARCH_RESULT, 7);
    }

    #[test]
    fn initialize_from_fresh_transitions_to_ready() {
        let mut m = SearchManager::new();
        m.initialize(MODE_NORMAL).unwrap();
        assert_eq!(m.state(), &SearchState::Ready);
    }

    #[test]
    fn initialize_unknown_mode_rejected() {
        let mut m = SearchManager::new();
        assert_eq!(m.initialize(42), Err(errors::UNKNOWN_MODE));
    }

    #[test]
    fn initialize_twice_is_already_initialized() {
        let mut m = SearchManager::new();
        m.initialize(MODE_NORMAL).unwrap();
        assert_eq!(m.initialize(MODE_NORMAL), Err(errors::ALREADY_INITIALIZED));
    }

    #[test]
    fn finalize_without_init_is_not_initialized() {
        let mut m = SearchManager::new();
        assert_eq!(m.finalize(), Err(errors::NOT_INITIALIZED));
    }

    #[test]
    fn finalize_happy_path() {
        let mut m = SearchManager::new();
        m.initialize(MODE_NORMAL).unwrap();
        m.finalize().unwrap();
        assert_eq!(m.state(), &SearchState::Uninitialized);
    }

    #[test]
    fn finalize_with_active_session_is_busy() {
        let mut m = initialized_manager_with_three();
        m.start_content_search(CONTENTSEARCHTYPE_MUSIC_ALL, SORTKEY_DEFAULT, SORTORDER_ASCENDING).unwrap();
        assert_eq!(m.finalize(), Err(errors::BUSY));
    }

    #[test]
    fn notification_open_close_cycle() {
        let mut m = initialized_manager_with_three();
        m.notification_open().unwrap();
        assert!(m.is_notification_open());
        assert_eq!(m.notification_open(), Err(errors::BUSY));
        m.notification_close().unwrap();
        assert!(!m.is_notification_open());
    }

    #[test]
    fn notification_close_without_open_is_not_initialized() {
        let mut m = initialized_manager_with_three();
        assert_eq!(m.notification_close(), Err(errors::NOT_INITIALIZED));
    }

    #[test]
    fn start_content_search_music_returns_two_results() {
        let mut m = initialized_manager_with_three();
        let id = m
            .start_content_search(CONTENTSEARCHTYPE_MUSIC_ALL, SORTKEY_TITLE, SORTORDER_ASCENDING)
            .unwrap();
        assert_eq!(id, 1);
        let count = m.end(id).unwrap();
        assert_eq!(count, 2);
    }

    #[test]
    fn start_content_search_sort_by_title_ascending() {
        let mut m = initialized_manager_with_three();
        let id = m
            .start_content_search(CONTENTSEARCHTYPE_MUSIC_ALL, SORTKEY_TITLE, SORTORDER_ASCENDING)
            .unwrap();
        let first = m.content_info_by_offset(id, 0).unwrap();
        assert_eq!(first.title, "Alpha");
        let second = m.content_info_by_offset(id, 1).unwrap();
        assert_eq!(second.title, "Bravo");
    }

    #[test]
    fn start_content_search_sort_by_title_descending() {
        let mut m = initialized_manager_with_three();
        let id = m
            .start_content_search(CONTENTSEARCHTYPE_MUSIC_ALL, SORTKEY_TITLE, SORTORDER_DESCENDING)
            .unwrap();
        let first = m.content_info_by_offset(id, 0).unwrap();
        assert_eq!(first.title, "Bravo");
    }

    #[test]
    fn start_content_search_unknown_search_type_rejected() {
        let mut m = initialized_manager_with_three();
        assert_eq!(
            m.start_content_search(999, SORTKEY_NONE, SORTORDER_NONE),
            Err(errors::NOT_SUPPORTED_SEARCH)
        );
    }

    #[test]
    fn start_content_search_none_type_rejected() {
        let mut m = initialized_manager_with_three();
        assert_eq!(
            m.start_content_search(CONTENTSEARCHTYPE_NONE, SORTKEY_NONE, SORTORDER_NONE),
            Err(errors::NOT_SUPPORTED_SEARCH)
        );
    }

    #[test]
    fn start_content_search_while_busy_is_busy() {
        let mut m = initialized_manager_with_three();
        m.start_content_search(CONTENTSEARCHTYPE_MUSIC_ALL, SORTKEY_DEFAULT, SORTORDER_ASCENDING).unwrap();
        assert_eq!(
            m.start_content_search(CONTENTSEARCHTYPE_PHOTO_ALL, SORTKEY_DEFAULT, SORTORDER_ASCENDING),
            Err(errors::BUSY)
        );
    }

    #[test]
    fn start_content_search_without_init_not_initialized() {
        let mut m = SearchManager::new();
        assert_eq!(
            m.start_content_search(CONTENTSEARCHTYPE_MUSIC_ALL, SORTKEY_NONE, SORTORDER_NONE),
            Err(errors::NOT_INITIALIZED)
        );
    }

    #[test]
    fn cancel_active_session_succeeds() {
        let mut m = initialized_manager_with_three();
        let id = m.start_content_search(CONTENTSEARCHTYPE_MUSIC_ALL, SORTKEY_DEFAULT, SORTORDER_ASCENDING).unwrap();
        m.cancel(id).unwrap();
        assert_eq!(m.active_id(), None);
    }

    #[test]
    fn cancel_wrong_id_is_invalid_searchid() {
        let mut m = initialized_manager_with_three();
        let _id = m.start_content_search(CONTENTSEARCHTYPE_MUSIC_ALL, SORTKEY_DEFAULT, SORTORDER_ASCENDING).unwrap();
        assert_eq!(m.cancel(999), Err(errors::INVALID_SEARCHID));
    }

    #[test]
    fn cancel_without_session_is_invalid_searchid() {
        let mut m = initialized_manager_with_three();
        assert_eq!(m.cancel(1), Err(errors::INVALID_SEARCHID));
    }

    #[test]
    fn end_after_cancel_is_invalid_searchid() {
        let mut m = initialized_manager_with_three();
        let id = m.start_content_search(CONTENTSEARCHTYPE_MUSIC_ALL, SORTKEY_DEFAULT, SORTORDER_ASCENDING).unwrap();
        m.cancel(id).unwrap();
        assert_eq!(m.end(id), Err(errors::INVALID_SEARCHID));
    }

    #[test]
    fn content_info_by_offset_out_of_range() {
        let mut m = initialized_manager_with_three();
        let id = m.start_content_search(CONTENTSEARCHTYPE_MUSIC_ALL, SORTKEY_NONE, SORTORDER_NONE).unwrap();
        assert_eq!(m.content_info_by_offset(id, 99).err(), Some(errors::OUT_OF_RANGE));
    }

    #[test]
    fn content_info_by_id_returns_metadata() {
        let m = initialized_manager_with_three();
        let target = ContentId::from_u64(2);
        let info = m.content_info_by_id(&target).unwrap();
        assert_eq!(info.title, "Bravo");
    }

    #[test]
    fn content_info_by_id_unknown_returns_content_not_found() {
        let m = initialized_manager_with_three();
        let target = ContentId::from_u64(999);
        assert_eq!(m.content_info_by_id(&target).err(), Some(errors::CONTENT_NOT_FOUND));
    }

    #[test]
    fn content_info_game_comment_returns_value_when_set() {
        let m = initialized_manager_with_three();
        let target = ContentId::from_u64(1);
        assert_eq!(m.content_info_game_comment(&target), Ok("My comment A"));
    }

    #[test]
    fn content_info_game_comment_returns_tag_err_when_empty() {
        let m = initialized_manager_with_three();
        let target = ContentId::from_u64(2);
        assert_eq!(m.content_info_game_comment(&target), Err(errors::TAG));
    }

    #[test]
    fn content_id_from_u64_is_deterministic() {
        assert_eq!(ContentId::from_u64(42), ContentId::from_u64(42));
        assert_ne!(ContentId::from_u64(1), ContentId::from_u64(2));
    }

    #[test]
    fn multiple_searches_increment_ids_monotonically() {
        let mut m = initialized_manager_with_three();
        let id1 = m.start_content_search(CONTENTSEARCHTYPE_MUSIC_ALL, SORTKEY_NONE, SORTORDER_NONE).unwrap();
        m.end(id1).unwrap();
        let id2 = m.start_content_search(CONTENTSEARCHTYPE_PHOTO_ALL, SORTKEY_NONE, SORTORDER_NONE).unwrap();
        assert_eq!(id2, id1 + 1);
        m.end(id2).unwrap();
    }

    #[test]
    fn is_known_content_search_type_accepts_known_rejects_others() {
        assert!(is_known_content_search_type(CONTENTSEARCHTYPE_NONE));
        assert!(is_known_content_search_type(CONTENTSEARCHTYPE_VIDEO_ALL));
        assert!(!is_known_content_search_type(42));
        assert!(!is_known_content_search_type(-1));
    }

    #[test]
    fn calls_while_finalizing_are_rejected() {
        // Finalizing state is not directly observable via API, but we
        // simulate it to prove the matchwarning paths exist.
        let mut m = SearchManager::new();
        m.initialize(MODE_NORMAL).unwrap();
        // Manually flip state to cover the branches.
        m.state = SearchState::Finalizing;
        assert_eq!(m.initialize(MODE_NORMAL), Err(errors::FINALIZING));
        assert_eq!(m.finalize(), Err(errors::FINALIZING));
        assert_eq!(
            m.start_content_search(CONTENTSEARCHTYPE_MUSIC_ALL, SORTKEY_NONE, SORTORDER_NONE),
            Err(errors::FINALIZING)
        );
    }

    #[test]
    fn full_search_flow_smoke() {
        let mut m = SearchManager::new();
        m.initialize(MODE_NORMAL).unwrap();
        let mut v1 = ContentInfo::new(ContentId::from_u64(10), CONTENTTYPE_VIDEO, "/dev_hdd0/video/v1.mp4");
        v1.title = "Movie 1".into();
        v1.game_comment = "AAA title".into();
        m.add_content(v1);
        let mut v2 = ContentInfo::new(ContentId::from_u64(11), CONTENTTYPE_VIDEO, "/dev_hdd0/video/v2.mp4");
        v2.title = "Movie 2".into();
        m.add_content(v2);
        m.notification_open().unwrap();
        let id = m
            .start_content_search(CONTENTSEARCHTYPE_VIDEO_ALL, SORTKEY_TITLE, SORTORDER_ASCENDING)
            .unwrap();
        let first = m.content_info_by_offset(id, 0).unwrap().title.clone();
        let second = m.content_info_by_offset(id, 1).unwrap().title.clone();
        assert_eq!(first, "Movie 1");
        assert_eq!(second, "Movie 2");
        assert_eq!(m.end(id).unwrap(), 2);
        m.notification_close().unwrap();
        m.finalize().unwrap();
    }
}
