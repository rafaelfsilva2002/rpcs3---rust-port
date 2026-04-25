//! `rpcs3-hle-cellhttputil` — HTTP utilities HLE.
//!
//! Ports `rpcs3/Emu/Cell/Modules/cellHttpUtil.cpp`. Stateless helpers
//! for parsing and building URIs, percent-encoding, base64, and
//! formatting / parsing request / status / header lines.
//!
//! ## Entry points covered
//!
//! | HLE function                      | Rust wrapper                |
//! |-----------------------------------|-----------------------------|
//! | `cellHttpUtilParseUri`            | [`Uri::parse`]              |
//! | `cellHttpUtilBuildUri`            | [`Uri::build`]              |
//! | `cellHttpUtilCopyUri`             | [`Uri::copy_into`]          |
//! | `cellHttpUtilEscapeUri`           | [`percent_encode`]          |
//! | `cellHttpUtilUnescapeUri`         | [`percent_decode`]          |
//! | `cellHttpUtilBase64Encoder`       | [`base64_encode`]           |
//! | `cellHttpUtilBase64Decoder`       | [`base64_decode`]           |
//! | `cellHttpUtilFormStatusLine`      | [`StatusLine::format`]      |
//! | `cellHttpUtilParseStatusLine`     | [`StatusLine::parse`]       |
//! | `cellHttpUtilFormRequestLine`     | [`RequestLine::format`]     |
//! | `cellHttpUtilParseHeader`         | [`parse_header`]            |

use rpcs3_emu_types::CellError;

// =====================================================================
// Error codes — byte-exact with cellHttpUtil.h:8-20
// =====================================================================

pub mod errors {
    use rpcs3_emu_types::CellError;

    pub const NO_MEMORY: CellError = CellError(0x8071_1001);
    pub const NO_BUFFER: CellError = CellError(0x8071_1002);
    pub const NO_STRING: CellError = CellError(0x8071_1003);
    pub const INSUFFICIENT: CellError = CellError(0x8071_1004);
    pub const INVALID_URI: CellError = CellError(0x8071_1005);
    pub const INVALID_HEADER: CellError = CellError(0x8071_1006);
    pub const INVALID_REQUEST: CellError = CellError(0x8071_1007);
    pub const INVALID_RESPONSE: CellError = CellError(0x8071_1008);
    pub const INVALID_LENGTH: CellError = CellError(0x8071_1009);
    pub const INVALID_CHARACTER: CellError = CellError(0x8071_100a);
}

// =====================================================================
// URI flags (cellHttpUtil.h:22-29)
// =====================================================================

pub const URI_FLAG_FULL_URI: u32 = 0x0000_0000;
pub const URI_FLAG_NO_SCHEME: u32 = 0x0000_0001;
pub const URI_FLAG_NO_CREDENTIALS: u32 = 0x0000_0002;
pub const URI_FLAG_NO_PASSWORD: u32 = 0x0000_0004;
pub const URI_FLAG_NO_PATH: u32 = 0x0000_0008;
pub const URI_FLAG_ALL_MASK: u32 = 0x0000_000F;

// =====================================================================
// Uri — parse / build
// =====================================================================

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Uri {
    pub scheme: String,
    pub hostname: String,
    pub username: String,
    pub password: String,
    pub path: String,
    pub port: u32,
}

impl Uri {
    /// `cellHttpUtilParseUri`. Accepts:
    /// `scheme://[user[:pass]@]host[:port][/path]`.
    pub fn parse(raw: &str) -> Result<Self, CellError> {
        // Scheme.
        let (scheme, rest) = raw.split_once("://").ok_or(errors::INVALID_URI)?;
        if scheme.is_empty() || !scheme.chars().all(|c| c.is_ascii_alphanumeric() || c == '+' || c == '-' || c == '.') {
            return Err(errors::INVALID_URI);
        }
        // Authority + path.
        let (authority, raw_path) = rest.split_once('/').map_or((rest, ""), |(a, p)| (a, p));
        if authority.is_empty() {
            return Err(errors::INVALID_URI);
        }
        // Credentials.
        let (creds, host_port) = authority.rsplit_once('@').map_or(("", authority), |(c, h)| (c, h));
        let (username, password) = if creds.is_empty() {
            (String::new(), String::new())
        } else {
            creds.split_once(':').map_or((creds.to_string(), String::new()), |(u, p)| (u.to_string(), p.to_string()))
        };
        if host_port.is_empty() {
            return Err(errors::INVALID_URI);
        }
        // Host + port.
        let (hostname, port_str) = host_port.split_once(':').map_or((host_port, None), |(h, p)| (h, Some(p)));
        if hostname.is_empty() {
            return Err(errors::INVALID_URI);
        }
        let port = match port_str {
            Some(p) => p.parse::<u32>().map_err(|_| errors::INVALID_URI)?,
            None => match scheme {
                "http" => 80,
                "https" => 443,
                "ftp" => 21,
                _ => 0,
            },
        };
        if port > 0xFFFF {
            return Err(errors::INVALID_URI);
        }
        let path = if raw_path.is_empty() { "/".to_string() } else { format!("/{raw_path}") };
        Ok(Self { scheme: scheme.into(), hostname: hostname.into(), username, password, path, port })
    }

    /// `cellHttpUtilBuildUri(uri, flags)`. `flags` = `URI_FLAG_*` bitmask
    /// that suppresses scheme / credentials / password / path.
    pub fn build(&self, flags: u32) -> Result<String, CellError> {
        if (flags & !URI_FLAG_ALL_MASK) != 0 {
            return Err(errors::INVALID_LENGTH);
        }
        let mut out = String::new();
        if flags & URI_FLAG_NO_SCHEME == 0 {
            out.push_str(&self.scheme);
            out.push_str("://");
        }
        // Credentials (NO_PASSWORD implies keeping user only).
        if flags & URI_FLAG_NO_CREDENTIALS == 0 && !self.username.is_empty() {
            out.push_str(&self.username);
            if flags & URI_FLAG_NO_PASSWORD == 0 && !self.password.is_empty() {
                out.push(':');
                out.push_str(&self.password);
            }
            out.push('@');
        }
        out.push_str(&self.hostname);
        // Include explicit port only if it's non-default for the scheme.
        let default_port = match self.scheme.as_str() {
            "http" => 80,
            "https" => 443,
            "ftp" => 21,
            _ => 0,
        };
        if self.port != default_port && self.port != 0 {
            out.push(':');
            out.push_str(&self.port.to_string());
        }
        if flags & URI_FLAG_NO_PATH == 0 {
            out.push_str(&self.path);
        }
        Ok(out)
    }

    /// `cellHttpUtilCopyUri(dest, src, pool, poolSize, required)`. Computes
    /// pool bytes needed (sum of nul-terminated strings). Returns the
    /// cloned URI + required pool size. If pool is < required, reports
    /// INSUFFICIENT.
    pub fn copy_into(&self, pool_size: u32) -> Result<(Self, u32), CellError> {
        let required = (self.scheme.len() + 1
            + self.hostname.len() + 1
            + self.username.len() + 1
            + self.password.len() + 1
            + self.path.len() + 1) as u32;
        if pool_size < required {
            return Err(errors::INSUFFICIENT);
        }
        Ok((self.clone(), required))
    }
}

// =====================================================================
// Percent encoding / decoding
// =====================================================================

/// `cellHttpUtilEscapeUri`. Percent-encodes every byte that is not in
/// the "unreserved" set of RFC 3986: `A-Z / a-z / 0-9 / - / _ / . / ~`.
/// Output is always valid ASCII.
#[must_use]
pub fn percent_encode(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for byte in input.bytes() {
        let unreserved = byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~');
        if unreserved {
            out.push(byte as char);
        } else {
            out.push('%');
            out.push(char::from(hex_upper(byte >> 4)));
            out.push(char::from(hex_upper(byte & 0xF)));
        }
    }
    out
}

/// `cellHttpUtilUnescapeUri`. Inverse of [`percent_encode`]. Returns
/// bytes because decoded output isn't guaranteed to be UTF-8.
pub fn percent_decode(input: &str) -> Result<Vec<u8>, CellError> {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if b == b'%' {
            if i + 2 >= bytes.len() {
                return Err(errors::INVALID_CHARACTER);
            }
            let hi = from_hex(bytes[i + 1]).ok_or(errors::INVALID_CHARACTER)?;
            let lo = from_hex(bytes[i + 2]).ok_or(errors::INVALID_CHARACTER)?;
            out.push((hi << 4) | lo);
            i += 3;
        } else if b < 0x20 || b >= 0x7F {
            // Control + high bytes in the raw input are invalid per
            // HTTP header grammar; the real lib rejects them.
            return Err(errors::INVALID_CHARACTER);
        } else {
            out.push(b);
            i += 1;
        }
    }
    Ok(out)
}

const fn hex_upper(nibble: u8) -> u8 {
    if nibble < 10 { b'0' + nibble } else { b'A' + nibble - 10 }
}

const fn from_hex(byte: u8) -> Option<u8> {
    Some(match byte {
        b'0'..=b'9' => byte - b'0',
        b'a'..=b'f' => byte - b'a' + 10,
        b'A'..=b'F' => byte - b'A' + 10,
        _ => return None,
    })
}

// =====================================================================
// Base64
// =====================================================================

const BASE64_ALPHABET: &[u8; 64] =
    b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// `cellHttpUtilBase64Encoder`. Standard RFC 4648 base64 with `=` padding.
#[must_use]
pub fn base64_encode(data: &[u8]) -> String {
    let mut out = String::with_capacity(((data.len() + 2) / 3) * 4);
    for chunk in data.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = chunk.get(1).copied().unwrap_or(0) as u32;
        let b2 = chunk.get(2).copied().unwrap_or(0) as u32;
        let n = (b0 << 16) | (b1 << 8) | b2;
        out.push(char::from(BASE64_ALPHABET[((n >> 18) & 0x3F) as usize]));
        out.push(char::from(BASE64_ALPHABET[((n >> 12) & 0x3F) as usize]));
        if chunk.len() > 1 {
            out.push(char::from(BASE64_ALPHABET[((n >> 6) & 0x3F) as usize]));
        } else {
            out.push('=');
        }
        if chunk.len() > 2 {
            out.push(char::from(BASE64_ALPHABET[(n & 0x3F) as usize]));
        } else {
            out.push('=');
        }
    }
    out
}

/// `cellHttpUtilBase64Decoder`. Lenient: accepts but ignores whitespace.
pub fn base64_decode(input: &str) -> Result<Vec<u8>, CellError> {
    let cleaned: Vec<u8> = input.bytes().filter(|b| !b.is_ascii_whitespace()).collect();
    if cleaned.len() % 4 != 0 {
        return Err(errors::INVALID_LENGTH);
    }
    let pad = cleaned.iter().rev().take_while(|&&b| b == b'=').count();
    if pad > 2 {
        return Err(errors::INVALID_CHARACTER);
    }
    let payload = &cleaned[..cleaned.len() - pad];
    let mut out = Vec::with_capacity((payload.len() / 4) * 3);
    for chunk in cleaned.chunks(4) {
        let mut n: u32 = 0;
        let mut shift = 18;
        let mut real_bytes = 0;
        for &b in chunk {
            if b == b'=' {
                break;
            }
            let v = base64_index(b).ok_or(errors::INVALID_CHARACTER)?;
            n |= (v as u32) << shift;
            shift -= 6;
            real_bytes += 1;
        }
        match real_bytes {
            2 => out.push((n >> 16) as u8),
            3 => {
                out.push((n >> 16) as u8);
                out.push((n >> 8) as u8);
            }
            4 => {
                out.push((n >> 16) as u8);
                out.push((n >> 8) as u8);
                out.push(n as u8);
            }
            _ => return Err(errors::INVALID_CHARACTER),
        }
    }
    Ok(out)
}

fn base64_index(byte: u8) -> Option<u8> {
    BASE64_ALPHABET.iter().position(|&b| b == byte).map(|i| i as u8)
}

// =====================================================================
// Request / status / header line helpers
// =====================================================================

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RequestLine {
    pub method: String,
    pub path: String,
    pub protocol: String,
    pub major_version: u32,
    pub minor_version: u32,
}

impl RequestLine {
    #[must_use]
    pub fn format(&self) -> String {
        format!("{} {} {}/{}.{}\r\n", self.method, self.path, self.protocol, self.major_version, self.minor_version)
    }

    /// `cellHttpUtilParseRequestLine`. Parses `METHOD PATH PROTO/MAJ.MIN`.
    pub fn parse(line: &str) -> Result<Self, CellError> {
        let trimmed = line.trim_end_matches(['\r', '\n']);
        let mut parts = trimmed.splitn(3, ' ');
        let method = parts.next().ok_or(errors::INVALID_REQUEST)?;
        let path = parts.next().ok_or(errors::INVALID_REQUEST)?;
        let proto_ver = parts.next().ok_or(errors::INVALID_REQUEST)?;
        if method.is_empty() || path.is_empty() {
            return Err(errors::INVALID_REQUEST);
        }
        let (protocol, ver) = proto_ver.split_once('/').ok_or(errors::INVALID_REQUEST)?;
        let (maj, min) = ver.split_once('.').ok_or(errors::INVALID_REQUEST)?;
        let major = maj.parse::<u32>().map_err(|_| errors::INVALID_REQUEST)?;
        let minor = min.parse::<u32>().map_err(|_| errors::INVALID_REQUEST)?;
        Ok(Self {
            method: method.into(),
            path: path.into(),
            protocol: protocol.into(),
            major_version: major,
            minor_version: minor,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StatusLine {
    pub protocol: String,
    pub major_version: u32,
    pub minor_version: u32,
    pub status_code: i32,
    pub reason_phrase: String,
}

impl StatusLine {
    #[must_use]
    pub fn format(&self) -> String {
        format!(
            "{}/{}.{} {} {}\r\n",
            self.protocol, self.major_version, self.minor_version, self.status_code, self.reason_phrase
        )
    }

    /// `cellHttpUtilParseStatusLine`. Parses `PROTO/MAJ.MIN STATUS REASON`.
    pub fn parse(line: &str) -> Result<Self, CellError> {
        let trimmed = line.trim_end_matches(['\r', '\n']);
        let mut parts = trimmed.splitn(3, ' ');
        let proto_ver = parts.next().ok_or(errors::INVALID_RESPONSE)?;
        let status = parts.next().ok_or(errors::INVALID_RESPONSE)?;
        let reason = parts.next().unwrap_or("");
        let (protocol, ver) = proto_ver.split_once('/').ok_or(errors::INVALID_RESPONSE)?;
        let (maj, min) = ver.split_once('.').ok_or(errors::INVALID_RESPONSE)?;
        let major = maj.parse::<u32>().map_err(|_| errors::INVALID_RESPONSE)?;
        let minor = min.parse::<u32>().map_err(|_| errors::INVALID_RESPONSE)?;
        let status_code = status.parse::<i32>().map_err(|_| errors::INVALID_RESPONSE)?;
        if !(100..=599).contains(&status_code) {
            return Err(errors::INVALID_RESPONSE);
        }
        Ok(Self {
            protocol: protocol.into(),
            major_version: major,
            minor_version: minor,
            status_code,
            reason_phrase: reason.into(),
        })
    }
}

/// `cellHttpUtilParseHeader`. Parses `Name: Value`. Rejects empty name
/// and control characters.
pub fn parse_header(line: &str) -> Result<(String, String), CellError> {
    let trimmed = line.trim_end_matches(['\r', '\n']);
    let (name, value) = trimmed.split_once(':').ok_or(errors::INVALID_HEADER)?;
    if name.is_empty() || name.contains(' ') {
        return Err(errors::INVALID_HEADER);
    }
    if name.bytes().any(|b| b < 0x20 || b == 0x7F) {
        return Err(errors::INVALID_HEADER);
    }
    Ok((name.to_string(), value.trim().to_string()))
}

// =====================================================================
// Tests
// =====================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_codes_byte_exact() {
        assert_eq!(errors::NO_MEMORY.0, 0x8071_1001);
        assert_eq!(errors::NO_BUFFER.0, 0x8071_1002);
        assert_eq!(errors::NO_STRING.0, 0x8071_1003);
        assert_eq!(errors::INSUFFICIENT.0, 0x8071_1004);
        assert_eq!(errors::INVALID_URI.0, 0x8071_1005);
        assert_eq!(errors::INVALID_HEADER.0, 0x8071_1006);
        assert_eq!(errors::INVALID_REQUEST.0, 0x8071_1007);
        assert_eq!(errors::INVALID_RESPONSE.0, 0x8071_1008);
        assert_eq!(errors::INVALID_LENGTH.0, 0x8071_1009);
        assert_eq!(errors::INVALID_CHARACTER.0, 0x8071_100a);
    }

    #[test]
    fn uri_flags_stable() {
        assert_eq!(URI_FLAG_FULL_URI, 0);
        assert_eq!(URI_FLAG_NO_SCHEME, 1);
        assert_eq!(URI_FLAG_NO_CREDENTIALS, 2);
        assert_eq!(URI_FLAG_NO_PASSWORD, 4);
        assert_eq!(URI_FLAG_NO_PATH, 8);
        assert_eq!(URI_FLAG_ALL_MASK, 0xF);
    }

    #[test]
    fn uri_parse_http_default() {
        let u = Uri::parse("http://example.com").unwrap();
        assert_eq!(u.scheme, "http");
        assert_eq!(u.hostname, "example.com");
        assert_eq!(u.port, 80);
        assert_eq!(u.path, "/");
        assert!(u.username.is_empty());
    }

    #[test]
    fn uri_parse_https_with_path() {
        let u = Uri::parse("https://api.example.com/v1/users").unwrap();
        assert_eq!(u.scheme, "https");
        assert_eq!(u.port, 443);
        assert_eq!(u.path, "/v1/users");
    }

    #[test]
    fn uri_parse_with_port() {
        let u = Uri::parse("http://host:8080/api").unwrap();
        assert_eq!(u.port, 8080);
    }

    #[test]
    fn uri_parse_with_credentials() {
        let u = Uri::parse("http://alice:secret@host.com/").unwrap();
        assert_eq!(u.username, "alice");
        assert_eq!(u.password, "secret");
    }

    #[test]
    fn uri_parse_with_username_only() {
        let u = Uri::parse("ftp://user@ftp.example.com/").unwrap();
        assert_eq!(u.username, "user");
        assert_eq!(u.password, "");
        assert_eq!(u.port, 21);
    }

    #[test]
    fn uri_parse_missing_scheme_rejected() {
        assert_eq!(Uri::parse("example.com").err(), Some(errors::INVALID_URI));
    }

    #[test]
    fn uri_parse_empty_host_rejected() {
        assert_eq!(Uri::parse("http://").err(), Some(errors::INVALID_URI));
    }

    #[test]
    fn uri_parse_bad_port_rejected() {
        assert_eq!(Uri::parse("http://host:abc").err(), Some(errors::INVALID_URI));
        assert_eq!(Uri::parse("http://host:99999999").err(), Some(errors::INVALID_URI));
    }

    #[test]
    fn uri_build_full_uri() {
        let u = Uri::parse("http://user:pw@host:8080/api").unwrap();
        assert_eq!(u.build(URI_FLAG_FULL_URI).unwrap(), "http://user:pw@host:8080/api");
    }

    #[test]
    fn uri_build_no_scheme() {
        let u = Uri::parse("http://host/api").unwrap();
        let built = u.build(URI_FLAG_NO_SCHEME).unwrap();
        assert!(!built.contains("http://"));
        assert!(built.contains("host"));
    }

    #[test]
    fn uri_build_no_credentials_strips_user_and_pass() {
        let u = Uri::parse("http://user:pw@host/").unwrap();
        let built = u.build(URI_FLAG_NO_CREDENTIALS).unwrap();
        assert!(!built.contains("user"));
        assert!(!built.contains("pw"));
    }

    #[test]
    fn uri_build_no_password_keeps_user() {
        let u = Uri::parse("http://user:pw@host/").unwrap();
        let built = u.build(URI_FLAG_NO_PASSWORD).unwrap();
        assert!(built.contains("user@"));
        assert!(!built.contains(":pw"));
    }

    #[test]
    fn uri_build_no_path() {
        let u = Uri::parse("http://host/api/v1").unwrap();
        let built = u.build(URI_FLAG_NO_PATH).unwrap();
        assert!(!built.contains("/api"));
    }

    #[test]
    fn uri_build_bad_flag_rejected() {
        let u = Uri::parse("http://host/").unwrap();
        assert_eq!(u.build(0xFF).err(), Some(errors::INVALID_LENGTH));
    }

    #[test]
    fn uri_copy_requires_pool_size() {
        let u = Uri::parse("http://user:pw@host:8080/api").unwrap();
        let (cloned, req) = u.copy_into(10_000).unwrap();
        assert_eq!(cloned, u);
        assert!(req > 0);
    }

    #[test]
    fn uri_copy_insufficient_pool_rejected() {
        let u = Uri::parse("http://user:pw@host:8080/api").unwrap();
        assert_eq!(u.copy_into(1).err(), Some(errors::INSUFFICIENT));
    }

    #[test]
    fn percent_encode_rfc3986_unreserved_untouched() {
        assert_eq!(percent_encode("abcXYZ123-._~"), "abcXYZ123-._~");
    }

    #[test]
    fn percent_encode_reserved_characters_escaped() {
        assert_eq!(percent_encode(" /?&="), "%20%2F%3F%26%3D");
    }

    #[test]
    fn percent_encode_utf8_bytes() {
        assert_eq!(percent_encode("☃"), "%E2%98%83");
    }

    #[test]
    fn percent_decode_round_trip() {
        let raw = "Hello World/?&=";
        let encoded = percent_encode(raw);
        let decoded = percent_decode(&encoded).unwrap();
        assert_eq!(decoded, raw.as_bytes());
    }

    #[test]
    fn percent_decode_invalid_hex_rejected() {
        assert_eq!(percent_decode("%GG").err(), Some(errors::INVALID_CHARACTER));
    }

    #[test]
    fn percent_decode_truncated_escape_rejected() {
        assert_eq!(percent_decode("%2").err(), Some(errors::INVALID_CHARACTER));
    }

    #[test]
    fn percent_decode_control_char_rejected() {
        assert_eq!(percent_decode("a\tb").err(), Some(errors::INVALID_CHARACTER));
    }

    #[test]
    fn base64_encode_empty() {
        assert_eq!(base64_encode(&[]), "");
    }

    #[test]
    fn base64_encode_padding() {
        assert_eq!(base64_encode(b"f"), "Zg==");
        assert_eq!(base64_encode(b"fo"), "Zm8=");
        assert_eq!(base64_encode(b"foo"), "Zm9v");
        assert_eq!(base64_encode(b"foob"), "Zm9vYg==");
    }

    #[test]
    fn base64_round_trip() {
        for s in ["", "a", "ab", "abc", "abcd", "Hello, World!"] {
            let encoded = base64_encode(s.as_bytes());
            let decoded = base64_decode(&encoded).unwrap();
            assert_eq!(decoded, s.as_bytes());
        }
    }

    #[test]
    fn base64_decode_ignores_whitespace() {
        let r = base64_decode("Zm9v\nYg==").unwrap();
        assert_eq!(r, b"foob");
    }

    #[test]
    fn base64_decode_bad_length_rejected() {
        assert_eq!(base64_decode("abc").err(), Some(errors::INVALID_LENGTH));
    }

    #[test]
    fn base64_decode_invalid_char_rejected() {
        assert_eq!(base64_decode("!!!!").err(), Some(errors::INVALID_CHARACTER));
    }

    #[test]
    fn request_line_format_round_trip() {
        let r = RequestLine {
            method: "GET".into(),
            path: "/api/v1".into(),
            protocol: "HTTP".into(),
            major_version: 1,
            minor_version: 1,
        };
        assert_eq!(r.format(), "GET /api/v1 HTTP/1.1\r\n");
        let parsed = RequestLine::parse(&r.format()).unwrap();
        assert_eq!(parsed, r);
    }

    #[test]
    fn request_line_parse_malformed_rejected() {
        assert_eq!(RequestLine::parse("not valid").err(), Some(errors::INVALID_REQUEST));
        assert_eq!(RequestLine::parse("GET /").err(), Some(errors::INVALID_REQUEST));
        assert_eq!(RequestLine::parse("GET / HTTPnoslash").err(), Some(errors::INVALID_REQUEST));
    }

    #[test]
    fn status_line_format_round_trip() {
        let s = StatusLine {
            protocol: "HTTP".into(),
            major_version: 1,
            minor_version: 1,
            status_code: 200,
            reason_phrase: "OK".into(),
        };
        assert_eq!(s.format(), "HTTP/1.1 200 OK\r\n");
        let parsed = StatusLine::parse(&s.format()).unwrap();
        assert_eq!(parsed, s);
    }

    #[test]
    fn status_line_parse_out_of_range_rejected() {
        assert_eq!(StatusLine::parse("HTTP/1.1 99 Wrong\r\n").err(), Some(errors::INVALID_RESPONSE));
        assert_eq!(StatusLine::parse("HTTP/1.1 600 Wrong\r\n").err(), Some(errors::INVALID_RESPONSE));
    }

    #[test]
    fn status_line_parse_missing_reason_ok() {
        let s = StatusLine::parse("HTTP/1.1 204\r\n").unwrap();
        assert_eq!(s.status_code, 204);
        assert_eq!(s.reason_phrase, "");
    }

    #[test]
    fn parse_header_happy_path() {
        let (n, v) = parse_header("Content-Type: application/json\r\n").unwrap();
        assert_eq!(n, "Content-Type");
        assert_eq!(v, "application/json");
    }

    #[test]
    fn parse_header_empty_name_rejected() {
        assert_eq!(parse_header(": value").err(), Some(errors::INVALID_HEADER));
    }

    #[test]
    fn parse_header_space_in_name_rejected() {
        assert_eq!(parse_header("Bad Header: v").err(), Some(errors::INVALID_HEADER));
    }

    #[test]
    fn parse_header_no_colon_rejected() {
        assert_eq!(parse_header("BadHeaderValue").err(), Some(errors::INVALID_HEADER));
    }

    #[test]
    fn parse_header_control_char_rejected() {
        assert_eq!(parse_header("Header\x01Name: v").err(), Some(errors::INVALID_HEADER));
    }

    #[test]
    fn full_utilities_smoke() {
        // Parse + build + percent encode + base64 in one pipeline.
        let u = Uri::parse("https://alice:secret@api.example.com:8443/path?query=1").unwrap();
        assert_eq!(u.username, "alice");
        assert_eq!(u.hostname, "api.example.com");
        assert_eq!(u.port, 8443);
        let stripped = u.build(URI_FLAG_NO_CREDENTIALS).unwrap();
        assert!(!stripped.contains("alice"));
        let encoded = percent_encode("a b&c=d");
        assert_eq!(encoded, "a%20b%26c%3Dd");
        assert_eq!(percent_decode(&encoded).unwrap(), b"a b&c=d");
        let b64 = base64_encode(b"credentials");
        assert_eq!(base64_decode(&b64).unwrap(), b"credentials");
        let req = RequestLine {
            method: "GET".into(),
            path: "/x".into(),
            protocol: "HTTP".into(),
            major_version: 1,
            minor_version: 0,
        };
        assert!(req.format().contains("GET /x HTTP/1.0"));
    }
}
