//! Rust port of `rpcs3/Emu/Cell/Modules/cellMusicSelectionContext.cpp`.
//!
//! The C++ file is explicitly documented at cpp:8 as _"just a helper and
//! not a real cell entity"_ — it exposes no PRX entry points. The
//! `music_selection_context` struct is consumed by the higher-level
//! `cellMusic` / `cellMusicDecode` / `cellMusic2` modules to represent
//! the playlist the game currently has selected (see `cellMusic.h:142`).
//!
//! Scope of this port:
//!
//!  - Serialization of the `CellMusicSelectionContext` 2048-byte blob
//!    (`set` / `get` at cpp:12-41) — `"SUS\0"` magic + hash payload.
//!  - `step_track` navigation (cpp:281-371) covering all four repeat
//!    modes plus the `ContextOption::Shuffle` reshuffle trigger.
//!  - `set_track` playlist lookup (cpp:258-279) with `ends_with`
//!    matching.
//!  - Helper constants: `max_depth`, `target_file_type`, `target_version`,
//!    `CELL_MUSIC_SELECTION_CONTEXT_SIZE`, `magic`.
//!  - The `get_next_hash` static counter (cpp:56-60) and the
//!    `context_to_hex` debug helper (cpp:62-72).
//!
//! Out of scope: YAML persistence (`create_playlist` / `load_playlist`,
//! cpp:121-256) and directory scanning (`set_playlist`, cpp:86-119) both
//! depend on `fs::` host filesystem APIs — the Rust port stays
//! `no_std` + `alloc` and takes the playlist vector directly from the
//! caller.

#![no_std]
#![forbid(unsafe_code)]

extern crate alloc;

use alloc::string::String;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicU64, Ordering};

/// Byte-exact at `cellMusic.h:129` — the serialized context must fit in
/// exactly this many bytes.
pub const CELL_MUSIC_SELECTION_CONTEXT_SIZE: usize = 2048;

/// Byte-exact at `cellMusic.h:145` — `char magic[4] = "SUS"`, i.e.
/// `['S', 'U', 'S', '\0']` because `"SUS"` as a C string literal
/// auto-nul-terminates and fills the fourth byte with 0.
pub const MAGIC: [u8; 4] = *b"SUS\0";

/// Byte-exact at `cellMusic.h:154` — the ceiling on directory depth the
/// C++ `set_playlist` walks when scanning for media files.
pub const MAX_DEPTH: u32 = 2;

/// Byte-exact at `cellMusic.h:155` — the `FileType` value written into
/// every playlist YAML so that loaders can detect an incompatible blob.
pub const TARGET_FILE_TYPE: &str = "Music Playlist";

/// Byte-exact at `cellMusic.h:156`.
pub const TARGET_VERSION: &str = "1.0";

/// Sentinel returned by `step_track` when the playlist is exhausted
/// under a non-looping repeat mode. Matches the `umax` alias used in
/// cpp:286/302/312/350.
pub const UMAX: u32 = u32::MAX;

// --- Mirror enums (cellSearch.h:69-78/210-220) --------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum CellSearchContentType {
    None = 0,
    Music = 1,
    MusicList = 2,
    Photo = 3,
    PhotoList = 4,
    Video = 5,
    VideoList = 6,
    Scene = 7,
}

impl Default for CellSearchContentType {
    fn default() -> Self {
        Self::Music
    }
}

impl CellSearchContentType {
    #[must_use]
    pub const fn from_u32(v: u32) -> Option<Self> {
        match v {
            0 => Some(Self::None),
            1 => Some(Self::Music),
            2 => Some(Self::MusicList),
            3 => Some(Self::Photo),
            4 => Some(Self::PhotoList),
            5 => Some(Self::Video),
            6 => Some(Self::VideoList),
            7 => Some(Self::Scene),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum CellSearchRepeatMode {
    None = 0,
    Repeat1 = 1,
    All = 2,
    NoRepeat1 = 3,
}

impl Default for CellSearchRepeatMode {
    fn default() -> Self {
        Self::None
    }
}

impl CellSearchRepeatMode {
    #[must_use]
    pub const fn from_u32(v: u32) -> Option<Self> {
        match v {
            0 => Some(Self::None),
            1 => Some(Self::Repeat1),
            2 => Some(Self::All),
            3 => Some(Self::NoRepeat1),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum CellSearchContextOption {
    None = 0,
    Shuffle = 1,
}

impl Default for CellSearchContextOption {
    fn default() -> Self {
        Self::None
    }
}

impl CellSearchContextOption {
    #[must_use]
    pub const fn from_u32(v: u32) -> Option<Self> {
        match v {
            0 => Some(Self::None),
            1 => Some(Self::Shuffle),
            _ => None,
        }
    }
}

// --- Raw 2048-byte wire struct ------------------------------------------

/// Mirror of `CellMusicSelectionContext` (cellMusic.h:135-140): an opaque
/// 2048-byte blob that the firmware treats as an opaque token and that
/// the RPCS3 helper hijacks for its own layout (magic + hash).
#[derive(Clone)]
#[repr(C)]
pub struct CellMusicSelectionContext {
    pub data: [u8; CELL_MUSIC_SELECTION_CONTEXT_SIZE],
}

impl core::fmt::Debug for CellMusicSelectionContext {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        // The array is too large to blast into a regular Debug output —
        // print a summary instead so test failures stay readable.
        f.debug_struct("CellMusicSelectionContext")
            .field("data_len", &self.data.len())
            .field("magic", &&self.data[..4])
            .finish()
    }
}

impl PartialEq for CellMusicSelectionContext {
    fn eq(&self, other: &Self) -> bool {
        self.data[..] == other.data[..]
    }
}
impl Eq for CellMusicSelectionContext {}

impl Default for CellMusicSelectionContext {
    fn default() -> Self {
        Self {
            data: [0; CELL_MUSIC_SELECTION_CONTEXT_SIZE],
        }
    }
}

// --- Hash counter (cpp:56-60) -------------------------------------------

static HASH_COUNTER: AtomicU64 = AtomicU64::new(0);

/// `music_selection_context::get_next_hash` (cpp:56-60) — a
/// monotonically-incrementing counter used to name on-disk playlists.
/// The C++ uses a `static u64 hash_counter = 0;` local; the Rust port
/// uses a global `AtomicU64` so it stays threadsafe and the same call
/// site across every `MusicSelectionContext` instance bumps the shared
/// counter.
#[must_use]
pub fn get_next_hash() -> String {
    let n = HASH_COUNTER.fetch_add(1, Ordering::Relaxed);
    alloc::format!("music_selection_context_{n}")
}

/// `music_selection_context::context_to_hex` (cpp:62-72). Renders every
/// byte as `" %.2x"` — leading space + 2-char lowercase hex, matching
/// the `fmt::append(dahex, " %.2x", ...)` call at cpp:68.
#[must_use]
pub fn context_to_hex(ctx: &CellMusicSelectionContext) -> String {
    let mut s = String::with_capacity(CELL_MUSIC_SELECTION_CONTEXT_SIZE * 3);
    for byte in ctx.data.iter() {
        s.push_str(&alloc::format!(" {byte:02x}"));
    }
    s
}

// --- Main struct --------------------------------------------------------

/// Rust mirror of `music_selection_context` (cellMusic.h:142-179). The
/// `valid` flag on the C++ side is a `bool` guarded by `atomic_storage`;
/// the Rust port keeps it as a plain field because every access path we
/// model is single-threaded — higher layers (`cellMusic`) own the mutex.
#[derive(Debug, Default, Clone)]
pub struct MusicSelectionContext {
    pub valid: bool,
    pub magic: [u8; 4],
    pub hash: String,
    pub content_type: CellSearchContentType,
    pub repeat_mode: CellSearchRepeatMode,
    pub context_option: CellSearchContextOption,
    pub first_track: u32,
    pub current_track: u32,
    pub playlist: Vec<String>,
    /// Counter exposed for tests — bumps each time `step_track` triggers
    /// the `Shuffle + All` reshuffle branch at cpp:359-367.
    pub shuffle_triggered: u32,
}

impl MusicSelectionContext {
    /// Build an empty context mirroring the C++ default-constructor
    /// values at cellMusic.h:144-151.
    #[must_use]
    pub fn new() -> Self {
        Self {
            valid: false,
            magic: MAGIC,
            hash: String::new(),
            content_type: CellSearchContentType::Music,
            repeat_mode: CellSearchRepeatMode::None,
            context_option: CellSearchContextOption::None,
            first_track: 0,
            current_track: 0,
            playlist: Vec::new(),
            shuffle_triggered: 0,
        }
    }

    /// `music_selection_context::set` (cpp:12-23). Validates the magic
    /// prefix and extracts the hash payload. Unlike the C++ version,
    /// the Rust port does **not** call `load_playlist()` — the caller is
    /// expected to hand us the playlist directly (see the module doc).
    /// Returns `false` if the magic prefix does not match,
    /// mirroring cpp:14-17.
    pub fn set(&mut self, in_ctx: &CellMusicSelectionContext) -> bool {
        if in_ctx.data[..MAGIC.len()] != MAGIC {
            return false;
        }
        let start = MAGIC.len();
        let end = in_ctx.data[start..]
            .iter()
            .position(|&b| b == 0)
            .map(|n| start + n)
            .unwrap_or(CELL_MUSIC_SELECTION_CONTEXT_SIZE);
        self.hash = match core::str::from_utf8(&in_ctx.data[start..end]) {
            Ok(s) => s.into(),
            Err(_) => return false,
        };
        true
    }

    /// `music_selection_context::get` (cpp:25-41). Serializes back into
    /// a 2048-byte blob: magic at offset 0, hash starting at offset 4.
    ///
    /// Mirrors the `fmt::throw_exception` at cpp:29 by returning `None`
    /// if the combined payload wouldn't fit — the caller decides how to
    /// surface that (the real firmware aborts, but the port keeps panics
    /// out of library code).
    #[must_use]
    pub fn get(&self) -> Option<CellMusicSelectionContext> {
        if self.hash.len() + MAGIC.len() > CELL_MUSIC_SELECTION_CONTEXT_SIZE {
            return None;
        }
        let mut out = CellMusicSelectionContext::default();
        out.data[..MAGIC.len()].copy_from_slice(&MAGIC);
        let hash_bytes = self.hash.as_bytes();
        out.data[MAGIC.len()..MAGIC.len() + hash_bytes.len()].copy_from_slice(hash_bytes);
        Some(out)
    }

    /// Assign the playlist directly — the Rust equivalent of the middle
    /// of cpp:86-119 once the directory walk is stripped out. Sets
    /// `content_type` to `MusicList` for many tracks and `Music` for
    /// single entries, matching cpp:95/111, and flips `valid = true`
    /// (cpp:118).
    pub fn set_playlist<I, S>(&mut self, tracks: I)
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.playlist = tracks.into_iter().map(Into::into).collect();
        self.content_type = if self.playlist.len() <= 1 {
            CellSearchContentType::Music
        } else {
            CellSearchContentType::MusicList
        };
        self.valid = !self.playlist.is_empty();
        self.current_track = 0;
        self.first_track = 0;
    }

    /// `music_selection_context::set_track` (cpp:258-279). Finds the
    /// track whose path ends with `track` (the firmware uses that form
    /// because the VFS-translated playlist prefixes the tracks with
    /// `/dev_hdd0/music/…` while the caller often has the raw host
    /// path). No-op on empty input or an empty playlist — matches
    /// cpp:260/262-266 early returns.
    pub fn set_track(&mut self, track: &str) -> bool {
        if track.is_empty() || self.playlist.is_empty() {
            return false;
        }
        for (i, entry) in self.playlist.iter().enumerate() {
            if track.ends_with(entry.as_str()) {
                let idx = i as u32;
                self.first_track = idx;
                self.current_track = idx;
                return true;
            }
        }
        false
    }

    /// `music_selection_context::step_track` (cpp:281-371). Returns the
    /// new `current_track` index or [`UMAX`] when the playlist is
    /// exhausted / in `NoRepeat1` mode. The `Shuffle` + `All`
    /// combination triggers a reshuffle each time the cursor wraps —
    /// the caller-supplied `shuffler` callback owns the actual
    /// permutation so the port stays deterministic in tests.
    pub fn step_track(&mut self, next: bool, shuffler: &mut dyn FnMut(&mut Vec<String>)) -> u32 {
        if self.playlist.is_empty() {
            self.current_track = UMAX;
            return UMAX;
        }
        match self.repeat_mode {
            CellSearchRepeatMode::None => {
                if next {
                    self.current_track = self.current_track.wrapping_add(1);
                    if self.current_track as usize >= self.playlist.len() {
                        self.current_track = UMAX;
                        return UMAX;
                    }
                } else {
                    if self.current_track == 0 {
                        self.current_track = UMAX;
                        return UMAX;
                    }
                    self.current_track -= 1;
                }
            }
            CellSearchRepeatMode::Repeat1 => {
                // Keep decoding the same track (cpp:320-324).
            }
            CellSearchRepeatMode::All => {
                let len = self.playlist.len() as u32;
                if next {
                    self.current_track = (self.current_track + 1) % len;
                } else if self.current_track == 0 {
                    self.current_track = len - 1;
                } else {
                    self.current_track -= 1;
                }
            }
            CellSearchRepeatMode::NoRepeat1 => {
                // cpp:346-352 — terminal condition.
                self.current_track = UMAX;
                return UMAX;
            }
        }

        // Shuffle trigger cpp:359-367.
        if self.context_option == CellSearchContextOption::Shuffle
            && self.repeat_mode == CellSearchRepeatMode::All
            && self.playlist.len() > 1
        {
            let wrapped = if next {
                self.current_track == 0
            } else {
                self.current_track as usize == self.playlist.len() - 1
            };
            if wrapped {
                shuffler(&mut self.playlist);
                self.shuffle_triggered = self.shuffle_triggered.saturating_add(1);
            }
        }

        self.current_track
    }

    /// Convenience wrapper — calls [`Self::step_track`] with a no-op
    /// shuffler so the caller doesn't have to plumb one through for
    /// non-Shuffle modes.
    pub fn step_track_no_shuffle(&mut self, next: bool) -> u32 {
        self.step_track(next, &mut |_| {})
    }

    /// Lightweight `Debug` / `to_string` clone of cpp:43-54 — produces
    /// exactly the single-line prefix the C++ writes before the
    /// per-track lines so tests can assert the header bits.
    #[must_use]
    pub fn to_string_header(&self) -> String {
        alloc::format!(
            ".magic='{}', .content_type={}, .repeat_mode={}, .context_option={}, .first_track={}, .tracks={}, .hash='{}'",
            core::str::from_utf8(&self.magic[..3]).unwrap_or("?"),
            self.content_type as u32,
            self.repeat_mode as u32,
            self.context_option as u32,
            self.first_track,
            self.playlist.len(),
            self.hash,
        )
    }
}

impl From<&MusicSelectionContext> for bool {
    /// Mirrors `operator bool()` at cellMusic.h:172-175 — returns the
    /// `valid` flag.
    fn from(ctx: &MusicSelectionContext) -> bool {
        ctx.valid
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::string::ToString;

    #[test]
    fn constants_byte_exact() {
        assert_eq!(CELL_MUSIC_SELECTION_CONTEXT_SIZE, 2048);
        assert_eq!(MAGIC, *b"SUS\0");
        assert_eq!(MAX_DEPTH, 2);
        assert_eq!(TARGET_FILE_TYPE, "Music Playlist");
        assert_eq!(TARGET_VERSION, "1.0");
        assert_eq!(UMAX, u32::MAX);
    }

    #[test]
    fn content_type_discriminants_byte_exact() {
        assert_eq!(CellSearchContentType::None as u32, 0);
        assert_eq!(CellSearchContentType::Music as u32, 1);
        assert_eq!(CellSearchContentType::MusicList as u32, 2);
        assert_eq!(CellSearchContentType::Photo as u32, 3);
        assert_eq!(CellSearchContentType::PhotoList as u32, 4);
        assert_eq!(CellSearchContentType::Video as u32, 5);
        assert_eq!(CellSearchContentType::VideoList as u32, 6);
        assert_eq!(CellSearchContentType::Scene as u32, 7);
        assert_eq!(CellSearchContentType::from_u32(8), None);
    }

    #[test]
    fn repeat_mode_discriminants_byte_exact() {
        assert_eq!(CellSearchRepeatMode::None as u32, 0);
        assert_eq!(CellSearchRepeatMode::Repeat1 as u32, 1);
        assert_eq!(CellSearchRepeatMode::All as u32, 2);
        assert_eq!(CellSearchRepeatMode::NoRepeat1 as u32, 3);
        assert_eq!(CellSearchRepeatMode::from_u32(4), None);
    }

    #[test]
    fn context_option_discriminants_byte_exact() {
        assert_eq!(CellSearchContextOption::None as u32, 0);
        assert_eq!(CellSearchContextOption::Shuffle as u32, 1);
        assert_eq!(CellSearchContextOption::from_u32(2), None);
    }

    #[test]
    fn new_defaults_match_cpp() {
        let ctx = MusicSelectionContext::new();
        assert!(!ctx.valid);
        assert_eq!(ctx.magic, *b"SUS\0");
        assert_eq!(ctx.content_type, CellSearchContentType::Music);
        assert_eq!(ctx.repeat_mode, CellSearchRepeatMode::None);
        assert_eq!(ctx.context_option, CellSearchContextOption::None);
        assert_eq!(ctx.first_track, 0);
        assert_eq!(ctx.current_track, 0);
        assert!(ctx.playlist.is_empty());
    }

    #[test]
    fn set_rejects_bad_magic() {
        let mut wire = CellMusicSelectionContext::default();
        wire.data[..4].copy_from_slice(b"BAD\0");
        let mut ctx = MusicSelectionContext::new();
        assert!(!ctx.set(&wire));
    }

    #[test]
    fn set_extracts_hash_up_to_nul() {
        let mut wire = CellMusicSelectionContext::default();
        wire.data[..4].copy_from_slice(&MAGIC);
        let hash = b"music_selection_context_7";
        wire.data[4..4 + hash.len()].copy_from_slice(hash);
        let mut ctx = MusicSelectionContext::new();
        assert!(ctx.set(&wire));
        assert_eq!(ctx.hash, "music_selection_context_7");
    }

    #[test]
    fn set_empty_hash_ok() {
        let mut wire = CellMusicSelectionContext::default();
        wire.data[..4].copy_from_slice(&MAGIC);
        let mut ctx = MusicSelectionContext::new();
        assert!(ctx.set(&wire));
        assert_eq!(ctx.hash, "");
    }

    #[test]
    fn get_roundtrip_matches_layout() {
        let mut ctx = MusicSelectionContext::new();
        ctx.hash = "hello".to_string();
        let wire = ctx.get().unwrap();
        assert_eq!(&wire.data[..4], &MAGIC);
        assert_eq!(&wire.data[4..9], b"hello");
        // Everything past the hash is zero-filled.
        assert!(wire.data[9..].iter().all(|&b| b == 0));
    }

    #[test]
    fn get_rejects_oversize_hash() {
        let mut ctx = MusicSelectionContext::new();
        ctx.hash = "x".repeat(CELL_MUSIC_SELECTION_CONTEXT_SIZE);
        assert!(ctx.get().is_none());
    }

    #[test]
    fn set_then_get_roundtrip() {
        let mut ctx1 = MusicSelectionContext::new();
        ctx1.hash = "music_selection_context_42".to_string();
        let wire = ctx1.get().unwrap();

        let mut ctx2 = MusicSelectionContext::new();
        assert!(ctx2.set(&wire));
        assert_eq!(ctx2.hash, "music_selection_context_42");
    }

    #[test]
    fn get_next_hash_is_monotonic() {
        let a = get_next_hash();
        let b = get_next_hash();
        // Extract the numeric suffix and assert a < b. The counter is
        // process-global so we only compare relative ordering.
        let parse = |s: &str| -> u64 {
            s.rsplit('_')
                .next()
                .unwrap()
                .parse()
                .unwrap()
        };
        assert!(parse(&a) < parse(&b));
        assert!(b.starts_with("music_selection_context_"));
    }

    #[test]
    fn context_to_hex_format() {
        let mut ctx = CellMusicSelectionContext::default();
        ctx.data[0] = 0xAB;
        ctx.data[1] = 0xCD;
        ctx.data[2] = 0x12;
        let hex = context_to_hex(&ctx);
        assert!(hex.starts_with(" ab cd 12"));
        // Every remaining byte is 0x00.
        assert!(hex.contains(" 00 00 00"));
        // Exact length: 3 chars per byte.
        assert_eq!(hex.len(), CELL_MUSIC_SELECTION_CONTEXT_SIZE * 3);
    }

    #[test]
    fn set_playlist_single_entry_is_music() {
        let mut ctx = MusicSelectionContext::new();
        ctx.set_playlist(["/dev_hdd0/music/track1.mp3"]);
        assert_eq!(ctx.content_type, CellSearchContentType::Music);
        assert!(ctx.valid);
        assert_eq!(ctx.playlist.len(), 1);
    }

    #[test]
    fn set_playlist_multiple_entries_is_musiclist() {
        let mut ctx = MusicSelectionContext::new();
        ctx.set_playlist(["a.mp3", "b.mp3", "c.mp3"]);
        assert_eq!(ctx.content_type, CellSearchContentType::MusicList);
        assert_eq!(ctx.playlist.len(), 3);
    }

    #[test]
    fn set_playlist_empty_keeps_invalid() {
        let mut ctx = MusicSelectionContext::new();
        ctx.set_playlist(core::iter::empty::<&str>());
        assert!(!ctx.valid);
        assert!(ctx.playlist.is_empty());
    }

    #[test]
    fn set_track_finds_by_suffix_match() {
        let mut ctx = MusicSelectionContext::new();
        ctx.set_playlist(["/a.mp3", "/b.mp3", "/c.mp3"]);
        assert!(ctx.set_track("/host/dir/b.mp3"));
        assert_eq!(ctx.current_track, 1);
        assert_eq!(ctx.first_track, 1);
    }

    #[test]
    fn set_track_empty_is_noop() {
        let mut ctx = MusicSelectionContext::new();
        ctx.set_playlist(["a"]);
        assert!(!ctx.set_track(""));
    }

    #[test]
    fn set_track_no_match_returns_false() {
        let mut ctx = MusicSelectionContext::new();
        ctx.set_playlist(["a", "b"]);
        assert!(!ctx.set_track("x"));
    }

    #[test]
    fn set_track_empty_playlist_returns_false() {
        let mut ctx = MusicSelectionContext::new();
        assert!(!ctx.set_track("any"));
    }

    #[test]
    fn step_track_empty_playlist_returns_umax() {
        let mut ctx = MusicSelectionContext::new();
        assert_eq!(ctx.step_track_no_shuffle(true), UMAX);
        assert_eq!(ctx.current_track, UMAX);
    }

    #[test]
    fn step_track_repeat_none_advances_until_end() {
        let mut ctx = MusicSelectionContext::new();
        ctx.set_playlist(["a", "b", "c"]);
        ctx.repeat_mode = CellSearchRepeatMode::None;
        assert_eq!(ctx.step_track_no_shuffle(true), 1);
        assert_eq!(ctx.step_track_no_shuffle(true), 2);
        assert_eq!(ctx.step_track_no_shuffle(true), UMAX);
    }

    #[test]
    fn step_track_repeat_none_prev_from_start_is_umax() {
        let mut ctx = MusicSelectionContext::new();
        ctx.set_playlist(["a", "b"]);
        ctx.repeat_mode = CellSearchRepeatMode::None;
        assert_eq!(ctx.step_track_no_shuffle(false), UMAX);
    }

    #[test]
    fn step_track_repeat1_stays_on_current() {
        let mut ctx = MusicSelectionContext::new();
        ctx.set_playlist(["a", "b"]);
        ctx.repeat_mode = CellSearchRepeatMode::Repeat1;
        ctx.current_track = 1;
        assert_eq!(ctx.step_track_no_shuffle(true), 1);
        assert_eq!(ctx.step_track_no_shuffle(false), 1);
    }

    #[test]
    fn step_track_all_wraps_forward() {
        let mut ctx = MusicSelectionContext::new();
        ctx.set_playlist(["a", "b", "c"]);
        ctx.repeat_mode = CellSearchRepeatMode::All;
        ctx.current_track = 2;
        assert_eq!(ctx.step_track_no_shuffle(true), 0);
    }

    #[test]
    fn step_track_all_wraps_backward() {
        let mut ctx = MusicSelectionContext::new();
        ctx.set_playlist(["a", "b", "c"]);
        ctx.repeat_mode = CellSearchRepeatMode::All;
        ctx.current_track = 0;
        assert_eq!(ctx.step_track_no_shuffle(false), 2);
    }

    #[test]
    fn step_track_norepeat1_always_umax() {
        let mut ctx = MusicSelectionContext::new();
        ctx.set_playlist(["a", "b"]);
        ctx.repeat_mode = CellSearchRepeatMode::NoRepeat1;
        assert_eq!(ctx.step_track_no_shuffle(true), UMAX);
        assert_eq!(ctx.current_track, UMAX);
    }

    #[test]
    fn step_track_shuffle_all_triggers_on_wrap_forward() {
        let mut ctx = MusicSelectionContext::new();
        ctx.set_playlist(["a", "b", "c"]);
        ctx.repeat_mode = CellSearchRepeatMode::All;
        ctx.context_option = CellSearchContextOption::Shuffle;
        ctx.current_track = 2;
        let mut calls = 0u32;
        let mut hook = |list: &mut Vec<String>| {
            calls += 1;
            list.rotate_left(1);
        };
        let idx = ctx.step_track(true, &mut hook);
        assert_eq!(idx, 0);
        assert_eq!(calls, 1);
        assert_eq!(ctx.shuffle_triggered, 1);
    }

    #[test]
    fn step_track_shuffle_all_triggers_on_wrap_backward() {
        let mut ctx = MusicSelectionContext::new();
        ctx.set_playlist(["a", "b", "c"]);
        ctx.repeat_mode = CellSearchRepeatMode::All;
        ctx.context_option = CellSearchContextOption::Shuffle;
        ctx.current_track = 0;
        let mut calls = 0u32;
        let mut hook = |_: &mut Vec<String>| calls += 1;
        let idx = ctx.step_track(false, &mut hook);
        assert_eq!(idx, 2);
        assert_eq!(calls, 1);
    }

    #[test]
    fn step_track_shuffle_no_trigger_without_wrap() {
        let mut ctx = MusicSelectionContext::new();
        ctx.set_playlist(["a", "b", "c"]);
        ctx.repeat_mode = CellSearchRepeatMode::All;
        ctx.context_option = CellSearchContextOption::Shuffle;
        ctx.current_track = 0;
        let mut calls = 0u32;
        ctx.step_track(true, &mut |_| calls += 1);
        assert_eq!(calls, 0);
    }

    #[test]
    fn step_track_shuffle_single_track_no_trigger() {
        let mut ctx = MusicSelectionContext::new();
        ctx.set_playlist(["only"]);
        ctx.repeat_mode = CellSearchRepeatMode::All;
        ctx.context_option = CellSearchContextOption::Shuffle;
        let mut calls = 0u32;
        ctx.step_track(true, &mut |_| calls += 1);
        assert_eq!(calls, 0);
    }

    #[test]
    fn to_string_header_includes_all_fields() {
        let mut ctx = MusicSelectionContext::new();
        ctx.set_playlist(["a", "b"]);
        ctx.hash = "H".to_string();
        ctx.repeat_mode = CellSearchRepeatMode::All;
        ctx.context_option = CellSearchContextOption::Shuffle;
        let s = ctx.to_string_header();
        assert!(s.contains(".magic='SUS'"));
        assert!(s.contains(".content_type=2"));
        assert!(s.contains(".repeat_mode=2"));
        assert!(s.contains(".context_option=1"));
        assert!(s.contains(".tracks=2"));
        assert!(s.contains(".hash='H'"));
    }

    #[test]
    fn valid_flag_is_exposed_via_bool_conversion() {
        let mut ctx = MusicSelectionContext::new();
        assert!(!bool::from(&ctx));
        ctx.set_playlist(["track"]);
        assert!(bool::from(&ctx));
    }

    #[test]
    fn full_music_selection_context_lifecycle_smoke() {
        // 1. Caller scans a directory (simulated by a pre-filled Vec)
        //    then installs the tracks + picks an initial position.
        let mut ctx = MusicSelectionContext::new();
        ctx.set_playlist([
            "/dev_hdd0/music/track_1.mp3",
            "/dev_hdd0/music/track_2.mp3",
            "/dev_hdd0/music/track_3.mp3",
        ]);
        assert_eq!(ctx.content_type, CellSearchContentType::MusicList);
        ctx.hash = get_next_hash();
        ctx.repeat_mode = CellSearchRepeatMode::All;
        ctx.context_option = CellSearchContextOption::Shuffle;
        assert!(ctx.set_track("/host/stuff/dev_hdd0/music/track_2.mp3"));
        assert_eq!(ctx.current_track, 1);

        // 2. Serialise to wire format, round-trip back.
        let wire = ctx.get().unwrap();
        assert_eq!(&wire.data[..4], &MAGIC);

        let mut peer = MusicSelectionContext::new();
        assert!(peer.set(&wire));
        assert_eq!(peer.hash, ctx.hash);

        // 3. Drive step_track over a full ring — the Shuffle branch
        //    must fire exactly once when we wrap past index 0.
        let mut shuffle_calls = 0u32;
        let before = ctx.playlist.clone();
        let _ = ctx.step_track(true, &mut |_| shuffle_calls += 1); // 1 -> 2
        let _ = ctx.step_track(true, &mut |list| {
            shuffle_calls += 1;
            list.rotate_left(1);
        }); // 2 -> 0 (wrap, shuffle)
        assert_eq!(shuffle_calls, 1);
        assert_eq!(ctx.shuffle_triggered, 1);
        assert_ne!(ctx.playlist, before);

        // 4. Switch to NoRepeat1 — the next call terminates the stream.
        ctx.repeat_mode = CellSearchRepeatMode::NoRepeat1;
        assert_eq!(ctx.step_track_no_shuffle(true), UMAX);
    }
}
