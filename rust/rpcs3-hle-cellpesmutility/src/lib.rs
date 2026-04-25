//! `rpcs3-hle-cellpesmutility` — PS3 PESM movie-recording encryption HLE.
//!
//! Ports `rpcs3/Emu/Cell/Modules/cellPesmUtility.cpp` (113 linhas).
//! Every entry point is `UNIMPLEMENTED_FUNC` in the C++ source; the
//! Rust port adds an observable lifecycle FSM + a sample-encryption
//! counter so higher layers can drive the library through Initialize
//! → OpenDevice → LoadAsync → PrepareRec → StartMovieRec → encrypt*
//! → EndMovieRec → UnloadAsync → CloseDevice → Finalize.
//!
//! ## Entry points covered
//!
//! | C++ function                 | Rust wrapper                      |
//! |------------------------------|-----------------------------------|
//! | `cellPesmInitialize`         | [`Pesm::initialize`]              |
//! | `cellPesmFinalize` / `Finalize2` | [`Pesm::finalize`] / [`Pesm::finalize2`] |
//! | `cellPesmOpenDevice` / `CloseDevice` | [`Pesm::open_device`] / [`Pesm::close_device`] |
//! | `cellPesmLoadAsync` / `UnloadAsync`  | [`Pesm::load_async`] / [`Pesm::unload_async`]  |
//! | `cellPesmInitEntry` / `InitEntry2`   | [`Pesm::init_entry`] / [`Pesm::init_entry2`]   |
//! | `cellPesmPrepareRec`         | [`Pesm::prepare_rec`]             |
//! | `cellPesmStartMovieRec` / `EndMovieRec` | [`Pesm::start_movie_rec`] / [`Pesm::end_movie_rec`] |
//! | `cellPesmEncryptSample` / `EncryptSample2` | [`Pesm::encrypt_sample`] / [`Pesm::encrypt_sample2`] |
//! | `cellPesmGetSinf`            | [`Pesm::get_sinf`]                |

use rpcs3_emu_types::CellError;

// =====================================================================
// Error codes (the C++ module has none; we reuse generic codes)
// =====================================================================

pub const CELL_EINVAL: CellError = CellError(0x8001_0002);

// =====================================================================
// Lifecycle state
// =====================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PesmState {
    Uninitialized,
    Initialized,
    DeviceOpen,
    Loaded,
    Prepared,
    Recording,
}

impl Default for PesmState { fn default() -> Self { Self::Uninitialized } }

// =====================================================================
// Manager
// =====================================================================

#[derive(Debug, Clone, Default)]
pub struct Pesm {
    pub state: PesmState,
    pub init_version: u32, // 0 = original Initialize, 1 = Initialize+InitEntry2 path
    pub samples_encrypted: u32,
    pub samples2_encrypted: u32,
    pub sinf_available: bool,
    pub load_tokens: u32, // LoadAsync adds, UnloadAsync subtracts
}

impl Pesm {
    #[must_use]
    pub fn new() -> Self { Self::default() }

    fn require_at_least(&self, min: PesmState) -> Result<(), CellError> {
        let ord = pesm_state_ord(self.state);
        let req = pesm_state_ord(min);
        if ord >= req { Ok(()) } else { Err(CELL_EINVAL) }
    }

    // ---- Init / Finalize --------------------------------------------

    /// Port of `cellPesmInitialize`.
    ///
    /// # Errors
    /// * [`CELL_EINVAL`] if already initialised.
    pub fn initialize(&mut self) -> Result<(), CellError> {
        if self.state != PesmState::Uninitialized {
            return Err(CELL_EINVAL);
        }
        self.state = PesmState::Initialized;
        self.init_version = 0;
        Ok(())
    }

    /// Port of `cellPesmFinalize`.
    pub fn finalize(&mut self) -> Result<(), CellError> {
        if self.state == PesmState::Uninitialized {
            return Err(CELL_EINVAL);
        }
        *self = Self::default();
        Ok(())
    }

    /// Port of `cellPesmFinalize2` — V2 tear-down; behaves identically
    /// to `Finalize` but bumps the version marker (tests can verify
    /// which path ran).
    pub fn finalize2(&mut self) -> Result<(), CellError> {
        self.finalize()?;
        // Leave a sentinel so tests can distinguish.
        self.init_version = u32::MAX;
        Ok(())
    }

    /// Port of `cellPesmInitEntry`.  Registers a "V1" entry — the
    /// paired initializer that PS3 games call after `Initialize`.
    pub fn init_entry(&mut self) -> Result<(), CellError> {
        self.require_at_least(PesmState::Initialized)
    }

    /// Port of `cellPesmInitEntry2`.  Switches the library to the V2
    /// path (observable via `init_version`).
    pub fn init_entry2(&mut self) -> Result<(), CellError> {
        self.require_at_least(PesmState::Initialized)?;
        self.init_version = 1;
        Ok(())
    }

    // ---- Device / Load ---------------------------------------------

    /// Port of `cellPesmOpenDevice` — transitions Initialized → DeviceOpen.
    pub fn open_device(&mut self) -> Result<(), CellError> {
        if self.state != PesmState::Initialized {
            return Err(CELL_EINVAL);
        }
        self.state = PesmState::DeviceOpen;
        Ok(())
    }

    /// Port of `cellPesmCloseDevice` — requires DeviceOpen / Loaded /
    /// Prepared (not Recording).
    pub fn close_device(&mut self) -> Result<(), CellError> {
        match self.state {
            PesmState::DeviceOpen | PesmState::Loaded | PesmState::Prepared => {
                self.state = PesmState::Initialized;
                self.load_tokens = 0;
                self.sinf_available = false;
                Ok(())
            }
            _ => Err(CELL_EINVAL),
        }
    }

    /// Port of `cellPesmLoadAsync`.  Increments `load_tokens`; on the
    /// first call transitions DeviceOpen → Loaded.
    pub fn load_async(&mut self) -> Result<(), CellError> {
        if self.state < PesmState::DeviceOpen {
            return Err(CELL_EINVAL);
        }
        self.load_tokens = self.load_tokens.saturating_add(1);
        if self.state == PesmState::DeviceOpen {
            self.state = PesmState::Loaded;
        }
        self.sinf_available = true;
        Ok(())
    }

    /// Port of `cellPesmUnloadAsync`.  Decrements; at 0 goes back to
    /// DeviceOpen.
    pub fn unload_async(&mut self) -> Result<(), CellError> {
        if self.load_tokens == 0 { return Err(CELL_EINVAL); }
        self.load_tokens -= 1;
        if self.load_tokens == 0 {
            self.state = PesmState::DeviceOpen;
            self.sinf_available = false;
        }
        Ok(())
    }

    // ---- Rec / Encrypt ---------------------------------------------

    /// Port of `cellPesmPrepareRec`.
    pub fn prepare_rec(&mut self) -> Result<(), CellError> {
        if self.state != PesmState::Loaded {
            return Err(CELL_EINVAL);
        }
        self.state = PesmState::Prepared;
        Ok(())
    }

    /// Port of `cellPesmStartMovieRec`.
    pub fn start_movie_rec(&mut self) -> Result<(), CellError> {
        if self.state != PesmState::Prepared {
            return Err(CELL_EINVAL);
        }
        self.state = PesmState::Recording;
        Ok(())
    }

    /// Port of `cellPesmEndMovieRec`.
    pub fn end_movie_rec(&mut self) -> Result<(), CellError> {
        if self.state != PesmState::Recording {
            return Err(CELL_EINVAL);
        }
        self.state = PesmState::Prepared;
        Ok(())
    }

    /// Port of `cellPesmEncryptSample`.  Only valid while Recording.
    pub fn encrypt_sample(&mut self) -> Result<(), CellError> {
        if self.state != PesmState::Recording {
            return Err(CELL_EINVAL);
        }
        self.samples_encrypted = self.samples_encrypted.saturating_add(1);
        Ok(())
    }

    /// Port of `cellPesmEncryptSample2` — V2 variant.
    pub fn encrypt_sample2(&mut self) -> Result<(), CellError> {
        if self.state != PesmState::Recording {
            return Err(CELL_EINVAL);
        }
        self.samples2_encrypted = self.samples2_encrypted.saturating_add(1);
        Ok(())
    }

    /// Port of `cellPesmGetSinf`.  Requires `LoadAsync` to have been
    /// called so the Sinf (SCE Information Block) is available.
    pub fn get_sinf(&self) -> Result<(), CellError> {
        if !self.sinf_available { return Err(CELL_EINVAL); }
        Ok(())
    }
}

fn pesm_state_ord(s: PesmState) -> u8 {
    match s {
        PesmState::Uninitialized => 0,
        PesmState::Initialized   => 1,
        PesmState::DeviceOpen    => 2,
        PesmState::Loaded        => 3,
        PesmState::Prepared      => 4,
        PesmState::Recording     => 5,
    }
}

impl PartialOrd for PesmState {
    fn partial_cmp(&self, other: &Self) -> Option<core::cmp::Ordering> {
        pesm_state_ord(*self).partial_cmp(&pesm_state_ord(*other))
    }
}

impl Ord for PesmState {
    fn cmp(&self, other: &Self) -> core::cmp::Ordering {
        pesm_state_ord(*self).cmp(&pesm_state_ord(*other))
    }
}

// =====================================================================
// Registry
// =====================================================================

pub const REGISTERED_ENTRY_POINTS: &[&str] = &[
    "cellPesmInitialize",
    "cellPesmFinalize",
    "cellPesmLoadAsync",
    "cellPesmOpenDevice",
    "cellPesmEncryptSample",
    "cellPesmUnloadAsync",
    "cellPesmGetSinf",
    "cellPesmStartMovieRec",
    "cellPesmInitEntry",
    "cellPesmEndMovieRec",
    "cellPesmEncryptSample2",
    "cellPesmFinalize2",
    "cellPesmCloseDevice",
    "cellPesmInitEntry2",
    "cellPesmPrepareRec",
];

#[must_use]
pub fn is_registered(name: &str) -> bool {
    REGISTERED_ENTRY_POINTS.contains(&name)
}

// =====================================================================
// Tests
// =====================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // ---- constants ---------------------------------------------------

    #[test]
    fn cell_einval_byte_exact() {
        assert_eq!(CELL_EINVAL.0, 0x8001_0002);
    }

    #[test]
    fn registry_has_15_entries() {
        assert_eq!(REGISTERED_ENTRY_POINTS.len(), 15);
    }

    #[test]
    fn registry_cpp_order() {
        // Order from REG_FUNC block cpp:98-112.
        assert_eq!(REGISTERED_ENTRY_POINTS[0], "cellPesmInitialize");
        assert_eq!(REGISTERED_ENTRY_POINTS[1], "cellPesmFinalize");
        assert_eq!(REGISTERED_ENTRY_POINTS[14], "cellPesmPrepareRec");
    }

    #[test]
    fn registry_rejects_unknown() {
        assert!(!is_registered("cellPesmMissing"));
    }

    // ---- Init / Finalize --------------------------------------------

    #[test]
    fn initialize_transitions_to_initialized() {
        let mut p = Pesm::new();
        p.initialize().unwrap();
        assert_eq!(p.state, PesmState::Initialized);
        assert_eq!(p.init_version, 0);
    }

    #[test]
    fn initialize_twice_is_einval() {
        let mut p = Pesm::new();
        p.initialize().unwrap();
        assert_eq!(p.initialize().unwrap_err(), CELL_EINVAL);
    }

    #[test]
    fn finalize_without_init_is_einval() {
        let mut p = Pesm::new();
        assert_eq!(p.finalize().unwrap_err(), CELL_EINVAL);
    }

    #[test]
    fn finalize_resets_to_uninitialized() {
        let mut p = Pesm::new();
        p.initialize().unwrap();
        p.finalize().unwrap();
        assert_eq!(p.state, PesmState::Uninitialized);
    }

    #[test]
    fn finalize2_leaves_sentinel() {
        let mut p = Pesm::new();
        p.initialize().unwrap();
        p.finalize2().unwrap();
        assert_eq!(p.state, PesmState::Uninitialized);
        assert_eq!(p.init_version, u32::MAX);
    }

    #[test]
    fn init_entry_requires_initialized() {
        let mut p = Pesm::new();
        assert_eq!(p.init_entry().unwrap_err(), CELL_EINVAL);
        p.initialize().unwrap();
        p.init_entry().unwrap();
    }

    #[test]
    fn init_entry2_bumps_version() {
        let mut p = Pesm::new();
        p.initialize().unwrap();
        p.init_entry2().unwrap();
        assert_eq!(p.init_version, 1);
    }

    // ---- Device / Load ---------------------------------------------

    #[test]
    fn open_device_requires_initialized() {
        let mut p = Pesm::new();
        assert_eq!(p.open_device().unwrap_err(), CELL_EINVAL);
    }

    #[test]
    fn open_device_transitions() {
        let mut p = Pesm::new();
        p.initialize().unwrap();
        p.open_device().unwrap();
        assert_eq!(p.state, PesmState::DeviceOpen);
    }

    #[test]
    fn close_device_from_initialized_is_einval() {
        let mut p = Pesm::new();
        p.initialize().unwrap();
        assert_eq!(p.close_device().unwrap_err(), CELL_EINVAL);
    }

    #[test]
    fn close_device_rejects_recording() {
        let mut p = Pesm::new();
        p.initialize().unwrap();
        p.open_device().unwrap();
        p.load_async().unwrap();
        p.prepare_rec().unwrap();
        p.start_movie_rec().unwrap();
        assert_eq!(p.close_device().unwrap_err(), CELL_EINVAL);
    }

    #[test]
    fn close_device_returns_to_initialized() {
        let mut p = Pesm::new();
        p.initialize().unwrap();
        p.open_device().unwrap();
        p.close_device().unwrap();
        assert_eq!(p.state, PesmState::Initialized);
    }

    #[test]
    fn load_async_increments_tokens() {
        let mut p = Pesm::new();
        p.initialize().unwrap();
        p.open_device().unwrap();
        p.load_async().unwrap();
        p.load_async().unwrap();
        p.load_async().unwrap();
        assert_eq!(p.load_tokens, 3);
        assert_eq!(p.state, PesmState::Loaded);
        assert!(p.sinf_available);
    }

    #[test]
    fn load_async_requires_device_open() {
        let mut p = Pesm::new();
        assert_eq!(p.load_async().unwrap_err(), CELL_EINVAL);
    }

    #[test]
    fn unload_async_decrements_and_exits_on_zero() {
        let mut p = Pesm::new();
        p.initialize().unwrap();
        p.open_device().unwrap();
        p.load_async().unwrap();
        p.load_async().unwrap();
        p.unload_async().unwrap();
        assert_eq!(p.load_tokens, 1);
        assert_eq!(p.state, PesmState::Loaded);
        p.unload_async().unwrap();
        assert_eq!(p.load_tokens, 0);
        assert_eq!(p.state, PesmState::DeviceOpen);
        assert!(!p.sinf_available);
    }

    #[test]
    fn unload_async_with_zero_tokens_is_einval() {
        let mut p = Pesm::new();
        assert_eq!(p.unload_async().unwrap_err(), CELL_EINVAL);
    }

    // ---- Rec / Encrypt ---------------------------------------------

    #[test]
    fn prepare_rec_requires_loaded() {
        let mut p = Pesm::new();
        p.initialize().unwrap();
        p.open_device().unwrap();
        assert_eq!(p.prepare_rec().unwrap_err(), CELL_EINVAL);
    }

    #[test]
    fn start_movie_rec_requires_prepared() {
        let mut p = Pesm::new();
        assert_eq!(p.start_movie_rec().unwrap_err(), CELL_EINVAL);
    }

    #[test]
    fn start_end_movie_rec_roundtrip() {
        let mut p = Pesm::new();
        p.initialize().unwrap();
        p.open_device().unwrap();
        p.load_async().unwrap();
        p.prepare_rec().unwrap();
        p.start_movie_rec().unwrap();
        assert_eq!(p.state, PesmState::Recording);
        p.end_movie_rec().unwrap();
        assert_eq!(p.state, PesmState::Prepared);
    }

    #[test]
    fn encrypt_sample_requires_recording() {
        let mut p = Pesm::new();
        assert_eq!(p.encrypt_sample().unwrap_err(), CELL_EINVAL);
    }

    #[test]
    fn encrypt_sample_increments_counter() {
        let mut p = Pesm::new();
        p.initialize().unwrap();
        p.open_device().unwrap();
        p.load_async().unwrap();
        p.prepare_rec().unwrap();
        p.start_movie_rec().unwrap();
        for _ in 0..5 {
            p.encrypt_sample().unwrap();
        }
        assert_eq!(p.samples_encrypted, 5);
        assert_eq!(p.samples2_encrypted, 0);
    }

    #[test]
    fn encrypt_sample2_separate_counter() {
        let mut p = Pesm::new();
        p.initialize().unwrap();
        p.open_device().unwrap();
        p.load_async().unwrap();
        p.prepare_rec().unwrap();
        p.start_movie_rec().unwrap();
        p.encrypt_sample().unwrap();
        p.encrypt_sample2().unwrap();
        p.encrypt_sample2().unwrap();
        assert_eq!(p.samples_encrypted, 1);
        assert_eq!(p.samples2_encrypted, 2);
    }

    // ---- Sinf -------------------------------------------------------

    #[test]
    fn get_sinf_before_load_is_einval() {
        let p = Pesm::new();
        assert_eq!(p.get_sinf().unwrap_err(), CELL_EINVAL);
    }

    #[test]
    fn get_sinf_after_load_ok() {
        let mut p = Pesm::new();
        p.initialize().unwrap();
        p.open_device().unwrap();
        p.load_async().unwrap();
        p.get_sinf().unwrap();
    }

    // ---- PesmState ordering ----------------------------------------

    #[test]
    fn state_ordering() {
        assert!(PesmState::Uninitialized < PesmState::Initialized);
        assert!(PesmState::Initialized < PesmState::DeviceOpen);
        assert!(PesmState::DeviceOpen < PesmState::Loaded);
        assert!(PesmState::Loaded < PesmState::Prepared);
        assert!(PesmState::Prepared < PesmState::Recording);
    }

    // ---- full smoke -------------------------------------------------

    #[test]
    fn full_pesm_lifecycle_smoke() {
        let mut p = Pesm::new();

        // 1. Init + entry2 (V2 path).
        p.initialize().unwrap();
        p.init_entry2().unwrap();
        assert_eq!(p.init_version, 1);

        // 2. Open device + LoadAsync twice (ref counting).
        p.open_device().unwrap();
        p.load_async().unwrap();
        p.load_async().unwrap();
        assert_eq!(p.load_tokens, 2);
        p.get_sinf().unwrap();

        // 3. Record 3 samples + 2 samples2.
        p.prepare_rec().unwrap();
        p.start_movie_rec().unwrap();
        for _ in 0..3 { p.encrypt_sample().unwrap(); }
        p.encrypt_sample2().unwrap();
        p.encrypt_sample2().unwrap();
        p.end_movie_rec().unwrap();
        assert_eq!(p.samples_encrypted, 3);
        assert_eq!(p.samples2_encrypted, 2);

        // 4. Unload twice + close device + finalize.
        p.unload_async().unwrap();
        p.unload_async().unwrap();
        assert_eq!(p.state, PesmState::DeviceOpen);
        p.close_device().unwrap();
        p.finalize().unwrap();
        assert_eq!(p.state, PesmState::Uninitialized);
    }
}
