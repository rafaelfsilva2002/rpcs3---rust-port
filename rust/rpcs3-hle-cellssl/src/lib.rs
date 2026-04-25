//! `rpcs3-hle-cellssl` — TLS/SSL helpers HLE layer.
//!
//! Ports `rpcs3/Emu/Cell/Modules/cellSsl.cpp`. cellSsl is a thin
//! wrapper over root-CA certificate management. Games call it before
//! HTTPS to select which CA bundle to trust. This port tracks the
//! init state, selected certs bitmask, and loaded cert slots.
//!
//! ## Entry points covered
//!
//! | HLE function                  | Rust wrapper                    |
//! |-------------------------------|---------------------------------|
//! | `cellSslInit`                 | [`cell_ssl_init`]               |
//! | `cellSslEnd`                  | [`cell_ssl_end`]                |
//! | `cellSslCertGetSerialNumber`  | [`cell_ssl_cert_get_serial_number`] |
//! | `cellSslCertGetPublicKey`     | [`cell_ssl_cert_get_public_key`] |
//! | `cellSslCertGetIssuerName`    | [`cell_ssl_cert_get_issuer_name`] |
//! | `cellSslCertGetSubjectName`   | [`cell_ssl_cert_get_subject_name`] |
//! | `cellSslCertGetNotBefore`     | [`cell_ssl_cert_get_not_before`] |
//! | `cellSslCertGetNotAfter`      | [`cell_ssl_cert_get_not_after`] |
//! | `cellSslCertGetMd5Fingerprint`| [`cell_ssl_cert_get_md5_fingerprint`] |

use rpcs3_emu_types::CellError;

// =====================================================================
// Error codes — byte-exact with cellSsl.h:5-19
// =====================================================================

pub mod errors {
    use rpcs3_emu_types::CellError;

    pub const NOT_INITIALIZED: CellError = CellError(0x8074_0001);
    pub const ALREADY_INITIALIZED: CellError = CellError(0x8074_0002);
    pub const INITIALIZATION_FAILED: CellError = CellError(0x8074_0003);
    pub const NO_BUFFER: CellError = CellError(0x8074_0004);
    pub const INVALID_CERTIFICATE: CellError = CellError(0x8074_0005);
    pub const UNRETRIEVABLE: CellError = CellError(0x8074_0006);
    pub const INVALID_FORMAT: CellError = CellError(0x8074_0007);
    pub const NOT_FOUND: CellError = CellError(0x8074_0008);
    pub const INVALID_TIME: CellError = CellError(0x8074_0031);
    pub const INVALID_NEGATIVE_TIME: CellError = CellError(0x8074_0032);
    pub const INCORRECT_TIME: CellError = CellError(0x8074_0033);
    pub const UNDEFINED_TIME_TYPE: CellError = CellError(0x8074_0034);
    pub const NO_MEMORY: CellError = CellError(0x8074_0035);
    pub const NO_STRING: CellError = CellError(0x8074_0036);
    pub const UNKNOWN_LOAD_CERT: CellError = CellError(0x8074_0037);
}

// =====================================================================
// Load-cert bitmask constants (subset)
// =====================================================================

pub const CERT_BALTIMORE_CT: u64 = 0x0000_0000_0000_0020;
pub const CERT_ENTRUST_NET_SS_CA: u64 = 0x0000_0000_0002_0000;
pub const CERT_EQUIFAX_SEC_CA: u64 = 0x0000_0000_0004_0000;
pub const CERT_GEOTRUST_GCA: u64 = 0x0000_0000_0010_0000;
pub const CERT_GLOBALSIGN_RCA: u64 = 0x0000_0000_0020_0000;
pub const CERT_RSA_SECURE_SERVER: u64 = 0x0000_0000_0400_0000;
pub const CERT_THAWTE_PREM_SCA: u64 = 0x0000_0000_0800_0000;
pub const CERT_THAWTE_SCA: u64 = 0x0000_0000_1000_0000;
pub const CERT_VERISIGN_TSA_CA: u64 = 0x0000_0000_4000_0000;
pub const CERT_AAA_CERT_SERVICES: u64 = 0x0000_0000_8000_0000;
pub const CERT_ADDTRUST_EXT_CA: u64 = 0x0000_0001_0000_0000;
pub const CERT_DIGICERT_HA_EV_RCA: u64 = 0x0000_0010_0000_0000;
pub const CERT_DIGICERT_A_ID_RCA: u64 = 0x0000_0020_0000_0000;
pub const CERT_DIGICERT_GLOBAL_RCA: u64 = 0x0000_0040_0000_0000;
pub const CERT_STARFIELD_S_RC: u64 = 0x0000_2000_0000_0000;
pub const CERT_STARTCOM_CA: u64 = 0x0080_0000_0000_0000;
pub const CERT_VERISIGN_U_RCA: u64 = 0x0008_0000_0000_0000;

/// Mask of every certificate bit the runtime knows about. Unknown
/// bits in a load mask would trigger `UNKNOWN_LOAD_CERT`.
pub const CERT_KNOWN_MASK: u64 = CERT_BALTIMORE_CT
    | CERT_ENTRUST_NET_SS_CA
    | CERT_EQUIFAX_SEC_CA
    | CERT_GEOTRUST_GCA
    | CERT_GLOBALSIGN_RCA
    | CERT_RSA_SECURE_SERVER
    | CERT_THAWTE_PREM_SCA
    | CERT_THAWTE_SCA
    | CERT_VERISIGN_TSA_CA
    | CERT_AAA_CERT_SERVICES
    | CERT_ADDTRUST_EXT_CA
    | CERT_DIGICERT_HA_EV_RCA
    | CERT_DIGICERT_A_ID_RCA
    | CERT_DIGICERT_GLOBAL_RCA
    | CERT_STARFIELD_S_RC
    | CERT_STARTCOM_CA
    | CERT_VERISIGN_U_RCA;

// =====================================================================
// Data model
// =====================================================================

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Certificate {
    pub serial_number: Vec<u8>,
    pub issuer_name: String,
    pub subject_name: String,
    pub public_key: Vec<u8>,
    pub not_before_unix: i64,
    pub not_after_unix: i64,
    pub md5_fingerprint: [u8; 16],
}

impl Certificate {
    /// Stub cert useful for tests.
    pub fn dummy(name: &str) -> Self {
        Self {
            serial_number: vec![0x01, 0x02, 0x03, 0x04],
            issuer_name: name.to_owned(),
            subject_name: name.to_owned(),
            public_key: vec![0xAB; 32],
            not_before_unix: 0,
            not_after_unix: i64::MAX,
            md5_fingerprint: [0x99; 16],
        }
    }
}

#[derive(Debug, Default)]
pub struct SslManager {
    initialized: bool,
    loaded_cert_mask: u64,
    /// Slot handle → parsed certificate.
    certs: std::collections::BTreeMap<u32, Certificate>,
    next_cert_handle: u32,
}

// =====================================================================
// Time type constants
// =====================================================================

pub const TIME_TYPE_NOT_BEFORE: u32 = 0;
pub const TIME_TYPE_NOT_AFTER: u32 = 1;

// =====================================================================
// Syscalls
// =====================================================================

fn ensure_init(m: &SslManager) -> Result<(), CellError> {
    if m.initialized { Ok(()) } else { Err(errors::NOT_INITIALIZED) }
}

/// `cellSslInit(pool, pool_size)`.
#[must_use]
pub fn cell_ssl_init(m: &mut SslManager, pool_size: u32) -> Result<(), CellError> {
    if m.initialized {
        return Err(errors::ALREADY_INITIALIZED);
    }
    if pool_size == 0 {
        return Err(errors::NO_BUFFER);
    }
    m.initialized = true;
    Ok(())
}

/// `cellSslEnd()`.
#[must_use]
pub fn cell_ssl_end(m: &mut SslManager) -> Result<(), CellError> {
    ensure_init(m)?;
    m.initialized = false;
    m.loaded_cert_mask = 0;
    m.certs.clear();
    m.next_cert_handle = 0;
    Ok(())
}

/// `cellSslCertLoadFromBitmask(mask)` — flip bits in the loaded-cert
/// bitmap. Unknown bits trigger `UNKNOWN_LOAD_CERT`.
#[must_use]
pub fn cell_ssl_cert_load_from_bitmask(
    m: &mut SslManager,
    mask: u64,
) -> Result<(), CellError> {
    ensure_init(m)?;
    if mask & !CERT_KNOWN_MASK != 0 {
        return Err(errors::UNKNOWN_LOAD_CERT);
    }
    m.loaded_cert_mask |= mask;
    Ok(())
}

/// Test helper: register a parsed certificate. Real impl loads from
/// filesystem; the emu core plugs this in.
pub fn install_certificate(m: &mut SslManager, cert: Certificate) -> Result<u32, CellError> {
    ensure_init(m)?;
    m.next_cert_handle += 1;
    let h = m.next_cert_handle;
    m.certs.insert(h, cert);
    Ok(h)
}

fn lookup<'a>(m: &'a SslManager, handle: u32) -> Result<&'a Certificate, CellError> {
    m.certs.get(&handle).ok_or(errors::NOT_FOUND)
}

#[must_use]
pub fn cell_ssl_cert_get_serial_number(
    m: &SslManager,
    handle: u32,
) -> Result<Vec<u8>, CellError> {
    ensure_init(m)?;
    Ok(lookup(m, handle)?.serial_number.clone())
}

#[must_use]
pub fn cell_ssl_cert_get_issuer_name(
    m: &SslManager,
    handle: u32,
) -> Result<String, CellError> {
    ensure_init(m)?;
    Ok(lookup(m, handle)?.issuer_name.clone())
}

#[must_use]
pub fn cell_ssl_cert_get_subject_name(
    m: &SslManager,
    handle: u32,
) -> Result<String, CellError> {
    ensure_init(m)?;
    Ok(lookup(m, handle)?.subject_name.clone())
}

#[must_use]
pub fn cell_ssl_cert_get_public_key(
    m: &SslManager,
    handle: u32,
) -> Result<Vec<u8>, CellError> {
    ensure_init(m)?;
    Ok(lookup(m, handle)?.public_key.clone())
}

#[must_use]
pub fn cell_ssl_cert_get_not_before(
    m: &SslManager,
    handle: u32,
) -> Result<i64, CellError> {
    ensure_init(m)?;
    Ok(lookup(m, handle)?.not_before_unix)
}

#[must_use]
pub fn cell_ssl_cert_get_not_after(
    m: &SslManager,
    handle: u32,
) -> Result<i64, CellError> {
    ensure_init(m)?;
    Ok(lookup(m, handle)?.not_after_unix)
}

#[must_use]
pub fn cell_ssl_cert_get_md5_fingerprint(
    m: &SslManager,
    handle: u32,
) -> Result<[u8; 16], CellError> {
    ensure_init(m)?;
    Ok(lookup(m, handle)?.md5_fingerprint)
}

/// `cellSslCertGetNameEntryCount/GetNameEntryInfo` — helper that picks
/// NOT_BEFORE or NOT_AFTER based on the time_type argument.
#[must_use]
pub fn cell_ssl_cert_get_time(
    m: &SslManager,
    handle: u32,
    time_type: u32,
) -> Result<i64, CellError> {
    ensure_init(m)?;
    match time_type {
        TIME_TYPE_NOT_BEFORE => Ok(lookup(m, handle)?.not_before_unix),
        TIME_TYPE_NOT_AFTER => Ok(lookup(m, handle)?.not_after_unix),
        _ => Err(errors::UNDEFINED_TIME_TYPE),
    }
}

/// Return the currently-loaded cert bitmask (test helper).
#[must_use]
pub fn loaded_cert_mask(m: &SslManager) -> u64 {
    m.loaded_cert_mask
}

// =====================================================================
// Tests
// =====================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn init_mgr() -> SslManager {
        let mut m = SslManager::default();
        cell_ssl_init(&mut m, 0x1_0000).unwrap();
        m
    }

    // --- constants ------------------------------------------------

    #[test]
    fn error_codes_byte_exact_vs_cpp() {
        assert_eq!(errors::NOT_INITIALIZED.0, 0x8074_0001);
        assert_eq!(errors::INVALID_CERTIFICATE.0, 0x8074_0005);
        assert_eq!(errors::INVALID_TIME.0, 0x8074_0031);
        assert_eq!(errors::UNKNOWN_LOAD_CERT.0, 0x8074_0037);
    }

    #[test]
    fn cert_bitmask_constants_match_cellSsl_h() {
        assert_eq!(CERT_BALTIMORE_CT, 0x0000_0000_0000_0020);
        assert_eq!(CERT_RSA_SECURE_SERVER, 0x0000_0000_0400_0000);
        assert_eq!(CERT_AAA_CERT_SERVICES, 0x0000_0000_8000_0000);
        assert_eq!(CERT_DIGICERT_GLOBAL_RCA, 0x0000_0040_0000_0000);
        assert_eq!(CERT_STARTCOM_CA, 0x0080_0000_0000_0000);
    }

    // --- init / end ----------------------------------------------

    #[test]
    fn init_twice_is_already_initialized() {
        let mut m = SslManager::default();
        cell_ssl_init(&mut m, 0x1_0000).unwrap();
        assert_eq!(
            cell_ssl_init(&mut m, 0x1_0000).unwrap_err(),
            errors::ALREADY_INITIALIZED,
        );
    }

    #[test]
    fn init_zero_pool_is_no_buffer() {
        let mut m = SslManager::default();
        assert_eq!(cell_ssl_init(&mut m, 0).unwrap_err(), errors::NO_BUFFER);
    }

    #[test]
    fn end_without_init_is_not_initialized() {
        let mut m = SslManager::default();
        assert_eq!(cell_ssl_end(&mut m).unwrap_err(), errors::NOT_INITIALIZED);
    }

    #[test]
    fn end_clears_loaded_cert_mask() {
        let mut m = init_mgr();
        cell_ssl_cert_load_from_bitmask(&mut m, CERT_BALTIMORE_CT).unwrap();
        cell_ssl_end(&mut m).unwrap();
        cell_ssl_init(&mut m, 0x1_0000).unwrap();
        assert_eq!(loaded_cert_mask(&m), 0);
    }

    // --- cert load bitmask ----------------------------------------

    #[test]
    fn load_from_bitmask_accumulates() {
        let mut m = init_mgr();
        cell_ssl_cert_load_from_bitmask(&mut m, CERT_BALTIMORE_CT).unwrap();
        cell_ssl_cert_load_from_bitmask(&mut m, CERT_VERISIGN_TSA_CA).unwrap();
        assert_eq!(
            loaded_cert_mask(&m),
            CERT_BALTIMORE_CT | CERT_VERISIGN_TSA_CA,
        );
    }

    #[test]
    fn load_from_unknown_bit_is_unknown_load_cert() {
        let mut m = init_mgr();
        // Some arbitrary bit not in CERT_KNOWN_MASK.
        let bogus = 0x1u64 << 1;
        assert_eq!(
            cell_ssl_cert_load_from_bitmask(&mut m, bogus).unwrap_err(),
            errors::UNKNOWN_LOAD_CERT,
        );
    }

    #[test]
    fn load_without_init_is_not_initialized() {
        let mut m = SslManager::default();
        assert_eq!(
            cell_ssl_cert_load_from_bitmask(&mut m, CERT_BALTIMORE_CT).unwrap_err(),
            errors::NOT_INITIALIZED,
        );
    }

    // --- cert lookup ops ------------------------------------------

    #[test]
    fn install_cert_round_trips_all_fields() {
        let mut m = init_mgr();
        let cert = Certificate {
            serial_number: vec![0xAA, 0xBB, 0xCC, 0xDD],
            issuer_name: "Example CA".into(),
            subject_name: "example.com".into(),
            public_key: vec![1, 2, 3, 4, 5],
            not_before_unix: 1000,
            not_after_unix: 2000,
            md5_fingerprint: [0x55; 16],
        };
        let h = install_certificate(&mut m, cert.clone()).unwrap();

        assert_eq!(cell_ssl_cert_get_serial_number(&m, h).unwrap(), cert.serial_number);
        assert_eq!(cell_ssl_cert_get_issuer_name(&m, h).unwrap(), "Example CA");
        assert_eq!(cell_ssl_cert_get_subject_name(&m, h).unwrap(), "example.com");
        assert_eq!(cell_ssl_cert_get_public_key(&m, h).unwrap(), cert.public_key);
        assert_eq!(cell_ssl_cert_get_not_before(&m, h).unwrap(), 1000);
        assert_eq!(cell_ssl_cert_get_not_after(&m, h).unwrap(), 2000);
        assert_eq!(cell_ssl_cert_get_md5_fingerprint(&m, h).unwrap(), [0x55; 16]);
    }

    #[test]
    fn lookup_unknown_handle_is_not_found() {
        let m = init_mgr();
        assert_eq!(
            cell_ssl_cert_get_serial_number(&m, 999).unwrap_err(),
            errors::NOT_FOUND,
        );
    }

    #[test]
    fn lookup_without_init_is_not_initialized() {
        let m = SslManager::default();
        assert_eq!(
            cell_ssl_cert_get_serial_number(&m, 1).unwrap_err(),
            errors::NOT_INITIALIZED,
        );
    }

    // --- time helper ----------------------------------------------

    #[test]
    fn get_time_not_before_returns_cert_field() {
        let mut m = init_mgr();
        let mut cert = Certificate::dummy("CA");
        cert.not_before_unix = 12345;
        cert.not_after_unix = 67890;
        let h = install_certificate(&mut m, cert).unwrap();

        assert_eq!(cell_ssl_cert_get_time(&m, h, TIME_TYPE_NOT_BEFORE).unwrap(), 12345);
        assert_eq!(cell_ssl_cert_get_time(&m, h, TIME_TYPE_NOT_AFTER).unwrap(), 67890);
    }

    #[test]
    fn get_time_bad_type_is_undefined_time_type() {
        let mut m = init_mgr();
        let h = install_certificate(&mut m, Certificate::dummy("x")).unwrap();
        assert_eq!(
            cell_ssl_cert_get_time(&m, h, 99).unwrap_err(),
            errors::UNDEFINED_TIME_TYPE,
        );
    }

    // --- known mask smoke test ------------------------------------

    #[test]
    fn full_known_mask_loads_cleanly() {
        let mut m = init_mgr();
        cell_ssl_cert_load_from_bitmask(&mut m, CERT_KNOWN_MASK).unwrap();
        assert_eq!(loaded_cert_mask(&m), CERT_KNOWN_MASK);
    }

    #[test]
    fn dummy_cert_round_trips_via_install() {
        let mut m = init_mgr();
        let h = install_certificate(&mut m, Certificate::dummy("Test CA")).unwrap();
        assert_eq!(cell_ssl_cert_get_issuer_name(&m, h).unwrap(), "Test CA");
    }

    #[test]
    fn install_cert_without_init_is_not_initialized() {
        let mut m = SslManager::default();
        assert_eq!(
            install_certificate(&mut m, Certificate::dummy("x")).unwrap_err(),
            errors::NOT_INITIALIZED,
        );
    }
}
