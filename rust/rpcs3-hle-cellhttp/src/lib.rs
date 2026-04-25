//! `rpcs3-hle-cellhttp` — HTTP client library HLE.
//!
//! Ports `rpcs3/Emu/Cell/Modules/cellHttp.cpp`. libHttp is the PS3 HTTP
//! stack that games use for patch downloads, marketplace queries,
//! in-game storefronts, etc. The object model is:
//!
//! 1. `Init` — registers a memory container for the library.
//! 2. `CreateClient` — allocate a client (holds cookies / persistent
//!    connection config).
//! 3. `CreateTransaction(client, method, uri)` — associate a request
//!    with a client.
//! 4. `SendRequest` / `RecvResponse` / `AddRequestHeader` / `GetStatusCode`.
//! 5. `DestroyTransaction` / `DestroyClient` / `End`.
//!
//! The Rust model focuses on lifecycle + URI validation + header map +
//! method enum + response store. Real IO is plug-in.

use rpcs3_emu_types::CellError;
use std::collections::HashMap;

// =====================================================================
// Error codes — byte-exact with cellHttp.h:31-118
// (Full error table is huge; we surface the high-frequency codes used
//  by games + the broad network/error category headers.)
// =====================================================================

pub mod errors {
    use rpcs3_emu_types::CellError;

    pub const ALREADY_INITIALIZED: CellError = CellError(0x8071_0001);
    pub const NOT_INITIALIZED: CellError = CellError(0x8071_0002);
    pub const NO_MEMORY: CellError = CellError(0x8071_0003);
    pub const NO_BUFFER: CellError = CellError(0x8071_0004);
    pub const NO_STRING: CellError = CellError(0x8071_0005);
    pub const INSUFFICIENT: CellError = CellError(0x8071_0006);
    pub const INVALID_URI: CellError = CellError(0x8071_0007);
    pub const INVALID_HEADER: CellError = CellError(0x8071_0008);
    pub const BAD_METHOD: CellError = CellError(0x8071_0009);
    pub const BAD_CLIENT: CellError = CellError(0x8071_0010);
    pub const BAD_TRANS: CellError = CellError(0x8071_0011);
    pub const NO_CONNECTION: CellError = CellError(0x8071_0012);
    pub const NO_REQUEST_SENT: CellError = CellError(0x8071_0013);
    pub const ALREADY_BUILT: CellError = CellError(0x8071_0014);
    pub const ALREADY_SENT: CellError = CellError(0x8071_0015);
    pub const NO_HEADER: CellError = CellError(0x8071_0016);
    pub const NO_CONTENT_LENGTH: CellError = CellError(0x8071_0017);
    pub const TOO_MANY_REDIRECTS: CellError = CellError(0x8071_0018);
    pub const TOO_MANY_AUTHS: CellError = CellError(0x8071_0019);
    pub const TRANS_NO_CONNECTION: CellError = CellError(0x8071_0020);
    pub const CB_FAILED: CellError = CellError(0x8071_0021);
    pub const NOT_PIPED: CellError = CellError(0x8071_0022);
    pub const OUT_OF_ORDER_PIPE: CellError = CellError(0x8071_0023);
    pub const TRANS_ABORTED: CellError = CellError(0x8071_0024);
    pub const BROKEN_PIPELINE: CellError = CellError(0x8071_0025);
    pub const UNAVAILABLE: CellError = CellError(0x8071_0026);
    pub const INVALID_VALUE: CellError = CellError(0x8071_0027);
    pub const CANNOT_AUTHENTICATE: CellError = CellError(0x8071_0028);
    pub const COOKIE_NOT_FOUND: CellError = CellError(0x8071_0041);
    pub const COOKIE_INVALID_DOMAIN: CellError = CellError(0x8071_0042);
    pub const LINE_EXCEEDS_MAX: CellError = CellError(0x8071_0045);
    pub const REQUIRES_BASIC_AUTH: CellError = CellError(0x8071_0046);
    pub const UNKNOWN: CellError = CellError(0x8071_0051);
    pub const INTERNAL: CellError = CellError(0x8071_0052);
    pub const NET_CONNECT_TIMEOUT: CellError = CellError(0x8071_0092);
    pub const NET_SELECT_TIMEOUT: CellError = CellError(0x8071_0093);
    pub const NET_SEND_TIMEOUT: CellError = CellError(0x8071_0094);
}

/// Broad network error category anchors. Low byte of each code carries
/// the sub-error; high-byte is the category.
pub mod net_categories {
    pub const TYPE_MASK: u32 = 0xffff_ff00;
    pub const ERROR_MASK: u32 = 0xff;

    pub const RESOLVER: u32 = 0x8071_0100;
    pub const ABORT: u32 = 0x8071_0200;
    pub const OPTION: u32 = 0x8071_0300;
    pub const SOCKET: u32 = 0x8071_0400;
    pub const CONNECT: u32 = 0x8071_0500;
    pub const SEND: u32 = 0x8071_0600;
    pub const RECV: u32 = 0x8071_0700;
    pub const SELECT: u32 = 0x8071_0800;
}

// =====================================================================
// Limits
// =====================================================================

pub const MAX_USERNAME: usize = 256;
pub const MAX_PASSWORD: usize = 256;

/// Per-header line max (PS3 matches the HTTP/1.1 recommendation of 8KB;
/// the real lib rejects lines >16KB with LINE_EXCEEDS_MAX).
pub const MAX_HEADER_LINE: usize = 16 * 1024;

/// Uncapped redirect chain safety limit.
pub const MAX_REDIRECTS: u32 = 5;

// =====================================================================
// HTTP methods
// =====================================================================

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Method {
    Get,
    Head,
    Post,
    Put,
    Delete,
    Options,
    Connect,
    Trace,
}

impl Method {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Get => "GET",
            Self::Head => "HEAD",
            Self::Post => "POST",
            Self::Put => "PUT",
            Self::Delete => "DELETE",
            Self::Options => "OPTIONS",
            Self::Connect => "CONNECT",
            Self::Trace => "TRACE",
        }
    }

    /// Parse the canonical upper-case spelling; anything else → None.
    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "GET" => Self::Get,
            "HEAD" => Self::Head,
            "POST" => Self::Post,
            "PUT" => Self::Put,
            "DELETE" => Self::Delete,
            "OPTIONS" => Self::Options,
            "CONNECT" => Self::Connect,
            "TRACE" => Self::Trace,
            _ => return None,
        })
    }
}

// =====================================================================
// URI validation
// =====================================================================

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Uri {
    pub scheme: String,
    pub host: String,
    pub port: u16,
    pub path: String,
}

impl Uri {
    /// Parse a minimal `http://host[:port]/path` or `https://…` URI.
    pub fn parse(raw: &str) -> Result<Self, CellError> {
        let (scheme, rest) = if let Some(r) = raw.strip_prefix("http://") {
            ("http", r)
        } else if let Some(r) = raw.strip_prefix("https://") {
            ("https", r)
        } else {
            return Err(errors::INVALID_URI);
        };
        let (authority, path) = rest.split_once('/').map_or((rest, ""), |(a, p)| (a, p));
        if authority.is_empty() {
            return Err(errors::INVALID_URI);
        }
        let (host, port_str) = authority.split_once(':').map_or((authority, None), |(h, p)| (h, Some(p)));
        if host.is_empty() {
            return Err(errors::INVALID_URI);
        }
        let port = if let Some(p) = port_str {
            p.parse::<u16>().map_err(|_| errors::INVALID_URI)?
        } else {
            match scheme {
                "http" => 80,
                "https" => 443,
                _ => return Err(errors::INVALID_URI),
            }
        };
        // Path is what comes after the authority '/' — reintroduce the '/'.
        let path = if path.is_empty() { "/".to_string() } else { format!("/{path}") };
        Ok(Self { scheme: scheme.into(), host: host.into(), port, path })
    }

    #[must_use]
    pub fn build(&self) -> String {
        let default = matches!(
            (self.scheme.as_str(), self.port),
            ("http", 80) | ("https", 443)
        );
        if default {
            format!("{}://{}{}", self.scheme, self.host, self.path)
        } else {
            format!("{}://{}:{}{}", self.scheme, self.host, self.port, self.path)
        }
    }
}

// =====================================================================
// Transactions
// =====================================================================

#[derive(Clone, Debug, PartialEq, Eq)]
enum TransState {
    Built,
    Sent,
    Received,
}

#[derive(Clone, Debug)]
struct Transaction {
    id: u32,
    client_id: u32,
    method: Method,
    uri: Uri,
    headers: HashMap<String, String>,
    state: TransState,
    status_code: u16,
    response_headers: HashMap<String, String>,
    response_body: Vec<u8>,
}

#[derive(Clone, Debug, Default)]
struct Client {
    id: u32,
    user_agent: String,
    basic_auth: Option<(String, String)>,
    cookies: HashMap<String, String>,
}

// =====================================================================
// Manager
// =====================================================================

#[derive(Clone, Debug)]
pub struct HttpManager {
    initialized: bool,
    clients: Vec<Client>,
    transactions: Vec<Transaction>,
    next_client_id: u32,
    next_trans_id: u32,
}

impl HttpManager {
    #[must_use]
    pub fn new() -> Self {
        Self {
            initialized: false,
            clients: Vec::new(),
            transactions: Vec::new(),
            next_client_id: 1,
            next_trans_id: 1,
        }
    }

    #[must_use]
    pub fn is_initialized(&self) -> bool {
        self.initialized
    }

    #[must_use]
    pub fn client_count(&self) -> usize {
        self.clients.len()
    }

    #[must_use]
    pub fn transaction_count(&self) -> usize {
        self.transactions.len()
    }

    // ----------------- Lifecycle -----------------

    pub fn init(&mut self) -> Result<(), CellError> {
        if self.initialized {
            return Err(errors::ALREADY_INITIALIZED);
        }
        self.initialized = true;
        self.clients.clear();
        self.transactions.clear();
        self.next_client_id = 1;
        self.next_trans_id = 1;
        Ok(())
    }

    pub fn end(&mut self) -> Result<(), CellError> {
        if !self.initialized {
            return Err(errors::NOT_INITIALIZED);
        }
        self.initialized = false;
        self.clients.clear();
        self.transactions.clear();
        Ok(())
    }

    // ----------------- Client -----------------

    pub fn create_client(&mut self) -> Result<u32, CellError> {
        self.require_initialized()?;
        let id = self.next_client_id;
        self.next_client_id = self.next_client_id.checked_add(1).ok_or(errors::NO_MEMORY)?;
        self.clients.push(Client { id, ..Default::default() });
        Ok(id)
    }

    pub fn destroy_client(&mut self, client_id: u32) -> Result<(), CellError> {
        self.require_initialized()?;
        // Reject if any transaction still references this client.
        if self.transactions.iter().any(|t| t.client_id == client_id) {
            return Err(errors::BAD_CLIENT);
        }
        let idx = self.clients.iter().position(|c| c.id == client_id).ok_or(errors::BAD_CLIENT)?;
        self.clients.remove(idx);
        Ok(())
    }

    pub fn set_user_agent(&mut self, client_id: u32, ua: impl Into<String>) -> Result<(), CellError> {
        self.require_initialized()?;
        let ua = ua.into();
        if ua.len() > MAX_HEADER_LINE {
            return Err(errors::LINE_EXCEEDS_MAX);
        }
        let idx = self.client_idx(client_id)?;
        self.clients[idx].user_agent = ua;
        Ok(())
    }

    pub fn set_basic_auth(
        &mut self,
        client_id: u32,
        username: impl Into<String>,
        password: impl Into<String>,
    ) -> Result<(), CellError> {
        self.require_initialized()?;
        let u = username.into();
        let p = password.into();
        if u.len() > MAX_USERNAME || p.len() > MAX_PASSWORD {
            return Err(errors::INVALID_VALUE);
        }
        let idx = self.client_idx(client_id)?;
        self.clients[idx].basic_auth = Some((u, p));
        Ok(())
    }

    pub fn add_cookie(
        &mut self,
        client_id: u32,
        name: impl Into<String>,
        value: impl Into<String>,
    ) -> Result<(), CellError> {
        self.require_initialized()?;
        let name = name.into();
        let value = value.into();
        if name.is_empty() || name.contains('=') {
            return Err(errors::COOKIE_INVALID_DOMAIN);
        }
        let idx = self.client_idx(client_id)?;
        self.clients[idx].cookies.insert(name, value);
        Ok(())
    }

    pub fn get_cookie(&self, client_id: u32, name: &str) -> Result<&str, CellError> {
        self.require_initialized()?;
        let idx = self.client_idx(client_id)?;
        self.clients[idx].cookies.get(name).map(String::as_str).ok_or(errors::COOKIE_NOT_FOUND)
    }

    // ----------------- Transaction -----------------

    pub fn create_transaction(
        &mut self,
        client_id: u32,
        method: Method,
        uri: &str,
    ) -> Result<u32, CellError> {
        self.require_initialized()?;
        let _ = self.client_idx(client_id)?;
        let parsed = Uri::parse(uri)?;
        let id = self.next_trans_id;
        self.next_trans_id = self.next_trans_id.checked_add(1).ok_or(errors::NO_MEMORY)?;
        self.transactions.push(Transaction {
            id,
            client_id,
            method,
            uri: parsed,
            headers: HashMap::new(),
            state: TransState::Built,
            status_code: 0,
            response_headers: HashMap::new(),
            response_body: Vec::new(),
        });
        Ok(id)
    }

    pub fn destroy_transaction(&mut self, trans_id: u32) -> Result<(), CellError> {
        self.require_initialized()?;
        let idx = self.trans_idx(trans_id)?;
        self.transactions.remove(idx);
        Ok(())
    }

    pub fn add_request_header(
        &mut self,
        trans_id: u32,
        name: impl Into<String>,
        value: impl Into<String>,
    ) -> Result<(), CellError> {
        self.require_initialized()?;
        let name = name.into();
        let value = value.into();
        if name.is_empty() || name.contains([':', '\r', '\n']) {
            return Err(errors::INVALID_HEADER);
        }
        if value.contains(['\r', '\n']) {
            return Err(errors::INVALID_HEADER);
        }
        if name.len() + value.len() + 2 > MAX_HEADER_LINE {
            return Err(errors::LINE_EXCEEDS_MAX);
        }
        let idx = self.trans_idx(trans_id)?;
        if self.transactions[idx].state != TransState::Built {
            return Err(errors::ALREADY_SENT);
        }
        self.transactions[idx].headers.insert(name, value);
        Ok(())
    }

    pub fn send_request(&mut self, trans_id: u32) -> Result<(), CellError> {
        self.require_initialized()?;
        let idx = self.trans_idx(trans_id)?;
        match self.transactions[idx].state {
            TransState::Built => {}
            TransState::Sent | TransState::Received => return Err(errors::ALREADY_SENT),
        }
        self.transactions[idx].state = TransState::Sent;
        Ok(())
    }

    /// Test hook: deliver a response. Real lib receives this via network.
    pub fn inject_response(
        &mut self,
        trans_id: u32,
        status_code: u16,
        headers: HashMap<String, String>,
        body: Vec<u8>,
    ) -> Result<(), CellError> {
        self.require_initialized()?;
        let idx = self.trans_idx(trans_id)?;
        if self.transactions[idx].state != TransState::Sent {
            return Err(errors::NO_REQUEST_SENT);
        }
        self.transactions[idx].status_code = status_code;
        self.transactions[idx].response_headers = headers;
        self.transactions[idx].response_body = body;
        self.transactions[idx].state = TransState::Received;
        Ok(())
    }

    pub fn get_status_code(&self, trans_id: u32) -> Result<u16, CellError> {
        self.require_initialized()?;
        let idx = self.trans_idx(trans_id)?;
        if self.transactions[idx].state != TransState::Received {
            return Err(errors::NO_REQUEST_SENT);
        }
        Ok(self.transactions[idx].status_code)
    }

    pub fn get_response_header(&self, trans_id: u32, name: &str) -> Result<&str, CellError> {
        self.require_initialized()?;
        let idx = self.trans_idx(trans_id)?;
        if self.transactions[idx].state != TransState::Received {
            return Err(errors::NO_REQUEST_SENT);
        }
        self.transactions[idx].response_headers.get(name).map(String::as_str).ok_or(errors::NO_HEADER)
    }

    pub fn get_content_length(&self, trans_id: u32) -> Result<u64, CellError> {
        let hdr = self.get_response_header(trans_id, "Content-Length")?;
        hdr.parse::<u64>().map_err(|_| errors::NO_CONTENT_LENGTH)
    }

    pub fn read_response_body(&mut self, trans_id: u32, out: &mut [u8]) -> Result<usize, CellError> {
        self.require_initialized()?;
        let idx = self.trans_idx(trans_id)?;
        if self.transactions[idx].state != TransState::Received {
            return Err(errors::NO_REQUEST_SENT);
        }
        let take = self.transactions[idx].response_body.len().min(out.len());
        let drained: Vec<u8> = self.transactions[idx].response_body.drain(..take).collect();
        out[..take].copy_from_slice(&drained);
        Ok(take)
    }

    // ----------------- Helpers -----------------

    fn require_initialized(&self) -> Result<(), CellError> {
        if self.initialized { Ok(()) } else { Err(errors::NOT_INITIALIZED) }
    }

    fn client_idx(&self, id: u32) -> Result<usize, CellError> {
        self.clients.iter().position(|c| c.id == id).ok_or(errors::BAD_CLIENT)
    }

    fn trans_idx(&self, id: u32) -> Result<usize, CellError> {
        self.transactions.iter().position(|t| t.id == id).ok_or(errors::BAD_TRANS)
    }
}

impl Default for HttpManager {
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

    fn initialized() -> HttpManager {
        let mut m = HttpManager::new();
        m.init().unwrap();
        m
    }

    #[test]
    fn error_codes_byte_exact() {
        assert_eq!(errors::ALREADY_INITIALIZED.0, 0x8071_0001);
        assert_eq!(errors::NOT_INITIALIZED.0, 0x8071_0002);
        assert_eq!(errors::INVALID_URI.0, 0x8071_0007);
        assert_eq!(errors::INVALID_HEADER.0, 0x8071_0008);
        assert_eq!(errors::BAD_METHOD.0, 0x8071_0009);
        assert_eq!(errors::BAD_CLIENT.0, 0x8071_0010);
        assert_eq!(errors::BAD_TRANS.0, 0x8071_0011);
        assert_eq!(errors::ALREADY_SENT.0, 0x8071_0015);
        assert_eq!(errors::NO_HEADER.0, 0x8071_0016);
        assert_eq!(errors::COOKIE_NOT_FOUND.0, 0x8071_0041);
        assert_eq!(errors::LINE_EXCEEDS_MAX.0, 0x8071_0045);
        assert_eq!(errors::NET_SEND_TIMEOUT.0, 0x8071_0094);
    }

    #[test]
    fn net_categories_stable() {
        assert_eq!(net_categories::TYPE_MASK, 0xffff_ff00);
        assert_eq!(net_categories::ERROR_MASK, 0xff);
        assert_eq!(net_categories::RESOLVER, 0x8071_0100);
        assert_eq!(net_categories::CONNECT, 0x8071_0500);
        assert_eq!(net_categories::SEND, 0x8071_0600);
        assert_eq!(net_categories::RECV, 0x8071_0700);
    }

    #[test]
    fn constants_stable() {
        assert_eq!(MAX_USERNAME, 256);
        assert_eq!(MAX_PASSWORD, 256);
        assert_eq!(MAX_HEADER_LINE, 16 * 1024);
        assert_eq!(MAX_REDIRECTS, 5);
    }

    #[test]
    fn method_round_trip_all_8_methods() {
        for method in [
            Method::Get,
            Method::Head,
            Method::Post,
            Method::Put,
            Method::Delete,
            Method::Options,
            Method::Connect,
            Method::Trace,
        ] {
            assert_eq!(Method::parse(method.as_str()), Some(method));
        }
    }

    #[test]
    fn method_parse_rejects_unknown_and_case() {
        assert_eq!(Method::parse("get"), None);
        assert_eq!(Method::parse("PATCH"), None);
        assert_eq!(Method::parse(""), None);
    }

    #[test]
    fn uri_parse_http_default_port() {
        let u = Uri::parse("http://example.com").unwrap();
        assert_eq!(u.scheme, "http");
        assert_eq!(u.host, "example.com");
        assert_eq!(u.port, 80);
        assert_eq!(u.path, "/");
    }

    #[test]
    fn uri_parse_https_default_port() {
        let u = Uri::parse("https://api.example.com/v1/data").unwrap();
        assert_eq!(u.scheme, "https");
        assert_eq!(u.port, 443);
        assert_eq!(u.path, "/v1/data");
    }

    #[test]
    fn uri_parse_explicit_port() {
        let u = Uri::parse("http://host:8080/a/b").unwrap();
        assert_eq!(u.port, 8080);
        assert_eq!(u.path, "/a/b");
    }

    #[test]
    fn uri_parse_missing_scheme_rejected() {
        assert_eq!(Uri::parse("example.com").err(), Some(errors::INVALID_URI));
        assert_eq!(Uri::parse("ftp://host").err(), Some(errors::INVALID_URI));
    }

    #[test]
    fn uri_parse_empty_host_rejected() {
        assert_eq!(Uri::parse("http://").err(), Some(errors::INVALID_URI));
        assert_eq!(Uri::parse("http:///path").err(), Some(errors::INVALID_URI));
    }

    #[test]
    fn uri_parse_bad_port_rejected() {
        assert_eq!(Uri::parse("http://host:99999").err(), Some(errors::INVALID_URI));
        assert_eq!(Uri::parse("http://host:abc").err(), Some(errors::INVALID_URI));
    }

    #[test]
    fn uri_build_round_trip_default_port() {
        let u = Uri::parse("http://example.com/x").unwrap();
        assert_eq!(u.build(), "http://example.com/x");
    }

    #[test]
    fn uri_build_round_trip_custom_port() {
        let u = Uri::parse("https://host:8443/api").unwrap();
        assert_eq!(u.build(), "https://host:8443/api");
    }

    #[test]
    fn init_and_end_round_trip() {
        let mut m = HttpManager::new();
        m.init().unwrap();
        assert!(m.is_initialized());
        m.end().unwrap();
        assert!(!m.is_initialized());
    }

    #[test]
    fn init_twice_rejected() {
        let mut m = initialized();
        assert_eq!(m.init(), Err(errors::ALREADY_INITIALIZED));
    }

    #[test]
    fn end_without_init_rejected() {
        let mut m = HttpManager::new();
        assert_eq!(m.end(), Err(errors::NOT_INITIALIZED));
    }

    #[test]
    fn create_client_without_init_rejected() {
        let mut m = HttpManager::new();
        assert_eq!(m.create_client(), Err(errors::NOT_INITIALIZED));
    }

    #[test]
    fn create_client_increments_id() {
        let mut m = initialized();
        let a = m.create_client().unwrap();
        let b = m.create_client().unwrap();
        assert_eq!(b, a + 1);
    }

    #[test]
    fn destroy_client_bad_id_rejected() {
        let mut m = initialized();
        assert_eq!(m.destroy_client(999), Err(errors::BAD_CLIENT));
    }

    #[test]
    fn destroy_client_with_live_transaction_rejected() {
        let mut m = initialized();
        let c = m.create_client().unwrap();
        m.create_transaction(c, Method::Get, "http://x.com/a").unwrap();
        assert_eq!(m.destroy_client(c), Err(errors::BAD_CLIENT));
    }

    #[test]
    fn set_user_agent_oversized_rejected() {
        let mut m = initialized();
        let c = m.create_client().unwrap();
        let long = "a".repeat(MAX_HEADER_LINE + 1);
        assert_eq!(m.set_user_agent(c, long), Err(errors::LINE_EXCEEDS_MAX));
    }

    #[test]
    fn set_basic_auth_oversized_username_rejected() {
        let mut m = initialized();
        let c = m.create_client().unwrap();
        let long = "u".repeat(MAX_USERNAME + 1);
        assert_eq!(m.set_basic_auth(c, long, "pw"), Err(errors::INVALID_VALUE));
    }

    #[test]
    fn add_cookie_empty_name_rejected() {
        let mut m = initialized();
        let c = m.create_client().unwrap();
        assert_eq!(m.add_cookie(c, "", "v"), Err(errors::COOKIE_INVALID_DOMAIN));
    }

    #[test]
    fn add_cookie_name_with_equals_rejected() {
        let mut m = initialized();
        let c = m.create_client().unwrap();
        assert_eq!(m.add_cookie(c, "bad=name", "v"), Err(errors::COOKIE_INVALID_DOMAIN));
    }

    #[test]
    fn add_and_get_cookie_round_trip() {
        let mut m = initialized();
        let c = m.create_client().unwrap();
        m.add_cookie(c, "session", "abc123").unwrap();
        assert_eq!(m.get_cookie(c, "session"), Ok("abc123"));
        assert_eq!(m.get_cookie(c, "missing"), Err(errors::COOKIE_NOT_FOUND));
    }

    #[test]
    fn create_transaction_bad_client_rejected() {
        let mut m = initialized();
        assert_eq!(
            m.create_transaction(999, Method::Get, "http://x.com/"),
            Err(errors::BAD_CLIENT)
        );
    }

    #[test]
    fn create_transaction_bad_uri_rejected() {
        let mut m = initialized();
        let c = m.create_client().unwrap();
        assert_eq!(
            m.create_transaction(c, Method::Get, "not-a-uri"),
            Err(errors::INVALID_URI)
        );
    }

    #[test]
    fn destroy_transaction_bad_id_rejected() {
        let mut m = initialized();
        assert_eq!(m.destroy_transaction(999), Err(errors::BAD_TRANS));
    }

    #[test]
    fn add_request_header_happy_path() {
        let mut m = initialized();
        let c = m.create_client().unwrap();
        let t = m.create_transaction(c, Method::Get, "http://x.com/").unwrap();
        m.add_request_header(t, "X-Token", "abc").unwrap();
    }

    #[test]
    fn add_request_header_empty_name_rejected() {
        let mut m = initialized();
        let c = m.create_client().unwrap();
        let t = m.create_transaction(c, Method::Get, "http://x.com/").unwrap();
        assert_eq!(m.add_request_header(t, "", "v"), Err(errors::INVALID_HEADER));
    }

    #[test]
    fn add_request_header_colon_in_name_rejected() {
        let mut m = initialized();
        let c = m.create_client().unwrap();
        let t = m.create_transaction(c, Method::Get, "http://x.com/").unwrap();
        assert_eq!(m.add_request_header(t, "X:Bad", "v"), Err(errors::INVALID_HEADER));
    }

    #[test]
    fn add_request_header_newline_in_value_rejected() {
        let mut m = initialized();
        let c = m.create_client().unwrap();
        let t = m.create_transaction(c, Method::Get, "http://x.com/").unwrap();
        assert_eq!(m.add_request_header(t, "X", "v\nattack"), Err(errors::INVALID_HEADER));
    }

    #[test]
    fn add_request_header_after_send_rejected() {
        let mut m = initialized();
        let c = m.create_client().unwrap();
        let t = m.create_transaction(c, Method::Get, "http://x.com/").unwrap();
        m.send_request(t).unwrap();
        assert_eq!(m.add_request_header(t, "X", "v"), Err(errors::ALREADY_SENT));
    }

    #[test]
    fn send_request_twice_rejected() {
        let mut m = initialized();
        let c = m.create_client().unwrap();
        let t = m.create_transaction(c, Method::Get, "http://x.com/").unwrap();
        m.send_request(t).unwrap();
        assert_eq!(m.send_request(t), Err(errors::ALREADY_SENT));
    }

    #[test]
    fn get_status_code_before_receive_rejected() {
        let mut m = initialized();
        let c = m.create_client().unwrap();
        let t = m.create_transaction(c, Method::Get, "http://x.com/").unwrap();
        assert_eq!(m.get_status_code(t), Err(errors::NO_REQUEST_SENT));
        m.send_request(t).unwrap();
        assert_eq!(m.get_status_code(t), Err(errors::NO_REQUEST_SENT));
    }

    #[test]
    fn inject_response_happy_path() {
        let mut m = initialized();
        let c = m.create_client().unwrap();
        let t = m.create_transaction(c, Method::Get, "http://x.com/").unwrap();
        m.send_request(t).unwrap();
        let mut hdrs = HashMap::new();
        hdrs.insert("Content-Type".into(), "application/json".into());
        hdrs.insert("Content-Length".into(), "42".into());
        m.inject_response(t, 200, hdrs, b"{\"ok\":true}".to_vec()).unwrap();
        assert_eq!(m.get_status_code(t), Ok(200));
        assert_eq!(m.get_response_header(t, "Content-Type"), Ok("application/json"));
        assert_eq!(m.get_content_length(t), Ok(42));
    }

    #[test]
    fn inject_response_without_send_rejected() {
        let mut m = initialized();
        let c = m.create_client().unwrap();
        let t = m.create_transaction(c, Method::Get, "http://x.com/").unwrap();
        assert_eq!(
            m.inject_response(t, 200, HashMap::new(), vec![]),
            Err(errors::NO_REQUEST_SENT)
        );
    }

    #[test]
    fn get_response_header_missing_is_no_header() {
        let mut m = initialized();
        let c = m.create_client().unwrap();
        let t = m.create_transaction(c, Method::Get, "http://x.com/").unwrap();
        m.send_request(t).unwrap();
        m.inject_response(t, 200, HashMap::new(), vec![]).unwrap();
        assert_eq!(m.get_response_header(t, "X-Missing"), Err(errors::NO_HEADER));
    }

    #[test]
    fn get_content_length_missing_header_is_no_header() {
        let mut m = initialized();
        let c = m.create_client().unwrap();
        let t = m.create_transaction(c, Method::Get, "http://x.com/").unwrap();
        m.send_request(t).unwrap();
        m.inject_response(t, 204, HashMap::new(), vec![]).unwrap();
        assert_eq!(m.get_content_length(t), Err(errors::NO_HEADER));
    }

    #[test]
    fn get_content_length_malformed_is_no_content_length() {
        let mut m = initialized();
        let c = m.create_client().unwrap();
        let t = m.create_transaction(c, Method::Get, "http://x.com/").unwrap();
        m.send_request(t).unwrap();
        let mut hdrs = HashMap::new();
        hdrs.insert("Content-Length".into(), "chunky".into());
        m.inject_response(t, 200, hdrs, vec![]).unwrap();
        assert_eq!(m.get_content_length(t), Err(errors::NO_CONTENT_LENGTH));
    }

    #[test]
    fn read_response_body_drains_progressively() {
        let mut m = initialized();
        let c = m.create_client().unwrap();
        let t = m.create_transaction(c, Method::Get, "http://x.com/").unwrap();
        m.send_request(t).unwrap();
        m.inject_response(t, 200, HashMap::new(), b"hello world".to_vec()).unwrap();
        let mut buf = [0u8; 5];
        assert_eq!(m.read_response_body(t, &mut buf), Ok(5));
        assert_eq!(&buf, b"hello");
        assert_eq!(m.read_response_body(t, &mut buf), Ok(5));
        assert_eq!(&buf, b" worl");
        assert_eq!(m.read_response_body(t, &mut buf), Ok(1));
        assert_eq!(m.read_response_body(t, &mut buf), Ok(0));
    }

    #[test]
    fn full_http_flow_smoke() {
        let mut m = HttpManager::new();
        m.init().unwrap();
        let c = m.create_client().unwrap();
        m.set_user_agent(c, "PS3/4.00").unwrap();
        m.set_basic_auth(c, "user", "pass").unwrap();
        m.add_cookie(c, "session", "xyz").unwrap();
        let t = m.create_transaction(c, Method::Post, "https://api.example.com:8443/submit").unwrap();
        m.add_request_header(t, "Content-Type", "application/json").unwrap();
        m.add_request_header(t, "Accept", "application/json").unwrap();
        m.send_request(t).unwrap();
        let mut hdrs = HashMap::new();
        hdrs.insert("Content-Length".into(), "11".into());
        m.inject_response(t, 201, hdrs, b"Hello World".to_vec()).unwrap();
        assert_eq!(m.get_status_code(t), Ok(201));
        assert_eq!(m.get_content_length(t), Ok(11));
        let mut buf = [0u8; 11];
        m.read_response_body(t, &mut buf).unwrap();
        assert_eq!(&buf, b"Hello World");
        m.destroy_transaction(t).unwrap();
        m.destroy_client(c).unwrap();
        m.end().unwrap();
    }
}
