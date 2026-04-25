//! `rpcs3-rsx-vertex-data` — Rust port of `rpcs3/Emu/RSX/rsx_vertex_data.cpp`.
//!
//! The RSX "push buffer" accumulates per-attribute vertex data as the
//! game submits immediate-mode commands. This crate freezes the helper
//! methods on `push_buffer_vertex_info` that compute vertex layout and
//! the vertex-ID during accumulation — pure math functions independent
//! of the RSX thread.
//!
//! Frozen:
//!
//! - `VertexBaseType` discriminants from `gcm_enums.h:1161..1226` and
//!   `1219..1225`. RSX uses two "parallel" enums: a raw `u8` table and
//!   the typed `vertex_base_type` that aliases to the same values.
//! - `get_vertex_size_in_dwords(type, size)` math (cpp:18..36):
//!   - `f` → `size` dwords
//!   - `ub`/`ub256` → 1 dword
//!   - `s1`/`s32k` → `size / 2` dwords
//!   - anything else → error (we return `None`).
//! - `get_vertex_id(size, dword_count, vertex_size)` (cpp:38..45):
//!   `size ? dword_count / vertex_size : 0`.

/// Raw RSX vertex-base-type codes (`RSX_VERTEX_BASE_TYPE_*`,
/// `gcm_enums.h:1161..1168`). Stored as `u8` on the wire.
pub const RSX_VERTEX_BASE_TYPE_UNDEFINED: u8 = 0;
pub const RSX_VERTEX_BASE_TYPE_SNORM16: u8 = 1;
pub const RSX_VERTEX_BASE_TYPE_FLOAT: u8 = 2;
pub const RSX_VERTEX_BASE_TYPE_HALF_FLOAT: u8 = 3;
pub const RSX_VERTEX_BASE_TYPE_UNORM8: u8 = 4;
pub const RSX_VERTEX_BASE_TYPE_SINT16: u8 = 5;
pub const RSX_VERTEX_BASE_TYPE_CMP32: u8 = 6;
pub const RSX_VERTEX_BASE_TYPE_UINT8: u8 = 7;

/// The typed enum from `gcm_enums.h:1217..1226`. Discriminants alias
/// the raw codes above.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VertexBaseType {
    S1 = RSX_VERTEX_BASE_TYPE_SNORM16,
    F = RSX_VERTEX_BASE_TYPE_FLOAT,
    Sf = RSX_VERTEX_BASE_TYPE_HALF_FLOAT,
    Ub = RSX_VERTEX_BASE_TYPE_UNORM8,
    S32k = RSX_VERTEX_BASE_TYPE_SINT16,
    Cmp = RSX_VERTEX_BASE_TYPE_CMP32,
    Ub256 = RSX_VERTEX_BASE_TYPE_UINT8,
}

/// Vertex size in dwords (cpp:18..36). Types always fit into 32-bit
/// slots: no fewer than 4 8-bit values and no fewer than 2 16-bit
/// values. Returns `None` for unsupported types (`Sf` / `Cmp` — the
/// cpp throws an exception in those cases).
#[must_use]
pub fn get_vertex_size_in_dwords(base_type: VertexBaseType, size: u8) -> Option<u8> {
    match base_type {
        VertexBaseType::F => Some(size),
        VertexBaseType::Ub | VertexBaseType::Ub256 => Some(1),
        VertexBaseType::S1 | VertexBaseType::S32k => Some(size / 2),
        // cpp throws `"Unsupported vertex base type"` — we surface it as None.
        VertexBaseType::Sf | VertexBaseType::Cmp => None,
    }
}

/// Current vertex ID in an accumulating push buffer (cpp:38..45).
/// - `size == 0` (attribute not yet pinned) → id 0.
/// - Otherwise → `dword_count / vertex_size_dwords`.
///
/// Returns `None` when `vertex_size` would be zero (malformed input).
#[must_use]
pub fn get_vertex_id(size: u8, dword_count: u32, vertex_size_dwords: u8) -> Option<u32> {
    if size == 0 {
        return Some(0);
    }
    if vertex_size_dwords == 0 {
        return None;
    }
    Some(dword_count / u32::from(vertex_size_dwords))
}

/// Required buffer length in dwords for `required_vertex_count` vertices
/// of `vertex_size_dwords` each (cpp:83 `data.resize(vertex_size * n)`).
#[must_use]
pub const fn buffer_dword_capacity(vertex_size_dwords: u8, required_vertex_count: u32) -> u64 {
    (vertex_size_dwords as u64) * (required_vertex_count as u64)
}

/// Compute the destination offset (in dwords) inside the push buffer for
/// the `vertex_id`-th vertex (cpp:70 `current_vertex = data + (vertex_count
/// - 1) * vertex_size`).
#[must_use]
pub const fn vertex_data_offset_dwords(vertex_id: u32, vertex_size_dwords: u8) -> u64 {
    (vertex_id as u64) * (vertex_size_dwords as u64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vertex_base_type_raw_codes() {
        assert_eq!(RSX_VERTEX_BASE_TYPE_UNDEFINED, 0);
        assert_eq!(RSX_VERTEX_BASE_TYPE_SNORM16, 1);
        assert_eq!(RSX_VERTEX_BASE_TYPE_FLOAT, 2);
        assert_eq!(RSX_VERTEX_BASE_TYPE_HALF_FLOAT, 3);
        assert_eq!(RSX_VERTEX_BASE_TYPE_UNORM8, 4);
        assert_eq!(RSX_VERTEX_BASE_TYPE_SINT16, 5);
        assert_eq!(RSX_VERTEX_BASE_TYPE_CMP32, 6);
        assert_eq!(RSX_VERTEX_BASE_TYPE_UINT8, 7);
    }

    #[test]
    fn typed_enum_aliases_raw_codes() {
        assert_eq!(VertexBaseType::S1 as u8, 1);
        assert_eq!(VertexBaseType::F as u8, 2);
        assert_eq!(VertexBaseType::Sf as u8, 3);
        assert_eq!(VertexBaseType::Ub as u8, 4);
        assert_eq!(VertexBaseType::S32k as u8, 5);
        assert_eq!(VertexBaseType::Cmp as u8, 6);
        assert_eq!(VertexBaseType::Ub256 as u8, 7);
    }

    #[test]
    fn vertex_size_in_dwords_float_is_size() {
        assert_eq!(get_vertex_size_in_dwords(VertexBaseType::F, 4), Some(4));
        assert_eq!(get_vertex_size_in_dwords(VertexBaseType::F, 3), Some(3));
        assert_eq!(get_vertex_size_in_dwords(VertexBaseType::F, 1), Some(1));
    }

    #[test]
    fn vertex_size_in_dwords_ub_variants_always_one() {
        assert_eq!(get_vertex_size_in_dwords(VertexBaseType::Ub, 4), Some(1));
        assert_eq!(get_vertex_size_in_dwords(VertexBaseType::Ub256, 4), Some(1));
    }

    #[test]
    fn vertex_size_in_dwords_16bit_types_halved() {
        assert_eq!(get_vertex_size_in_dwords(VertexBaseType::S1, 4), Some(2));
        assert_eq!(get_vertex_size_in_dwords(VertexBaseType::S32k, 2), Some(1));
    }

    #[test]
    fn vertex_size_in_dwords_unsupported_types_none() {
        assert_eq!(get_vertex_size_in_dwords(VertexBaseType::Sf, 4), None);
        assert_eq!(get_vertex_size_in_dwords(VertexBaseType::Cmp, 4), None);
    }

    #[test]
    fn get_vertex_id_zero_size_always_zero() {
        assert_eq!(get_vertex_id(0, 100, 4), Some(0));
        assert_eq!(get_vertex_id(0, 0, 0), Some(0));
    }

    #[test]
    fn get_vertex_id_computes_dword_count_div_vertex_size() {
        assert_eq!(get_vertex_id(4, 0, 4), Some(0));
        assert_eq!(get_vertex_id(4, 4, 4), Some(1));
        assert_eq!(get_vertex_id(4, 15, 4), Some(3));
        assert_eq!(get_vertex_id(4, 16, 4), Some(4));
    }

    #[test]
    fn get_vertex_id_zero_vertex_size_rejected() {
        assert_eq!(get_vertex_id(4, 100, 0), None);
    }

    #[test]
    fn buffer_dword_capacity_multiplies() {
        assert_eq!(buffer_dword_capacity(4, 100), 400);
        assert_eq!(buffer_dword_capacity(1, 100), 100);
        assert_eq!(buffer_dword_capacity(0, 100), 0);
    }

    #[test]
    fn vertex_data_offset_dwords_is_id_times_size() {
        assert_eq!(vertex_data_offset_dwords(0, 4), 0);
        assert_eq!(vertex_data_offset_dwords(1, 4), 4);
        assert_eq!(vertex_data_offset_dwords(10, 4), 40);
        assert_eq!(vertex_data_offset_dwords(1_000_000, 4), 4_000_000);
    }
}
