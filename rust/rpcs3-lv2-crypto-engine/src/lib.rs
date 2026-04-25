//! Rust port of `rpcs3/Emu/Cell/lv2/sys_crypto_engine.cpp` — PS3 LV2 crypto
//! engine syscalls (3 entries, 28 lines C++).
//!
//! `sys_crypto_engine_create(id_out)` / `destroy(id)` / `random_generate(buf, len)`.
//! All upstream stubs returning CELL_OK. Port adds id allocator FSM and
//! captures random buffer requests for test introspection.
//!
//! `no_std` + `alloc`. Single dep: `rpcs3-emu-types`.

#![no_std]

extern crate alloc;

use alloc::vec::Vec;
use rpcs3_emu_types::CellError;

pub const MODULE_NAME: &str = "sys_crypto_engine";

pub const REGISTERED_ENTRY_POINTS: &[&str] = &[
    "sys_crypto_engine_create",
    "sys_crypto_engine_destroy",
    "sys_crypto_engine_random_generate",
];

/// Placeholder error code for double-destroy / unknown id (upstream stub
/// always returns CELL_OK, but FSM enforcement helps tests).
pub const CRYPTO_ENGINE_ESRCH: CellError = CellError(0x8001_0005);

#[derive(Debug, Default)]
pub struct CryptoEngine {
    pub engines: Vec<u32>,
    pub next_id: u32,
    pub create_calls: u64,
    pub destroy_calls: u64,
    pub random_generate_calls: u64,
    pub bytes_generated: u64,
}

impl CryptoEngine {
    pub fn new() -> Self {
        Self {
            next_id: 1,
            ..Default::default()
        }
    }

    /// `sys_crypto_engine_create(id)` — allocates a new engine.
    pub fn create(&mut self, id_out: Option<&mut u32>) -> Result<(), CellError> {
        self.create_calls = self.create_calls.saturating_add(1);
        let id = self.next_id;
        self.next_id = self.next_id.wrapping_add(1);
        self.engines.push(id);
        if let Some(slot) = id_out {
            *slot = id;
        }
        Ok(())
    }

    /// `sys_crypto_engine_destroy(id)` — releases an engine.
    pub fn destroy(&mut self, id: u32) -> Result<(), CellError> {
        self.destroy_calls = self.destroy_calls.saturating_add(1);
        let before = self.engines.len();
        self.engines.retain(|e| *e != id);
        if self.engines.len() == before {
            return Err(CRYPTO_ENGINE_ESRCH);
        }
        Ok(())
    }

    /// `sys_crypto_engine_random_generate(buffer, buffer_size)`.
    pub fn random_generate(
        &mut self,
        _buffer_addr: u32,
        buffer_size: u64,
    ) -> Result<(), CellError> {
        self.random_generate_calls = self.random_generate_calls.saturating_add(1);
        self.bytes_generated = self.bytes_generated.saturating_add(buffer_size);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn module_and_entries() {
        assert_eq!(MODULE_NAME, "sys_crypto_engine");
        assert_eq!(REGISTERED_ENTRY_POINTS.len(), 3);
    }

    #[test]
    fn create_allocates_monotonic_ids() {
        let mut m = CryptoEngine::new();
        let mut id = 0u32;
        m.create(Some(&mut id)).unwrap();
        assert_eq!(id, 1);
        m.create(Some(&mut id)).unwrap();
        assert_eq!(id, 2);
        m.create(Some(&mut id)).unwrap();
        assert_eq!(id, 3);
        assert_eq!(m.engines.len(), 3);
    }

    #[test]
    fn create_with_null_out_still_allocates() {
        let mut m = CryptoEngine::new();
        m.create(None).unwrap();
        assert_eq!(m.engines.len(), 1);
        assert_eq!(m.engines[0], 1);
    }

    #[test]
    fn destroy_unknown_returns_esrch() {
        let mut m = CryptoEngine::new();
        assert_eq!(m.destroy(99), Err(CRYPTO_ENGINE_ESRCH));
    }

    #[test]
    fn destroy_removes_engine() {
        let mut m = CryptoEngine::new();
        let mut id = 0u32;
        m.create(Some(&mut id)).unwrap();
        m.destroy(id).unwrap();
        assert!(m.engines.is_empty());
        assert_eq!(m.destroy(id), Err(CRYPTO_ENGINE_ESRCH));
    }

    #[test]
    fn random_generate_accumulates_bytes() {
        let mut m = CryptoEngine::new();
        m.random_generate(0x4000_0000, 256).unwrap();
        m.random_generate(0x4000_1000, 1024).unwrap();
        m.random_generate(0x4000_2000, 64).unwrap();
        assert_eq!(m.bytes_generated, 256 + 1024 + 64);
        assert_eq!(m.random_generate_calls, 3);
    }

    #[test]
    fn full_lifecycle_smoke() {
        let mut m = CryptoEngine::new();
        let mut id1 = 0u32;
        let mut id2 = 0u32;
        m.create(Some(&mut id1)).unwrap();
        m.create(Some(&mut id2)).unwrap();
        m.random_generate(0, 4096).unwrap();
        m.destroy(id1).unwrap();
        m.random_generate(0, 64).unwrap();
        m.destroy(id2).unwrap();
        assert!(m.engines.is_empty());
        assert_eq!(m.create_calls, 2);
        assert_eq!(m.destroy_calls, 2);
        assert_eq!(m.random_generate_calls, 2);
    }
}
