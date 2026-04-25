//! `rpcs3-hle-cellhttps` — HTTPS / TLS extension HLE.
//!
//! Ports the HTTPS subset of `rpcs3/Emu/Cell/Modules/cellHttp.h:121-139`
//! plus `cellHttpsInit` and the cert-mgmt callbacks that games use to
//! bind a PS3 root-CA bundle to HTTPS sessions. Rides on top of
//! `rpcs3-hle-cellhttp` via its `Uri` builder.
//!
//! ## Entry points covered
//!
//! | HLE function                               | Rust wrapper                          |
//! |--------------------------------------------|---------------------------------------|
//! | `cellHttpsInit`                            | [`HttpsManager::init`]                |
//! | `cellHttpsEnd`                             | [`HttpsManager::end`]                 |
//! | `cellHttpsSetCaList`                       | [`HttpsManager::set_ca_list`]         |
//! | `cellHttpsCreateContext`                   | [`HttpsManager::create_context`]      |
//! | `cellHttpsDestroyContext`                  | [`HttpsManager::destroy_context`]     |
//! | `cellHttpsConnectionCreate` / `Destroy`    | [`HttpsManager::create_connection`]   |

use rpcs3_emu_types::CellError;

// =====================================================================
// Error codes — byte-exact with cellHttp.h:121-139
// =====================================================================

pub mod errors {
    use rpcs3_emu_types::CellError;

    pub const CERTIFICATE_LOAD: CellError = CellError(0x8071_0a01);
    pub const BAD_MEMORY: CellError = CellError(0x8071_0a02);
    pub const CONTEXT_CREATION: CellError = CellError(0x8071_0a03);
    pub const CONNECTION_CREATION: CellError = CellError(0x8071_0a04);
    pub const SOCKET_ASSOCIATION: CellError = CellError(0x8071_0a05);
    pub const HANDSHAKE: CellError = CellError(0x8071_0a06);
    pub const LOOKUP_CERTIFICATE: CellError = CellError(0x8071_0a07);
    pub const NO_SSL: CellError = CellError(0x8071_0a08);
    pub const KEY_LOAD: CellError = CellError(0x8071_0a09);
    pub const CERT_KEY_MISMATCH: CellError = CellError(0x8071_0a0a);
    pub const KEY_NEEDS_CERT: CellError = CellError(0x8071_0a0b);
    pub const CERT_NEEDS_KEY: CellError = CellError(0x8071_0a0c);
    pub const RETRY_CONNECTION: CellError = CellError(0x8071_0a0d);
    /// High byte 0x0b = SSL connect subcategory.
    pub const NET_SSL_CONNECT: CellError = CellError(0x8071_0b00);
    /// High byte 0x0c = SSL send subcategory.
    pub const NET_SSL_SEND: CellError = CellError(0x8071_0c00);
    /// High byte 0x0d = SSL recv subcategory.
    pub const NET_SSL_RECV: CellError = CellError(0x8071_0d00);
}

// =====================================================================
// Protocol versions / cipher suites (common PS3 HTTPS surface)
// =====================================================================

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TlsVersion {
    Sslv3,
    Tls10,
    Tls11,
    Tls12,
}

impl TlsVersion {
    #[must_use]
    pub const fn as_u16(self) -> u16 {
        match self {
            Self::Sslv3 => 0x0300,
            Self::Tls10 => 0x0301,
            Self::Tls11 => 0x0302,
            Self::Tls12 => 0x0303,
        }
    }

    #[must_use]
    pub const fn from_u16(v: u16) -> Option<Self> {
        Some(match v {
            0x0300 => Self::Sslv3,
            0x0301 => Self::Tls10,
            0x0302 => Self::Tls11,
            0x0303 => Self::Tls12,
            _ => return None,
        })
    }
}

// =====================================================================
// Domain types
// =====================================================================

/// Certificate slot — a DER-encoded X.509 blob + metadata the PS3 shell
/// exposes to games via `cellSsl` / `cellHttps`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Certificate {
    pub der: Vec<u8>,
    pub subject: String,
    pub issuer: String,
    pub not_before: i64, // Unix time
    pub not_after: i64,
}

impl Certificate {
    fn validate(&self) -> Result<(), CellError> {
        if self.der.is_empty() {
            return Err(errors::CERTIFICATE_LOAD);
        }
        if self.der.len() > 16 * 1024 {
            return Err(errors::BAD_MEMORY);
        }
        if self.subject.is_empty() || self.issuer.is_empty() {
            return Err(errors::CERTIFICATE_LOAD);
        }
        if self.not_after <= self.not_before {
            return Err(errors::CERTIFICATE_LOAD);
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PrivateKey {
    pub der: Vec<u8>,
}

impl PrivateKey {
    fn validate(&self) -> Result<(), CellError> {
        if self.der.is_empty() {
            return Err(errors::KEY_LOAD);
        }
        if self.der.len() > 8 * 1024 {
            return Err(errors::BAD_MEMORY);
        }
        Ok(())
    }
}

#[derive(Clone, Debug)]
pub struct Context {
    pub id: u32,
    pub min_version: TlsVersion,
    pub ca_list_count: u32,
    pub client_cert: Option<Certificate>,
    pub client_key: Option<PrivateKey>,
}

#[derive(Clone, Debug)]
pub struct Connection {
    pub id: u32,
    pub context_id: u32,
    pub socket: i32,
    pub handshake_complete: bool,
    pub peer_cert: Option<Certificate>,
}

// =====================================================================
// HttpsManager
// =====================================================================

pub const MAX_CA_LIST: usize = 64;
pub const MAX_CONTEXTS: usize = 32;
pub const MAX_CONNECTIONS: usize = 64;

#[derive(Clone, Debug)]
pub struct HttpsManager {
    initialized: bool,
    ca_list: Vec<Certificate>,
    contexts: Vec<Context>,
    connections: Vec<Connection>,
    next_ctx_id: u32,
    next_conn_id: u32,
}

impl HttpsManager {
    #[must_use]
    pub fn new() -> Self {
        Self {
            initialized: false,
            ca_list: Vec::new(),
            contexts: Vec::new(),
            connections: Vec::new(),
            next_ctx_id: 1,
            next_conn_id: 1,
        }
    }

    #[must_use]
    pub fn is_initialized(&self) -> bool {
        self.initialized
    }

    #[must_use]
    pub fn ca_list_len(&self) -> usize {
        self.ca_list.len()
    }

    #[must_use]
    pub fn context_count(&self) -> usize {
        self.contexts.len()
    }

    #[must_use]
    pub fn connection_count(&self) -> usize {
        self.connections.len()
    }

    // ----------------- Lifecycle -----------------

    pub fn init(&mut self) -> Result<(), CellError> {
        if self.initialized {
            return Err(errors::CONTEXT_CREATION);
        }
        self.initialized = true;
        Ok(())
    }

    pub fn end(&mut self) -> Result<(), CellError> {
        if !self.initialized {
            return Err(errors::NO_SSL);
        }
        self.initialized = false;
        self.ca_list.clear();
        self.contexts.clear();
        self.connections.clear();
        Ok(())
    }

    // ----------------- CA trust list -----------------

    /// `cellHttpsSetCaList(certList, count)`. Replaces the trusted-CA
    /// bundle used for future contexts.
    pub fn set_ca_list(&mut self, certs: Vec<Certificate>) -> Result<(), CellError> {
        self.require_initialized()?;
        if certs.len() > MAX_CA_LIST {
            return Err(errors::BAD_MEMORY);
        }
        for c in &certs {
            c.validate()?;
        }
        self.ca_list = certs;
        Ok(())
    }

    // ----------------- Contexts -----------------

    /// `cellHttpsCreateContext(minVersion)`. Uses the current CA list
    /// as the trust root.
    pub fn create_context(&mut self, min_version: TlsVersion) -> Result<u32, CellError> {
        self.require_initialized()?;
        if self.contexts.len() >= MAX_CONTEXTS {
            return Err(errors::CONTEXT_CREATION);
        }
        let id = self.next_ctx_id;
        self.next_ctx_id = self.next_ctx_id.checked_add(1).ok_or(errors::CONTEXT_CREATION)?;
        self.contexts.push(Context {
            id,
            min_version,
            ca_list_count: u32::try_from(self.ca_list.len()).unwrap_or(u32::MAX),
            client_cert: None,
            client_key: None,
        });
        Ok(id)
    }

    pub fn destroy_context(&mut self, context_id: u32) -> Result<(), CellError> {
        self.require_initialized()?;
        // Any connection still referencing the context blocks destroy.
        if self.connections.iter().any(|c| c.context_id == context_id) {
            return Err(errors::CONTEXT_CREATION);
        }
        let idx = self.ctx_idx(context_id)?;
        self.contexts.remove(idx);
        Ok(())
    }

    /// Client-cert mutual TLS. Must install cert + key together; either
    /// alone is a CERT_NEEDS_KEY / KEY_NEEDS_CERT error.
    pub fn set_client_identity(
        &mut self,
        context_id: u32,
        cert: Certificate,
        key: PrivateKey,
    ) -> Result<(), CellError> {
        self.require_initialized()?;
        cert.validate()?;
        key.validate()?;
        // Simple mismatch heuristic: cert DER must share a 4-byte
        // public-key prefix with the key. Real lib matches RSA moduli.
        if cert.der.len() >= 4 && key.der.len() >= 4 && cert.der[..4] != key.der[..4] {
            return Err(errors::CERT_KEY_MISMATCH);
        }
        let idx = self.ctx_idx(context_id)?;
        self.contexts[idx].client_cert = Some(cert);
        self.contexts[idx].client_key = Some(key);
        Ok(())
    }

    pub fn set_client_cert_only(&mut self, _context_id: u32, _cert: Certificate) -> Result<(), CellError> {
        Err(errors::CERT_NEEDS_KEY)
    }

    pub fn set_client_key_only(&mut self, _context_id: u32, _key: PrivateKey) -> Result<(), CellError> {
        Err(errors::KEY_NEEDS_CERT)
    }

    // ----------------- Connections -----------------

    /// `cellHttpsConnectionCreate(ctx, socket)`. Binds a TCP socket to
    /// a freshly-minted TLS connection (pre-handshake).
    pub fn create_connection(&mut self, context_id: u32, socket: i32) -> Result<u32, CellError> {
        self.require_initialized()?;
        if socket < 0 {
            return Err(errors::SOCKET_ASSOCIATION);
        }
        let _ = self.ctx_idx(context_id)?;
        if self.connections.len() >= MAX_CONNECTIONS {
            return Err(errors::CONNECTION_CREATION);
        }
        let id = self.next_conn_id;
        self.next_conn_id = self.next_conn_id.checked_add(1).ok_or(errors::CONNECTION_CREATION)?;
        self.connections.push(Connection {
            id,
            context_id,
            socket,
            handshake_complete: false,
            peer_cert: None,
        });
        Ok(id)
    }

    pub fn destroy_connection(&mut self, connection_id: u32) -> Result<(), CellError> {
        self.require_initialized()?;
        let idx = self.conn_idx(connection_id)?;
        self.connections.remove(idx);
        Ok(())
    }

    /// Test hook: mark the TLS handshake as complete and record the
    /// peer certificate. Real lib runs the full TLS state machine.
    pub fn complete_handshake(
        &mut self,
        connection_id: u32,
        peer_cert: Certificate,
    ) -> Result<(), CellError> {
        self.require_initialized()?;
        peer_cert.validate()?;
        let idx = self.conn_idx(connection_id)?;
        if self.connections[idx].handshake_complete {
            return Err(errors::HANDSHAKE);
        }
        // Peer cert must be signed by something in the CA list — simple
        // match: issuer string appears as a subject in the CA list.
        let issuer = peer_cert.issuer.clone();
        let trusted = self.ca_list.iter().any(|ca| ca.subject == issuer);
        if !trusted {
            return Err(errors::LOOKUP_CERTIFICATE);
        }
        self.connections[idx].handshake_complete = true;
        self.connections[idx].peer_cert = Some(peer_cert);
        Ok(())
    }

    pub fn require_handshake(&self, connection_id: u32) -> Result<(), CellError> {
        self.require_initialized()?;
        let idx = self.conn_idx(connection_id)?;
        if !self.connections[idx].handshake_complete {
            return Err(errors::HANDSHAKE);
        }
        Ok(())
    }

    pub fn peer_cert(&self, connection_id: u32) -> Result<&Certificate, CellError> {
        self.require_initialized()?;
        let idx = self.conn_idx(connection_id)?;
        self.connections[idx].peer_cert.as_ref().ok_or(errors::HANDSHAKE)
    }

    // ----------------- Helpers -----------------

    fn require_initialized(&self) -> Result<(), CellError> {
        if self.initialized { Ok(()) } else { Err(errors::NO_SSL) }
    }

    fn ctx_idx(&self, id: u32) -> Result<usize, CellError> {
        self.contexts.iter().position(|c| c.id == id).ok_or(errors::CONTEXT_CREATION)
    }

    fn conn_idx(&self, id: u32) -> Result<usize, CellError> {
        self.connections.iter().position(|c| c.id == id).ok_or(errors::CONNECTION_CREATION)
    }
}

impl Default for HttpsManager {
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

    fn ok_cert(subject: &str, issuer: &str) -> Certificate {
        Certificate {
            der: vec![0xDE, 0xAD, 0xBE, 0xEF, 0x00, 0x01, 0x02, 0x03],
            subject: subject.into(),
            issuer: issuer.into(),
            not_before: 1_000,
            not_after: 2_000,
        }
    }

    fn ok_key() -> PrivateKey {
        PrivateKey { der: vec![0xDE, 0xAD, 0xBE, 0xEF, 0x10, 0x11, 0x12, 0x13] }
    }

    fn initialized_with_ca() -> HttpsManager {
        let mut m = HttpsManager::new();
        m.init().unwrap();
        m.set_ca_list(vec![ok_cert("Root CA", "Root CA")]).unwrap();
        m
    }

    #[test]
    fn error_codes_byte_exact() {
        assert_eq!(errors::CERTIFICATE_LOAD.0, 0x8071_0a01);
        assert_eq!(errors::BAD_MEMORY.0, 0x8071_0a02);
        assert_eq!(errors::CONTEXT_CREATION.0, 0x8071_0a03);
        assert_eq!(errors::CONNECTION_CREATION.0, 0x8071_0a04);
        assert_eq!(errors::SOCKET_ASSOCIATION.0, 0x8071_0a05);
        assert_eq!(errors::HANDSHAKE.0, 0x8071_0a06);
        assert_eq!(errors::LOOKUP_CERTIFICATE.0, 0x8071_0a07);
        assert_eq!(errors::NO_SSL.0, 0x8071_0a08);
        assert_eq!(errors::KEY_LOAD.0, 0x8071_0a09);
        assert_eq!(errors::CERT_KEY_MISMATCH.0, 0x8071_0a0a);
        assert_eq!(errors::KEY_NEEDS_CERT.0, 0x8071_0a0b);
        assert_eq!(errors::CERT_NEEDS_KEY.0, 0x8071_0a0c);
        assert_eq!(errors::RETRY_CONNECTION.0, 0x8071_0a0d);
        assert_eq!(errors::NET_SSL_CONNECT.0, 0x8071_0b00);
        assert_eq!(errors::NET_SSL_SEND.0, 0x8071_0c00);
        assert_eq!(errors::NET_SSL_RECV.0, 0x8071_0d00);
    }

    #[test]
    fn tls_version_u16_stable() {
        assert_eq!(TlsVersion::Sslv3.as_u16(), 0x0300);
        assert_eq!(TlsVersion::Tls10.as_u16(), 0x0301);
        assert_eq!(TlsVersion::Tls11.as_u16(), 0x0302);
        assert_eq!(TlsVersion::Tls12.as_u16(), 0x0303);
    }

    #[test]
    fn tls_version_round_trip() {
        for v in [TlsVersion::Sslv3, TlsVersion::Tls10, TlsVersion::Tls11, TlsVersion::Tls12] {
            assert_eq!(TlsVersion::from_u16(v.as_u16()), Some(v));
        }
        assert_eq!(TlsVersion::from_u16(0xFFFF), None);
    }

    #[test]
    fn constants_stable() {
        assert_eq!(MAX_CA_LIST, 64);
        assert_eq!(MAX_CONTEXTS, 32);
        assert_eq!(MAX_CONNECTIONS, 64);
    }

    #[test]
    fn cert_validate_empty_der_rejected() {
        let mut c = ok_cert("s", "i");
        c.der.clear();
        assert_eq!(c.validate(), Err(errors::CERTIFICATE_LOAD));
    }

    #[test]
    fn cert_validate_huge_der_is_bad_memory() {
        let mut c = ok_cert("s", "i");
        c.der = vec![0u8; 16 * 1024 + 1];
        assert_eq!(c.validate(), Err(errors::BAD_MEMORY));
    }

    #[test]
    fn cert_validate_empty_subject_rejected() {
        let mut c = ok_cert("", "i");
        assert_eq!(c.validate(), Err(errors::CERTIFICATE_LOAD));
        c = ok_cert("s", "");
        assert_eq!(c.validate(), Err(errors::CERTIFICATE_LOAD));
    }

    #[test]
    fn cert_validate_inverted_dates_rejected() {
        let mut c = ok_cert("s", "i");
        c.not_before = 2_000;
        c.not_after = 1_000;
        assert_eq!(c.validate(), Err(errors::CERTIFICATE_LOAD));
    }

    #[test]
    fn key_validate_empty_der_rejected() {
        let k = PrivateKey { der: Vec::new() };
        assert_eq!(k.validate(), Err(errors::KEY_LOAD));
    }

    #[test]
    fn key_validate_huge_der_is_bad_memory() {
        let k = PrivateKey { der: vec![0u8; 8 * 1024 + 1] };
        assert_eq!(k.validate(), Err(errors::BAD_MEMORY));
    }

    #[test]
    fn init_end_round_trip() {
        let mut m = HttpsManager::new();
        m.init().unwrap();
        assert!(m.is_initialized());
        m.end().unwrap();
        assert!(!m.is_initialized());
    }

    #[test]
    fn init_twice_rejected() {
        let mut m = HttpsManager::new();
        m.init().unwrap();
        assert_eq!(m.init(), Err(errors::CONTEXT_CREATION));
    }

    #[test]
    fn end_without_init_is_no_ssl() {
        let mut m = HttpsManager::new();
        assert_eq!(m.end(), Err(errors::NO_SSL));
    }

    #[test]
    fn set_ca_list_without_init_is_no_ssl() {
        let mut m = HttpsManager::new();
        assert_eq!(m.set_ca_list(vec![ok_cert("s", "i")]), Err(errors::NO_SSL));
    }

    #[test]
    fn set_ca_list_oversize_rejected() {
        let mut m = HttpsManager::new();
        m.init().unwrap();
        let certs: Vec<_> = (0..=MAX_CA_LIST).map(|i| ok_cert(&format!("s{i}"), &format!("i{i}"))).collect();
        assert_eq!(m.set_ca_list(certs), Err(errors::BAD_MEMORY));
    }

    #[test]
    fn set_ca_list_invalid_cert_rejected() {
        let mut m = HttpsManager::new();
        m.init().unwrap();
        let bad = Certificate { der: Vec::new(), ..ok_cert("s", "i") };
        assert_eq!(m.set_ca_list(vec![bad]), Err(errors::CERTIFICATE_LOAD));
    }

    #[test]
    fn set_ca_list_replaces_existing() {
        let mut m = HttpsManager::new();
        m.init().unwrap();
        m.set_ca_list(vec![ok_cert("a", "a"), ok_cert("b", "b")]).unwrap();
        assert_eq!(m.ca_list_len(), 2);
        m.set_ca_list(vec![ok_cert("c", "c")]).unwrap();
        assert_eq!(m.ca_list_len(), 1);
    }

    #[test]
    fn create_context_happy_path() {
        let mut m = initialized_with_ca();
        let id = m.create_context(TlsVersion::Tls12).unwrap();
        assert_eq!(id, 1);
        assert_eq!(m.context_count(), 1);
    }

    #[test]
    fn create_context_without_init_rejected() {
        let mut m = HttpsManager::new();
        assert_eq!(m.create_context(TlsVersion::Tls12), Err(errors::NO_SSL));
    }

    #[test]
    fn destroy_context_bad_id_rejected() {
        let mut m = initialized_with_ca();
        assert_eq!(m.destroy_context(999), Err(errors::CONTEXT_CREATION));
    }

    #[test]
    fn destroy_context_with_live_connection_rejected() {
        let mut m = initialized_with_ca();
        let ctx = m.create_context(TlsVersion::Tls12).unwrap();
        m.create_connection(ctx, 42).unwrap();
        assert_eq!(m.destroy_context(ctx), Err(errors::CONTEXT_CREATION));
    }

    #[test]
    fn set_client_identity_cert_key_mismatch_rejected() {
        let mut m = initialized_with_ca();
        let ctx = m.create_context(TlsVersion::Tls12).unwrap();
        let mut key = ok_key();
        key.der[0] = 0x00; // Break the 4-byte prefix match.
        assert_eq!(m.set_client_identity(ctx, ok_cert("me", "root"), key), Err(errors::CERT_KEY_MISMATCH));
    }

    #[test]
    fn set_client_identity_happy_path() {
        let mut m = initialized_with_ca();
        let ctx = m.create_context(TlsVersion::Tls12).unwrap();
        m.set_client_identity(ctx, ok_cert("me", "root"), ok_key()).unwrap();
    }

    #[test]
    fn set_client_cert_only_errors() {
        let mut m = initialized_with_ca();
        let ctx = m.create_context(TlsVersion::Tls12).unwrap();
        assert_eq!(m.set_client_cert_only(ctx, ok_cert("s", "i")), Err(errors::CERT_NEEDS_KEY));
    }

    #[test]
    fn set_client_key_only_errors() {
        let mut m = initialized_with_ca();
        let ctx = m.create_context(TlsVersion::Tls12).unwrap();
        assert_eq!(m.set_client_key_only(ctx, ok_key()), Err(errors::KEY_NEEDS_CERT));
    }

    #[test]
    fn create_connection_happy_path() {
        let mut m = initialized_with_ca();
        let ctx = m.create_context(TlsVersion::Tls12).unwrap();
        let conn = m.create_connection(ctx, 7).unwrap();
        assert_eq!(conn, 1);
        assert_eq!(m.connection_count(), 1);
    }

    #[test]
    fn create_connection_bad_socket_rejected() {
        let mut m = initialized_with_ca();
        let ctx = m.create_context(TlsVersion::Tls12).unwrap();
        assert_eq!(m.create_connection(ctx, -1), Err(errors::SOCKET_ASSOCIATION));
    }

    #[test]
    fn create_connection_bad_context_rejected() {
        let mut m = initialized_with_ca();
        assert_eq!(m.create_connection(999, 7), Err(errors::CONTEXT_CREATION));
    }

    #[test]
    fn destroy_connection_bad_id_rejected() {
        let mut m = initialized_with_ca();
        assert_eq!(m.destroy_connection(999), Err(errors::CONNECTION_CREATION));
    }

    #[test]
    fn complete_handshake_trusted_cert() {
        let mut m = initialized_with_ca();
        let ctx = m.create_context(TlsVersion::Tls12).unwrap();
        let conn = m.create_connection(ctx, 7).unwrap();
        let peer = ok_cert("server.example.com", "Root CA");
        m.complete_handshake(conn, peer).unwrap();
        m.require_handshake(conn).unwrap();
    }

    #[test]
    fn complete_handshake_untrusted_cert_rejected() {
        let mut m = initialized_with_ca();
        let ctx = m.create_context(TlsVersion::Tls12).unwrap();
        let conn = m.create_connection(ctx, 7).unwrap();
        let peer = ok_cert("evil.example.com", "Untrusted");
        assert_eq!(m.complete_handshake(conn, peer), Err(errors::LOOKUP_CERTIFICATE));
    }

    #[test]
    fn complete_handshake_twice_rejected() {
        let mut m = initialized_with_ca();
        let ctx = m.create_context(TlsVersion::Tls12).unwrap();
        let conn = m.create_connection(ctx, 7).unwrap();
        m.complete_handshake(conn, ok_cert("srv", "Root CA")).unwrap();
        assert_eq!(m.complete_handshake(conn, ok_cert("srv2", "Root CA")), Err(errors::HANDSHAKE));
    }

    #[test]
    fn require_handshake_before_complete_fails() {
        let mut m = initialized_with_ca();
        let ctx = m.create_context(TlsVersion::Tls12).unwrap();
        let conn = m.create_connection(ctx, 7).unwrap();
        assert_eq!(m.require_handshake(conn), Err(errors::HANDSHAKE));
    }

    #[test]
    fn peer_cert_before_handshake_is_handshake_error() {
        let mut m = initialized_with_ca();
        let ctx = m.create_context(TlsVersion::Tls12).unwrap();
        let conn = m.create_connection(ctx, 7).unwrap();
        assert_eq!(m.peer_cert(conn).err(), Some(errors::HANDSHAKE));
    }

    #[test]
    fn peer_cert_after_handshake_returned() {
        let mut m = initialized_with_ca();
        let ctx = m.create_context(TlsVersion::Tls12).unwrap();
        let conn = m.create_connection(ctx, 7).unwrap();
        m.complete_handshake(conn, ok_cert("srv", "Root CA")).unwrap();
        assert_eq!(m.peer_cert(conn).unwrap().subject, "srv");
    }

    #[test]
    fn full_https_lifecycle_smoke() {
        let mut m = HttpsManager::new();
        m.init().unwrap();
        m.set_ca_list(vec![ok_cert("Corporate CA", "Corporate CA")]).unwrap();
        let ctx = m.create_context(TlsVersion::Tls12).unwrap();
        m.set_client_identity(ctx, ok_cert("client-id", "Corporate CA"), ok_key()).unwrap();
        let conn = m.create_connection(ctx, 101).unwrap();
        m.complete_handshake(conn, ok_cert("api.corp.example.com", "Corporate CA")).unwrap();
        m.require_handshake(conn).unwrap();
        assert_eq!(m.peer_cert(conn).unwrap().subject, "api.corp.example.com");
        m.destroy_connection(conn).unwrap();
        m.destroy_context(ctx).unwrap();
        m.end().unwrap();
    }
}
