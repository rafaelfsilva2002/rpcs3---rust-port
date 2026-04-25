//! `rpcs3-hle-celldmux` — generic demuxer framework HLE.
//!
//! Ports `rpcs3/Emu/Cell/Modules/cellDmux.cpp`. cellDmux is the API
//! games use to split a multiplexed container (PAMF / MP4 / AVI) into
//! per-stream elementary streams for cellAdec / cellVdec. The HLE
//! surface is `QueryAttr → Open → EnableEs → ResetStream → Release`.
//!
//! ## Entry points covered
//!
//! | HLE function                | Rust wrapper                         |
//! |-----------------------------|--------------------------------------|
//! | `cellDmuxQueryAttr`         | [`query_attr`]                       |
//! | `cellDmuxOpen`              | [`Dmux::open`]                       |
//! | `cellDmuxClose`             | [`Dmux::close`]                      |
//! | `cellDmuxResetStream`       | [`Dmux::reset_stream`]               |
//! | `cellDmuxEnableEs`          | [`Dmux::enable_es`]                  |
//! | `cellDmuxDisableEs`         | [`Dmux::disable_es`]                 |
//! | `cellDmuxResetEs`           | [`Dmux::reset_es`]                   |
//! | `cellDmuxReleaseAu`         | [`Dmux::release_au`]                 |
//! | `cellDmuxSetStream`         | [`Dmux::set_stream`]                 |

use rpcs3_emu_types::CellError;

// =====================================================================
// Error codes — byte-exact with cellDmux.h:8-15
// =====================================================================

pub mod errors {
    use rpcs3_emu_types::CellError;

    pub const ARG: CellError = CellError(0x8061_0201);
    pub const SEQ: CellError = CellError(0x8061_0202);
    pub const BUSY: CellError = CellError(0x8061_0203);
    pub const EMPTY: CellError = CellError(0x8061_0204);
    pub const FATAL: CellError = CellError(0x8061_0205);
}

// =====================================================================
// Stream type / message type (cellDmux.h:17-39)
// =====================================================================

pub const STREAM_TYPE_UNDEF: i32 = 0;
pub const STREAM_TYPE_PAMF: i32 = 1;
pub const STREAM_TYPE_TERMINATOR: i32 = 2;
/// cellSail-only: MP4 container.
pub const STREAM_TYPE_MP4: i32 = 0x81;
/// cellSail-only: AVI container.
pub const STREAM_TYPE_AVI: i32 = 0x82;

#[must_use]
pub fn is_known_stream_type(t: i32) -> bool {
    matches!(
        t,
        STREAM_TYPE_UNDEF | STREAM_TYPE_PAMF | STREAM_TYPE_TERMINATOR | STREAM_TYPE_MP4 | STREAM_TYPE_AVI
    )
}

pub const MSG_TYPE_DEMUX_DONE: i32 = 0;
pub const MSG_TYPE_FATAL_ERR: i32 = 1;
pub const MSG_TYPE_PROG_END_CODE: i32 = 2;

pub const ES_MSG_TYPE_AU_FOUND: i32 = 0;
pub const ES_MSG_TYPE_FLUSH_DONE: i32 = 1;

// =====================================================================
// Attribute + limits
// =====================================================================

pub const MAX_ES_PER_DMUX: usize = 16;
pub const MAX_HANDLES: u32 = 64;
/// Elementary-stream filter id is a single byte (matches PAMF `stream_id`).
pub const ES_FILTER_ID_MAX: u32 = 0xFF;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Attr {
    pub mem_size: u32,
    pub demux_ver: u32,
    pub pamf_ver: u32,
}

/// `cellDmuxQueryAttr(type, attr)`. Returns the memory footprint.
pub fn query_attr(stream_type: i32) -> Result<Attr, CellError> {
    if !is_known_stream_type(stream_type) {
        return Err(errors::ARG);
    }
    if stream_type == STREAM_TYPE_UNDEF || stream_type == STREAM_TYPE_TERMINATOR {
        return Err(errors::ARG);
    }
    // PAMF needs ~512 KB scratch; MP4/AVI roughly 2x to buffer moov atoms.
    let mem_size = match stream_type {
        STREAM_TYPE_PAMF => 512 * 1024,
        STREAM_TYPE_MP4 | STREAM_TYPE_AVI => 1024 * 1024,
        _ => return Err(errors::ARG),
    };
    Ok(Attr { mem_size, demux_ver: 0x0101_0000, pamf_ver: 0x0100_0000 })
}

// =====================================================================
// Domain types
// =====================================================================

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct StreamRange {
    pub stream_addr: u32,
    pub stream_size: u32,
    pub continuity: bool,
    pub user_data: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EsFilterId {
    pub stream_id: u8,
    pub private_stream_id: u8,
    pub supplemental_info1: u32,
    pub supplemental_info2: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EsResource {
    pub mem_addr: u32,
    pub mem_size: u32,
    pub mem_alignment: u32,
}

impl EsResource {
    fn validate(&self) -> Result<(), CellError> {
        if self.mem_addr == 0 {
            return Err(errors::ARG);
        }
        if self.mem_size == 0 {
            return Err(errors::ARG);
        }
        if self.mem_alignment == 0 || !self.mem_alignment.is_power_of_two() {
            return Err(errors::ARG);
        }
        if self.mem_addr % self.mem_alignment != 0 {
            return Err(errors::ARG);
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ElementaryStream {
    pub id: u32,
    pub filter: EsFilterId,
    pub resource: EsResource,
    pub state: EsState,
    pub pending_aus: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EsState {
    Idle,
    Enabled,
    Flushing,
}

// =====================================================================
// Dmux handle
// =====================================================================

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DmuxState {
    Idle,
    StreamSet,
}

#[derive(Clone, Debug)]
pub struct Handle {
    pub id: u32,
    pub stream_type: i32,
    pub state: DmuxState,
    pub stream: StreamRange,
    pub elementary_streams: Vec<ElementaryStream>,
    pub next_es_id: u32,
}

#[derive(Clone, Debug, Default)]
pub struct Dmux {
    handles: Vec<Handle>,
    next_id: u32,
}

impl Dmux {
    #[must_use]
    pub fn new() -> Self {
        Self { next_id: 1, ..Default::default() }
    }

    /// `cellDmuxOpen(type, resource, callback, handle_out)`.
    pub fn open(&mut self, stream_type: i32) -> Result<u32, CellError> {
        if !is_known_stream_type(stream_type) || stream_type == STREAM_TYPE_UNDEF || stream_type == STREAM_TYPE_TERMINATOR {
            return Err(errors::ARG);
        }
        if self.handles.len() >= MAX_HANDLES as usize {
            return Err(errors::FATAL);
        }
        let id = self.next_id;
        self.next_id = self.next_id.checked_add(1).ok_or(errors::FATAL)?;
        self.handles.push(Handle {
            id,
            stream_type,
            state: DmuxState::Idle,
            stream: StreamRange::default(),
            elementary_streams: Vec::new(),
            next_es_id: 1,
        });
        Ok(id)
    }

    pub fn close(&mut self, id: u32) -> Result<(), CellError> {
        let idx = self.handle_idx(id)?;
        if self.handles[idx].elementary_streams.iter().any(|e| e.state != EsState::Idle) {
            return Err(errors::BUSY);
        }
        self.handles.remove(idx);
        Ok(())
    }

    /// `cellDmuxSetStream(handle, stream_addr, stream_size, continuity, user_data)`.
    pub fn set_stream(&mut self, id: u32, stream: StreamRange) -> Result<(), CellError> {
        let idx = self.handle_idx(id)?;
        if stream.stream_addr == 0 || stream.stream_size == 0 {
            return Err(errors::ARG);
        }
        self.handles[idx].stream = stream;
        self.handles[idx].state = DmuxState::StreamSet;
        Ok(())
    }

    pub fn reset_stream(&mut self, id: u32) -> Result<(), CellError> {
        let idx = self.handle_idx(id)?;
        self.handles[idx].state = DmuxState::Idle;
        self.handles[idx].stream = StreamRange::default();
        // Running ESes transition back to Idle so the game can re-enable them.
        for es in &mut self.handles[idx].elementary_streams {
            es.state = EsState::Idle;
            es.pending_aus = 0;
        }
        Ok(())
    }

    /// `cellDmuxEnableEs(handle, filter, resource, cb, out_es)`.
    pub fn enable_es(
        &mut self,
        id: u32,
        filter: EsFilterId,
        resource: EsResource,
    ) -> Result<u32, CellError> {
        let idx = self.handle_idx(id)?;
        resource.validate()?;
        if self.handles[idx].elementary_streams.len() >= MAX_ES_PER_DMUX {
            return Err(errors::FATAL);
        }
        // Filter_id collisions: only one ES may claim a given (stream_id, private_stream_id) pair.
        if self.handles[idx].elementary_streams.iter().any(|e| {
            e.filter.stream_id == filter.stream_id && e.filter.private_stream_id == filter.private_stream_id
        }) {
            return Err(errors::BUSY);
        }
        let es_id = self.handles[idx].next_es_id;
        self.handles[idx].next_es_id = self.handles[idx].next_es_id.checked_add(1).ok_or(errors::FATAL)?;
        self.handles[idx].elementary_streams.push(ElementaryStream {
            id: es_id,
            filter,
            resource,
            state: EsState::Enabled,
            pending_aus: 0,
        });
        Ok(es_id)
    }

    pub fn disable_es(&mut self, id: u32, es_id: u32) -> Result<(), CellError> {
        let idx = self.handle_idx(id)?;
        let es_idx = self.es_idx(idx, es_id)?;
        // If the game hasn't released all AUs, the real lib returns BUSY.
        if self.handles[idx].elementary_streams[es_idx].pending_aus > 0 {
            return Err(errors::BUSY);
        }
        self.handles[idx].elementary_streams.remove(es_idx);
        Ok(())
    }

    pub fn reset_es(&mut self, id: u32, es_id: u32) -> Result<(), CellError> {
        let idx = self.handle_idx(id)?;
        let es_idx = self.es_idx(idx, es_id)?;
        self.handles[idx].elementary_streams[es_idx].state = EsState::Flushing;
        self.handles[idx].elementary_streams[es_idx].pending_aus = 0;
        Ok(())
    }

    /// Test hook: signal that the game got an AU_FOUND event for an ES.
    pub fn inject_au(&mut self, id: u32, es_id: u32) -> Result<(), CellError> {
        let idx = self.handle_idx(id)?;
        let es_idx = self.es_idx(idx, es_id)?;
        if self.handles[idx].elementary_streams[es_idx].state != EsState::Enabled {
            return Err(errors::SEQ);
        }
        self.handles[idx].elementary_streams[es_idx].pending_aus =
            self.handles[idx].elementary_streams[es_idx]
                .pending_aus
                .checked_add(1)
                .ok_or(errors::FATAL)?;
        Ok(())
    }

    /// `cellDmuxReleaseAu(es)`. Consumes one pending AU; EMPTY if none.
    pub fn release_au(&mut self, id: u32, es_id: u32) -> Result<(), CellError> {
        let idx = self.handle_idx(id)?;
        let es_idx = self.es_idx(idx, es_id)?;
        let es = &mut self.handles[idx].elementary_streams[es_idx];
        if es.pending_aus == 0 {
            return Err(errors::EMPTY);
        }
        es.pending_aus -= 1;
        Ok(())
    }

    pub fn es_state(&self, id: u32, es_id: u32) -> Result<EsState, CellError> {
        let idx = self.handle_idx(id)?;
        let es_idx = self.es_idx(idx, es_id)?;
        Ok(self.handles[idx].elementary_streams[es_idx].state)
    }

    pub fn pending_aus(&self, id: u32, es_id: u32) -> Result<u32, CellError> {
        let idx = self.handle_idx(id)?;
        let es_idx = self.es_idx(idx, es_id)?;
        Ok(self.handles[idx].elementary_streams[es_idx].pending_aus)
    }

    #[must_use]
    pub fn handle_count(&self) -> usize {
        self.handles.len()
    }

    fn handle_idx(&self, id: u32) -> Result<usize, CellError> {
        self.handles.iter().position(|h| h.id == id).ok_or(errors::ARG)
    }

    fn es_idx(&self, handle_idx: usize, es_id: u32) -> Result<usize, CellError> {
        self.handles[handle_idx]
            .elementary_streams
            .iter()
            .position(|e| e.id == es_id)
            .ok_or(errors::ARG)
    }
}

// =====================================================================
// Tests
// =====================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn ok_filter() -> EsFilterId {
        EsFilterId { stream_id: 0xE0, private_stream_id: 0, supplemental_info1: 0, supplemental_info2: 0 }
    }

    fn ok_resource() -> EsResource {
        EsResource { mem_addr: 0x1_0000, mem_size: 64 * 1024, mem_alignment: 0x80 }
    }

    fn ok_stream() -> StreamRange {
        StreamRange { stream_addr: 0x2_0000, stream_size: 1024 * 1024, continuity: true, user_data: 0 }
    }

    #[test]
    fn error_codes_byte_exact() {
        assert_eq!(errors::ARG.0, 0x8061_0201);
        assert_eq!(errors::SEQ.0, 0x8061_0202);
        assert_eq!(errors::BUSY.0, 0x8061_0203);
        assert_eq!(errors::EMPTY.0, 0x8061_0204);
        assert_eq!(errors::FATAL.0, 0x8061_0205);
    }

    #[test]
    fn stream_type_constants_stable() {
        assert_eq!(STREAM_TYPE_UNDEF, 0);
        assert_eq!(STREAM_TYPE_PAMF, 1);
        assert_eq!(STREAM_TYPE_TERMINATOR, 2);
        assert_eq!(STREAM_TYPE_MP4, 0x81);
        assert_eq!(STREAM_TYPE_AVI, 0x82);
    }

    #[test]
    fn msg_type_constants_stable() {
        assert_eq!(MSG_TYPE_DEMUX_DONE, 0);
        assert_eq!(MSG_TYPE_FATAL_ERR, 1);
        assert_eq!(MSG_TYPE_PROG_END_CODE, 2);
        assert_eq!(ES_MSG_TYPE_AU_FOUND, 0);
        assert_eq!(ES_MSG_TYPE_FLUSH_DONE, 1);
    }

    #[test]
    fn is_known_stream_type_helper() {
        assert!(is_known_stream_type(STREAM_TYPE_PAMF));
        assert!(is_known_stream_type(STREAM_TYPE_MP4));
        assert!(is_known_stream_type(STREAM_TYPE_AVI));
        assert!(!is_known_stream_type(99));
        assert!(!is_known_stream_type(-1));
    }

    #[test]
    fn query_attr_pamf_returns_512k() {
        let attr = query_attr(STREAM_TYPE_PAMF).unwrap();
        assert_eq!(attr.mem_size, 512 * 1024);
    }

    #[test]
    fn query_attr_mp4_and_avi_return_1m() {
        assert_eq!(query_attr(STREAM_TYPE_MP4).unwrap().mem_size, 1024 * 1024);
        assert_eq!(query_attr(STREAM_TYPE_AVI).unwrap().mem_size, 1024 * 1024);
    }

    #[test]
    fn query_attr_undef_rejected() {
        assert_eq!(query_attr(STREAM_TYPE_UNDEF), Err(errors::ARG));
    }

    #[test]
    fn query_attr_terminator_rejected() {
        assert_eq!(query_attr(STREAM_TYPE_TERMINATOR), Err(errors::ARG));
    }

    #[test]
    fn query_attr_unknown_type_rejected() {
        assert_eq!(query_attr(999), Err(errors::ARG));
    }

    #[test]
    fn open_happy_path() {
        let mut d = Dmux::new();
        let id = d.open(STREAM_TYPE_PAMF).unwrap();
        assert_eq!(id, 1);
        assert_eq!(d.handle_count(), 1);
    }

    #[test]
    fn open_undef_rejected() {
        let mut d = Dmux::new();
        assert_eq!(d.open(STREAM_TYPE_UNDEF), Err(errors::ARG));
    }

    #[test]
    fn open_terminator_rejected() {
        let mut d = Dmux::new();
        assert_eq!(d.open(STREAM_TYPE_TERMINATOR), Err(errors::ARG));
    }

    #[test]
    fn open_increments_ids() {
        let mut d = Dmux::new();
        let a = d.open(STREAM_TYPE_PAMF).unwrap();
        let b = d.open(STREAM_TYPE_PAMF).unwrap();
        assert_eq!(b, a + 1);
    }

    #[test]
    fn open_exceeds_cap_rejected() {
        let mut d = Dmux::new();
        for _ in 0..MAX_HANDLES {
            d.open(STREAM_TYPE_PAMF).unwrap();
        }
        assert_eq!(d.open(STREAM_TYPE_PAMF), Err(errors::FATAL));
    }

    #[test]
    fn close_bad_id_rejected() {
        let mut d = Dmux::new();
        assert_eq!(d.close(999), Err(errors::ARG));
    }

    #[test]
    fn close_with_active_es_is_busy() {
        let mut d = Dmux::new();
        let h = d.open(STREAM_TYPE_PAMF).unwrap();
        d.enable_es(h, ok_filter(), ok_resource()).unwrap();
        assert_eq!(d.close(h), Err(errors::BUSY));
    }

    #[test]
    fn set_stream_happy_path() {
        let mut d = Dmux::new();
        let h = d.open(STREAM_TYPE_PAMF).unwrap();
        d.set_stream(h, ok_stream()).unwrap();
    }

    #[test]
    fn set_stream_null_addr_rejected() {
        let mut d = Dmux::new();
        let h = d.open(STREAM_TYPE_PAMF).unwrap();
        let mut s = ok_stream();
        s.stream_addr = 0;
        assert_eq!(d.set_stream(h, s), Err(errors::ARG));
    }

    #[test]
    fn set_stream_zero_size_rejected() {
        let mut d = Dmux::new();
        let h = d.open(STREAM_TYPE_PAMF).unwrap();
        let mut s = ok_stream();
        s.stream_size = 0;
        assert_eq!(d.set_stream(h, s), Err(errors::ARG));
    }

    #[test]
    fn reset_stream_idles_all_es() {
        let mut d = Dmux::new();
        let h = d.open(STREAM_TYPE_PAMF).unwrap();
        let e = d.enable_es(h, ok_filter(), ok_resource()).unwrap();
        d.set_stream(h, ok_stream()).unwrap();
        d.inject_au(h, e).unwrap();
        assert_eq!(d.pending_aus(h, e), Ok(1));
        d.reset_stream(h).unwrap();
        assert_eq!(d.pending_aus(h, e), Ok(0));
        assert_eq!(d.es_state(h, e), Ok(EsState::Idle));
    }

    #[test]
    fn enable_es_bad_resource_rejected() {
        let mut d = Dmux::new();
        let h = d.open(STREAM_TYPE_PAMF).unwrap();
        let mut r = ok_resource();
        r.mem_addr = 0;
        assert_eq!(d.enable_es(h, ok_filter(), r), Err(errors::ARG));
    }

    #[test]
    fn enable_es_misaligned_addr_rejected() {
        let mut d = Dmux::new();
        let h = d.open(STREAM_TYPE_PAMF).unwrap();
        let mut r = ok_resource();
        r.mem_addr = 0x1_0001; // not aligned to 0x80
        assert_eq!(d.enable_es(h, ok_filter(), r), Err(errors::ARG));
    }

    #[test]
    fn enable_es_non_pow2_alignment_rejected() {
        let mut d = Dmux::new();
        let h = d.open(STREAM_TYPE_PAMF).unwrap();
        let mut r = ok_resource();
        r.mem_alignment = 3;
        assert_eq!(d.enable_es(h, ok_filter(), r), Err(errors::ARG));
    }

    #[test]
    fn enable_es_duplicate_filter_rejected() {
        let mut d = Dmux::new();
        let h = d.open(STREAM_TYPE_PAMF).unwrap();
        d.enable_es(h, ok_filter(), ok_resource()).unwrap();
        assert_eq!(d.enable_es(h, ok_filter(), ok_resource()), Err(errors::BUSY));
    }

    #[test]
    fn enable_es_different_filter_ok() {
        let mut d = Dmux::new();
        let h = d.open(STREAM_TYPE_PAMF).unwrap();
        d.enable_es(h, ok_filter(), ok_resource()).unwrap();
        let mut f = ok_filter();
        f.stream_id = 0xE1;
        d.enable_es(h, f, ok_resource()).unwrap();
    }

    #[test]
    fn enable_es_over_cap_rejected() {
        let mut d = Dmux::new();
        let h = d.open(STREAM_TYPE_PAMF).unwrap();
        for i in 0..MAX_ES_PER_DMUX {
            let mut f = ok_filter();
            f.stream_id = (0xE0 + i) as u8;
            d.enable_es(h, f, ok_resource()).unwrap();
        }
        let mut f = ok_filter();
        f.stream_id = 0xF0;
        assert_eq!(d.enable_es(h, f, ok_resource()), Err(errors::FATAL));
    }

    #[test]
    fn disable_es_unknown_id_rejected() {
        let mut d = Dmux::new();
        let h = d.open(STREAM_TYPE_PAMF).unwrap();
        assert_eq!(d.disable_es(h, 99), Err(errors::ARG));
    }

    #[test]
    fn disable_es_with_pending_aus_is_busy() {
        let mut d = Dmux::new();
        let h = d.open(STREAM_TYPE_PAMF).unwrap();
        let e = d.enable_es(h, ok_filter(), ok_resource()).unwrap();
        d.inject_au(h, e).unwrap();
        assert_eq!(d.disable_es(h, e), Err(errors::BUSY));
    }

    #[test]
    fn reset_es_sets_flushing_and_clears_aus() {
        let mut d = Dmux::new();
        let h = d.open(STREAM_TYPE_PAMF).unwrap();
        let e = d.enable_es(h, ok_filter(), ok_resource()).unwrap();
        d.inject_au(h, e).unwrap();
        d.reset_es(h, e).unwrap();
        assert_eq!(d.es_state(h, e), Ok(EsState::Flushing));
        assert_eq!(d.pending_aus(h, e), Ok(0));
    }

    #[test]
    fn inject_au_on_idle_es_is_seq() {
        let mut d = Dmux::new();
        let h = d.open(STREAM_TYPE_PAMF).unwrap();
        let e = d.enable_es(h, ok_filter(), ok_resource()).unwrap();
        d.reset_es(h, e).unwrap();
        assert_eq!(d.inject_au(h, e), Err(errors::SEQ));
    }

    #[test]
    fn release_au_happy_path() {
        let mut d = Dmux::new();
        let h = d.open(STREAM_TYPE_PAMF).unwrap();
        let e = d.enable_es(h, ok_filter(), ok_resource()).unwrap();
        d.inject_au(h, e).unwrap();
        d.inject_au(h, e).unwrap();
        assert_eq!(d.pending_aus(h, e), Ok(2));
        d.release_au(h, e).unwrap();
        assert_eq!(d.pending_aus(h, e), Ok(1));
    }

    #[test]
    fn release_au_empty_is_empty_error() {
        let mut d = Dmux::new();
        let h = d.open(STREAM_TYPE_PAMF).unwrap();
        let e = d.enable_es(h, ok_filter(), ok_resource()).unwrap();
        assert_eq!(d.release_au(h, e), Err(errors::EMPTY));
    }

    #[test]
    fn es_state_bad_id_rejected() {
        let mut d = Dmux::new();
        let h = d.open(STREAM_TYPE_PAMF).unwrap();
        assert_eq!(d.es_state(h, 99), Err(errors::ARG));
    }

    #[test]
    fn full_dmux_lifecycle_smoke() {
        let mut d = Dmux::new();
        let attr = query_attr(STREAM_TYPE_PAMF).unwrap();
        assert!(attr.mem_size > 0);
        let h = d.open(STREAM_TYPE_PAMF).unwrap();
        d.set_stream(h, ok_stream()).unwrap();

        // Enable three ESes (video, audio, user-data).
        let mut f1 = ok_filter();
        f1.stream_id = 0xE0;
        let mut f2 = ok_filter();
        f2.stream_id = 0xC0;
        let mut f3 = ok_filter();
        f3.stream_id = 0xBD;
        f3.private_stream_id = 0x22;
        let e1 = d.enable_es(h, f1, ok_resource()).unwrap();
        let e2 = d.enable_es(h, f2, ok_resource()).unwrap();
        let e3 = d.enable_es(h, f3, ok_resource()).unwrap();

        // Demuxer emits AUs → game releases them.
        for _ in 0..5 {
            d.inject_au(h, e1).unwrap();
            d.inject_au(h, e2).unwrap();
        }
        for _ in 0..5 {
            d.release_au(h, e1).unwrap();
            d.release_au(h, e2).unwrap();
        }

        // Reset stream (seek), re-enable, clean up.
        d.reset_stream(h).unwrap();
        d.disable_es(h, e1).unwrap();
        d.disable_es(h, e2).unwrap();
        d.disable_es(h, e3).unwrap();
        d.close(h).unwrap();
        assert_eq!(d.handle_count(), 0);
    }
}
