//! `rpcs3-hle-sys-net-user` — PS3 BSD-sockets user-mode HLE.
//!
//! Ports `rpcs3/Emu/Cell/Modules/sys_net_.cpp` (716 linhas).  The
//! module exposes ~100 entry points covering BSD sockets (accept,
//! bind, connect, listen, recv/send, …), inet address helpers, a
//! separate `netset_*` sub-API, the LV2 allocator callbacks
//! (`_sys_net_lib_*`), and two errno-location helpers that poke the
//! guest TLS block.  In RPCS3 most of these are stubs; the observable
//! contract this crate captures is:
//!
//! * `sys_net_inet_addr` returns `INET_ADDR_NONE = 0xFFFFFFFF` for
//!   every input (cpp:61-66).
//! * `_sys_net_errno_loc` / `_sys_net_h_errno_loc` compute the guest
//!   pointer `gpr[13] - 0x7030 + {0x2c, 0x28}` respectively (cpp:291-297
//!   / cpp:368-374).
//! * The 100+ entry points are registered under `sys_net`; the crate
//!   exposes an audit registry so higher layers can cross-check against
//!   real PS3 binaries.

extern crate alloc;

use rpcs3_emu_types::CellError;

// =====================================================================
// Error codes
// =====================================================================

pub const CELL_EINVAL: CellError = CellError(0x8001_0002);

// =====================================================================
// Constants — byte-exact with sys_net_.cpp
// =====================================================================

/// `inet_addr` sentinel return — cpp:65 `return 0xffffffff`.  Mirrors
/// the classic BSD `INADDR_NONE`.
pub const INET_ADDR_NONE: u32 = 0xFFFF_FFFF;

/// Offset subtracted from `gpr[13]` in both errno locators
/// (cpp:296 / cpp:373 `gpr[13] - 0x7030`).
pub const TLS_BASE_OFFSET: u32 = 0x7030;

/// `errno` offset within the TLS system-area block — cpp:296
/// `+ 0x2c`.
pub const ERRNO_TLS_OFFSET: u32 = 0x2C;

/// `h_errno` offset within the TLS system-area block — cpp:373
/// `+ 0x28`.
pub const H_ERRNO_TLS_OFFSET: u32 = 0x28;

// =====================================================================
// Errno locators — byte-exact formulas
// =====================================================================

/// Port of `_sys_net_errno_loc` (cpp:291-297).  Returns the guest
/// address of the per-thread `errno` slot given the thread's `gpr[13]`
/// value.  Formula: `gpr[13] - 0x7030 + 0x2c`.
#[must_use]
pub fn errno_loc(gpr13: u32) -> u32 {
    gpr13.wrapping_sub(TLS_BASE_OFFSET).wrapping_add(ERRNO_TLS_OFFSET)
}

/// Port of `_sys_net_h_errno_loc` (cpp:368-374).  Formula:
/// `gpr[13] - 0x7030 + 0x28`.
#[must_use]
pub fn h_errno_loc(gpr13: u32) -> u32 {
    gpr13.wrapping_sub(TLS_BASE_OFFSET).wrapping_add(H_ERRNO_TLS_OFFSET)
}

// =====================================================================
// inet helpers — observable behaviour of the RPCS3 stubs
// =====================================================================

/// Port of `sys_net_inet_addr` — the firmware stub unconditionally
/// returns [`INET_ADDR_NONE`] regardless of the input string
/// (cpp:61-66).  Preserved for byte-exact fidelity.
#[must_use]
pub fn inet_addr_stub(_cp_valid: bool) -> u32 { INET_ADDR_NONE }

/// Real `inet_addr` parser — not what the C++ stub does, but what
/// real PS3 libnet would return.  Expose separately so higher layers
/// can swap in genuine behaviour.  Returns [`INET_ADDR_NONE`] on parse
/// failure.
///
/// Accepts only dotted-quad `"a.b.c.d"` with each component in `0..=255`.
#[must_use]
pub fn inet_addr_parse(cp: &str) -> u32 {
    let mut out = 0u32;
    let mut count = 0;
    for part in cp.split('.') {
        if count >= 4 { return INET_ADDR_NONE; }
        let Ok(n) = part.parse::<u32>() else { return INET_ADDR_NONE };
        if n > 255 { return INET_ADDR_NONE; }
        out = (out << 8) | n;
        count += 1;
    }
    if count != 4 { return INET_ADDR_NONE; }
    // `inet_addr` returns network-byte-order big-endian; but callers
    // on the PS3 interpret the u32 directly as the packed address.
    out.to_be()
}

// =====================================================================
// Socket FSM
// =====================================================================

/// Socket state — covers the observable life-cycle a BSD socket
/// transitions through.  The PS3 surface doesn't enforce these
/// directly (the stub returns 0 everywhere), but higher layers can
/// use this to drive a real network backend.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SocketState {
    Created,
    Bound,
    Listening,
    Connected,
    Closed,
}

/// Minimal mirror of `sys_net_sockaddr` — only the fields the
/// firmware consults in the user-mode layer.  Real PS3 layout has
/// 16 bytes total (sin_family + sin_port + sin_addr + sin_zero).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SockAddr {
    pub family: u16,
    pub port: u16,
    pub addr: u32,
}

/// Tracks the life-cycle of a single BSD socket.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Socket {
    pub fd: i32,
    pub family: i32,
    pub sock_type: i32,
    pub protocol: i32,
    pub state: SocketState,
    pub local: SockAddr,
    pub peer: SockAddr,
    pub listen_backlog: i32,
}

impl Socket {
    #[must_use]
    pub fn new(fd: i32, family: i32, sock_type: i32, protocol: i32) -> Self {
        Self {
            fd, family, sock_type, protocol,
            state: SocketState::Created,
            local: SockAddr::default(),
            peer: SockAddr::default(),
            listen_backlog: 0,
        }
    }
}

/// Manager for the PS3 socket table.
#[derive(Debug, Default, Clone)]
pub struct NetUser {
    sockets: alloc::vec::Vec<Socket>,
    next_fd: i32,
}

impl NetUser {
    #[must_use]
    pub fn new() -> Self { Self { sockets: alloc::vec::Vec::new(), next_fd: 3 } }

    /// Port of `sys_net_socket`.  Returns a new fd on success; the
    /// C++ stub just returns 0, but higher layers need unique fds.
    pub fn socket(&mut self, family: i32, sock_type: i32, protocol: i32) -> i32 {
        let fd = self.next_fd;
        self.next_fd = self.next_fd.checked_add(1).unwrap_or(i32::MAX);
        self.sockets.push(Socket::new(fd, family, sock_type, protocol));
        fd
    }

    /// Port of `sys_net_bind`.
    ///
    /// # Errors
    /// * [`CELL_EINVAL`] if the fd is unknown.
    pub fn bind(&mut self, fd: i32, addr: SockAddr) -> Result<(), CellError> {
        let s = self.sockets.iter_mut().find(|s| s.fd == fd)
            .ok_or(CELL_EINVAL)?;
        s.local = addr;
        s.state = SocketState::Bound;
        Ok(())
    }

    /// Port of `sys_net_listen`.
    ///
    /// # Errors
    /// * [`CELL_EINVAL`] if the fd is unknown or the socket isn't
    ///   bound.
    pub fn listen(&mut self, fd: i32, backlog: i32) -> Result<(), CellError> {
        let s = self.sockets.iter_mut().find(|s| s.fd == fd)
            .ok_or(CELL_EINVAL)?;
        if s.state != SocketState::Bound { return Err(CELL_EINVAL); }
        s.state = SocketState::Listening;
        s.listen_backlog = backlog;
        Ok(())
    }

    /// Port of `sys_net_connect`.  Updates `peer` and transitions to
    /// `Connected`.
    ///
    /// # Errors
    /// * [`CELL_EINVAL`] if the fd is unknown or the socket is in the
    ///   wrong state (`Listening` / `Closed`).
    pub fn connect(&mut self, fd: i32, addr: SockAddr) -> Result<(), CellError> {
        let s = self.sockets.iter_mut().find(|s| s.fd == fd)
            .ok_or(CELL_EINVAL)?;
        if matches!(s.state, SocketState::Listening | SocketState::Closed) {
            return Err(CELL_EINVAL);
        }
        s.peer = addr;
        s.state = SocketState::Connected;
        Ok(())
    }

    /// Port of `sys_net_shutdown` / `sys_net_socketclose`.
    pub fn close(&mut self, fd: i32) -> Result<(), CellError> {
        let s = self.sockets.iter_mut().find(|s| s.fd == fd)
            .ok_or(CELL_EINVAL)?;
        s.state = SocketState::Closed;
        Ok(())
    }

    #[must_use]
    pub fn get(&self, fd: i32) -> Option<&Socket> {
        self.sockets.iter().find(|s| s.fd == fd)
    }

    #[must_use]
    pub fn len(&self) -> usize { self.sockets.len() }

    #[must_use]
    pub fn is_empty(&self) -> bool { self.sockets.is_empty() }
}

// =====================================================================
// Stub registry — the large list of registered entry points
// =====================================================================

/// All entry points `sys_net` registers with `REG_FNID` or `REG_FUNC`
/// in cpp:610-714.  Ordered to match the C++ source so cross-checks
/// against the PS3 binary's export table stay stable.
///
/// Covers: 30 BSD FNIDs + 28 `sys_net_*`/`_sys_net_*` utility funcs + 22
/// `_sys_net_lib_*` LV2 callbacks + 8 `sys_netset_*` + 7 `_sce_net_*`
/// = **95** entries.
pub const REGISTERED_ENTRY_POINTS: &[&str] = &[
    // 30 BSD-style FNIDs (cpp:612-641).
    "accept", "bind", "connect", "gethostbyaddr", "gethostbyname",
    "getpeername", "getsockname", "getsockopt", "inet_addr",
    "inet_aton", "inet_lnaof", "inet_makeaddr", "inet_netof",
    "inet_network", "inet_ntoa", "inet_ntop", "inet_pton",
    "listen", "recv", "recvfrom", "recvmsg", "send", "sendmsg",
    "sendto", "setsockopt", "shutdown", "socket", "socketclose",
    "socketpoll", "socketselect",
    // 28 sys_net/_sys_net utility funcs (cpp:643-670).
    "sys_net_initialize_network_ex", "sys_net_get_udpp2p_test_param",
    "sys_net_set_udpp2p_test_param", "sys_net_get_lib_name_server",
    "sys_net_if_ctl", "sys_net_get_if_list", "sys_net_get_name_server",
    "sys_net_get_netemu_test_param", "sys_net_get_routing_table_af",
    "sys_net_get_sockinfo", "sys_net_close_dump",
    "sys_net_set_test_param", "sys_net_show_nameserver",
    "_sys_net_errno_loc", "sys_net_set_resolver_configurations",
    "sys_net_show_route", "sys_net_read_dump",
    "sys_net_abort_resolver", "sys_net_abort_socket",
    "sys_net_set_lib_name_server", "sys_net_get_test_param",
    "sys_net_get_sockinfo_ex", "sys_net_open_dump",
    "sys_net_show_ifconfig", "sys_net_finalize_network",
    "_sys_net_h_errno_loc", "sys_net_set_netemu_test_param",
    "sys_net_free_thread_context",
    // 22 _sys_net_lib_* LV2 callbacks (cpp:672-696).
    "_sys_net_lib_abort", "_sys_net_lib_bnet_control",
    "__sys_net_lib_calloc", "_sys_net_lib_free",
    "_sys_net_lib_get_system_time", "_sys_net_lib_if_nametoindex",
    "_sys_net_lib_ioctl", "__sys_net_lib_malloc",
    "_sys_net_lib_rand", "__sys_net_lib_realloc",
    "_sys_net_lib_reset_libnetctl_queue",
    "_sys_net_lib_set_libnetctl_queue",
    "_sys_net_lib_thread_create", "_sys_net_lib_thread_exit",
    "_sys_net_lib_thread_join", "_sys_net_lib_sync_clear",
    "_sys_net_lib_sync_create", "_sys_net_lib_sync_destroy",
    "_sys_net_lib_sync_signal", "_sys_net_lib_sync_wait",
    "_sys_net_lib_sysctl", "_sys_net_lib_usleep",
    // 8 sys_netset_* (cpp:698-705).
    "sys_netset_abort", "sys_netset_close", "sys_netset_get_if_id",
    "sys_netset_get_key_value", "sys_netset_get_status",
    "sys_netset_if_down", "sys_netset_if_up", "sys_netset_open",
    // 7 _sce_net_* (cpp:707-714).
    "_sce_net_add_name_server", "_sce_net_add_name_server_with_char",
    "_sce_net_flush_route", "_sce_net_get_name_server",
    "_sce_net_set_default_gateway", "_sce_net_set_ip_and_mask",
    "_sce_net_set_name_server",
];

/// Returns `true` if `name` is one of the registered entry points.
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
    fn inet_addr_none_byte_exact() {
        // cpp:65 `return 0xffffffff`.
        assert_eq!(INET_ADDR_NONE, 0xFFFF_FFFF);
    }

    #[test]
    fn tls_offsets_byte_exact() {
        assert_eq!(TLS_BASE_OFFSET, 0x7030);
        assert_eq!(ERRNO_TLS_OFFSET, 0x2C);
        assert_eq!(H_ERRNO_TLS_OFFSET, 0x28);
    }

    #[test]
    fn cell_einval_byte_exact() {
        assert_eq!(CELL_EINVAL.0, 0x8001_0002);
    }

    // ---- errno locators --------------------------------------------

    #[test]
    fn errno_loc_formula() {
        // gpr[13] = 0x0100_0000 → 0x0100_0000 - 0x7030 + 0x2c = 0x00FF_8FFC
        assert_eq!(errno_loc(0x0100_0000), 0x0100_0000 - 0x7030 + 0x2C);
    }

    #[test]
    fn h_errno_loc_formula() {
        assert_eq!(h_errno_loc(0x0100_0000), 0x0100_0000 - 0x7030 + 0x28);
    }

    #[test]
    fn errno_and_h_errno_differ_by_four() {
        // 0x2c - 0x28 = 4
        assert_eq!(errno_loc(0x5000_0000) - h_errno_loc(0x5000_0000), 4);
    }

    #[test]
    fn errno_loc_wraps_below_gpr13() {
        // Small gpr[13] should still compute (via wrapping).
        let e = errno_loc(0x1000);
        let expected = 0x1000_u32.wrapping_sub(0x7030).wrapping_add(0x2C);
        assert_eq!(e, expected);
    }

    // ---- inet_addr --------------------------------------------------

    #[test]
    fn inet_addr_stub_always_returns_none() {
        assert_eq!(inet_addr_stub(true), INET_ADDR_NONE);
        assert_eq!(inet_addr_stub(false), INET_ADDR_NONE);
    }

    #[test]
    fn inet_addr_parse_dotted_quad() {
        // 127.0.0.1 = 0x7F000001 host-order, network = 0x0100007F
        assert_eq!(inet_addr_parse("127.0.0.1"), 0x0100_007F);
    }

    #[test]
    fn inet_addr_parse_zero_zero_zero_zero() {
        assert_eq!(inet_addr_parse("0.0.0.0"), 0);
    }

    #[test]
    fn inet_addr_parse_all_ones() {
        // 255.255.255.255 = INET_ADDR_NONE
        assert_eq!(inet_addr_parse("255.255.255.255"), 0xFFFF_FFFF);
    }

    #[test]
    fn inet_addr_parse_rejects_overflow() {
        assert_eq!(inet_addr_parse("256.0.0.0"), INET_ADDR_NONE);
        assert_eq!(inet_addr_parse("0.300.0.0"), INET_ADDR_NONE);
    }

    #[test]
    fn inet_addr_parse_rejects_short() {
        assert_eq!(inet_addr_parse("1.2.3"), INET_ADDR_NONE);
        assert_eq!(inet_addr_parse("1.2"),   INET_ADDR_NONE);
        assert_eq!(inet_addr_parse(""),      INET_ADDR_NONE);
    }

    #[test]
    fn inet_addr_parse_rejects_long() {
        assert_eq!(inet_addr_parse("1.2.3.4.5"), INET_ADDR_NONE);
    }

    #[test]
    fn inet_addr_parse_rejects_non_decimal() {
        assert_eq!(inet_addr_parse("1.a.3.4"), INET_ADDR_NONE);
        assert_eq!(inet_addr_parse("1.2.3.-1"), INET_ADDR_NONE);
    }

    // ---- Socket FSM --------------------------------------------------

    #[test]
    fn socket_starts_in_created_state() {
        let mut n = NetUser::new();
        let fd = n.socket(2, 1, 0); // AF_INET, SOCK_STREAM, any
        assert_eq!(n.get(fd).unwrap().state, SocketState::Created);
    }

    #[test]
    fn socket_fds_start_at_three() {
        let mut n = NetUser::new();
        let fd = n.socket(2, 1, 0);
        assert_eq!(fd, 3); // stdin/stdout/stderr = 0/1/2
    }

    #[test]
    fn socket_fds_are_monotonic() {
        let mut n = NetUser::new();
        let a = n.socket(2, 1, 0);
        let b = n.socket(2, 1, 0);
        let c = n.socket(2, 1, 0);
        assert_eq!((a, b, c), (3, 4, 5));
    }

    #[test]
    fn bind_transitions_to_bound() {
        let mut n = NetUser::new();
        let fd = n.socket(2, 1, 0);
        let addr = SockAddr { family: 2, port: 8080, addr: 0x7F00_0001 };
        n.bind(fd, addr).unwrap();
        assert_eq!(n.get(fd).unwrap().state, SocketState::Bound);
        assert_eq!(n.get(fd).unwrap().local, addr);
    }

    #[test]
    fn bind_unknown_fd_is_einval() {
        let mut n = NetUser::new();
        assert_eq!(
            n.bind(99, SockAddr::default()).unwrap_err(),
            CELL_EINVAL,
        );
    }

    #[test]
    fn listen_requires_bound() {
        let mut n = NetUser::new();
        let fd = n.socket(2, 1, 0);
        assert_eq!(n.listen(fd, 10).unwrap_err(), CELL_EINVAL);
    }

    #[test]
    fn listen_transitions_to_listening() {
        let mut n = NetUser::new();
        let fd = n.socket(2, 1, 0);
        n.bind(fd, SockAddr::default()).unwrap();
        n.listen(fd, 5).unwrap();
        let s = n.get(fd).unwrap();
        assert_eq!(s.state, SocketState::Listening);
        assert_eq!(s.listen_backlog, 5);
    }

    #[test]
    fn connect_transitions_to_connected() {
        let mut n = NetUser::new();
        let fd = n.socket(2, 1, 0);
        let peer = SockAddr { family: 2, port: 443, addr: 0x0101_0101 };
        n.connect(fd, peer).unwrap();
        let s = n.get(fd).unwrap();
        assert_eq!(s.state, SocketState::Connected);
        assert_eq!(s.peer, peer);
    }

    #[test]
    fn connect_rejects_listening_socket() {
        let mut n = NetUser::new();
        let fd = n.socket(2, 1, 0);
        n.bind(fd, SockAddr::default()).unwrap();
        n.listen(fd, 1).unwrap();
        assert_eq!(
            n.connect(fd, SockAddr::default()).unwrap_err(),
            CELL_EINVAL,
        );
    }

    #[test]
    fn close_transitions_to_closed() {
        let mut n = NetUser::new();
        let fd = n.socket(2, 1, 0);
        n.close(fd).unwrap();
        assert_eq!(n.get(fd).unwrap().state, SocketState::Closed);
    }

    #[test]
    fn connect_rejects_closed_socket() {
        let mut n = NetUser::new();
        let fd = n.socket(2, 1, 0);
        n.close(fd).unwrap();
        assert_eq!(
            n.connect(fd, SockAddr::default()).unwrap_err(),
            CELL_EINVAL,
        );
    }

    #[test]
    fn close_unknown_fd_is_einval() {
        let mut n = NetUser::new();
        assert_eq!(n.close(99).unwrap_err(), CELL_EINVAL);
    }

    // ---- registry ---------------------------------------------------

    #[test]
    fn registry_has_95_entries() {
        // 30 BSD + 28 utility + 22 LV2 callbacks + 8 netset + 7 sce = 95
        assert_eq!(REGISTERED_ENTRY_POINTS.len(), 95);
    }

    #[test]
    fn registry_contains_all_bsd() {
        for name in ["accept", "bind", "connect", "listen", "recv",
                     "send", "socket", "socketclose", "socketpoll",
                     "socketselect"] {
            assert!(is_registered(name), "{name}");
        }
    }

    #[test]
    fn registry_contains_inet_helpers() {
        for name in ["inet_addr", "inet_aton", "inet_ntoa", "inet_ntop",
                     "inet_pton", "inet_network"] {
            assert!(is_registered(name), "{name}");
        }
    }

    #[test]
    fn registry_contains_errno_locators() {
        assert!(is_registered("_sys_net_errno_loc"));
        assert!(is_registered("_sys_net_h_errno_loc"));
    }

    #[test]
    fn registry_contains_netset_family() {
        for name in ["sys_netset_abort", "sys_netset_close",
                     "sys_netset_get_if_id", "sys_netset_get_key_value",
                     "sys_netset_get_status", "sys_netset_if_down",
                     "sys_netset_if_up", "sys_netset_open"] {
            assert!(is_registered(name), "{name}");
        }
    }

    #[test]
    fn registry_contains_lib_callbacks() {
        for name in ["_sys_net_lib_abort", "__sys_net_lib_malloc",
                     "__sys_net_lib_calloc", "__sys_net_lib_realloc",
                     "_sys_net_lib_free", "_sys_net_lib_rand",
                     "_sys_net_lib_thread_create",
                     "_sys_net_lib_sync_create"] {
            assert!(is_registered(name), "{name}");
        }
    }

    #[test]
    fn registry_rejects_unknown() {
        assert!(!is_registered("sys_net_nonexistent"));
        assert!(!is_registered(""));
        // Case-sensitive.
        assert!(!is_registered("BIND"));
    }

    #[test]
    fn registry_has_no_duplicates() {
        let mut sorted: alloc::vec::Vec<&&str> = REGISTERED_ENTRY_POINTS.iter().collect();
        sorted.sort();
        for pair in sorted.windows(2) {
            assert_ne!(pair[0], pair[1], "duplicate: {:?}", pair[0]);
        }
    }

    // ---- full smoke -------------------------------------------------

    #[test]
    fn full_net_lifecycle_smoke() {
        let mut n = NetUser::new();

        // 1. Game calls socket(AF_INET, SOCK_STREAM, 0).
        let server_fd = n.socket(2, 1, 0);
        let client_fd = n.socket(2, 1, 0);
        assert_eq!((server_fd, client_fd), (3, 4));

        // 2. Server binds + listens.
        let server_addr = SockAddr { family: 2, port: 8080, addr: 0 };
        n.bind(server_fd, server_addr).unwrap();
        n.listen(server_fd, 5).unwrap();
        assert_eq!(n.get(server_fd).unwrap().state, SocketState::Listening);

        // 3. Client connects.
        let peer = SockAddr { family: 2, port: 8080, addr: 0x7F00_0001 };
        n.connect(client_fd, peer).unwrap();
        assert_eq!(n.get(client_fd).unwrap().state, SocketState::Connected);

        // 4. Parse an IPv4 address.
        let addr = inet_addr_parse("192.168.1.1");
        assert_ne!(addr, INET_ADDR_NONE);

        // 5. inet_addr stub still returns none (byte-exact firmware).
        assert_eq!(inet_addr_stub(true), INET_ADDR_NONE);

        // 6. Errno locator computation.
        let gpr13 = 0x1000_7030_u32;
        assert_eq!(errno_loc(gpr13), 0x1000_0000 + 0x2C);
        assert_eq!(h_errno_loc(gpr13), 0x1000_0000 + 0x28);

        // 7. Close both sockets.
        n.close(server_fd).unwrap();
        n.close(client_fd).unwrap();

        // 8. Further connect on closed socket fails.
        assert_eq!(n.connect(server_fd, peer).unwrap_err(), CELL_EINVAL);
    }
}
