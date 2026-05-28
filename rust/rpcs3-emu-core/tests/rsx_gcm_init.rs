//! R13.1 — cellGcm init verification.
//!
//! Runs the PSL1GHT fixture `single_gcm_init_v1.self` (a minimal
//! `rsxInit(&ctx, 0x10000, 1 MB, host_buffer)` then `return 0xC0DE`)
//! through `EmuCore::run_self` and asserts that the cellGcm HLE set up
//! the command-buffer state EXACTLY as RPCS3's `_cellGcmInitBody`
//! (`Emu/Cell/Modules/cellGcmSys.cpp:451-453`) +
//! `cellGcmGetConfiguration` do.
//!
//! Before R13.1 this fixture faulted with a null store at CIA 0x12784:
//! `cellGcmGetConfiguration` returned 0 without filling the config, so
//! PSL1GHT's local-memory pool allocator (which reads `localAddress` /
//! `localSize` from the config and writes a free-block header at the
//! base) stored to address 0. Implementing the two cellGcmSys NIDs
//! (`_cellGcmInitBody` 0x15bae46b + `cellGcmGetConfiguration`
//! 0xe315a0b2) and backing the local video-memory region unblocked it.
//!
//! NOTE on capture: this fixture emits NO GCM commands (it only calls
//! rsxInit), so the live command stream `[begin .. current)` is empty
//! by design — this test asserts the buffer is *initialized* and that
//! the capture path reads the real cellGcm context. A NON-EMPTY
//! real-libgcm capture (rsxInit + rsxClearSurface/draw through the real
//! context) is the next slice and needs a command-emitting fixture
//! built via the Docker PSL1GHT toolchain. The decode/replay pipeline
//! itself is already validated against real PSL1GHT bytes by R12.11b
//! (`rsx_capture_smoke.rs`).
//!
//! Skips gracefully when the fixture `.self` is absent.

use std::path::PathBuf;

use rpcs3_emu_core::EmuCore;
use rpcs3_rsx_state::replay_gcm;

/// Kernel-side gcm struct placement chosen by EmuCore's cellGcm HLE
/// (mirrors RPCS3 render->device_addr / dma_address; we carve a fixed
/// unused page). Must match `lib.rs` GCM_CTX_ADDR / GCM_CONTROL_ADDR.
const GCM_CTX_ADDR: u32 = 0x3000_0000;
const GCM_CONTROL_ADDR: u32 = 0x3000_0040;

/// Deterministic guest address of the fixture's 1 MB-aligned
/// `host_buffer` (the ioAddress passed to rsxInit) for the committed
/// `.self`. begin = ioAddress + 4096 (4 KB reserved header).
const IO_ADDRESS: u32 = 0x1020_0000;

fn fixture_self() -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.pop(); // rust/
    p.pop(); // rpcs3-master/
    p.push("behavior-freeze");
    p.push("fixtures");
    p.push("rsx");
    p.push("sources");
    p.push("single_gcm_init_v1");
    p.push("single_gcm_init_v1.self");
    p
}

fn read_be_u32(core: &EmuCore, addr: u32) -> u32 {
    let mut b = [0u8; 4];
    core.mem.read(addr, &mut b).expect("read gcm struct");
    u32::from_be_bytes(b)
}

#[test]
fn gcm_init_sets_up_command_buffer_context() {
    let path = fixture_self();
    if !path.exists() {
        eprintln!(
            "[R13.1] skip: {} not present (build via Docker PSL1GHT)",
            path.display()
        );
        return;
    }
    let bytes = std::fs::read(&path).expect("read .self");
    let mut core = EmuCore::new();
    core.step_budget = 5_000_000;
    core.permissive_unknown_syscalls = false;

    let report = core.run_self(&bytes).expect("run_self");

    // rsxInit completed and main returned 0xC0DE → no null store.
    assert_eq!(
        report.exit_status.status, 0xC0DE,
        "fixture must run rsxInit to completion and return 0xC0DE"
    );

    // CellGcmContextData (GCM.h:26) = 4 BE u32: begin/end/current/
    // callback. Values per cellGcmSys.cpp:451-453.
    let begin = read_be_u32(&core, GCM_CTX_ADDR);
    let end = read_be_u32(&core, GCM_CTX_ADDR + 4);
    let current = read_be_u32(&core, GCM_CTX_ADDR + 8);

    let expected_begin = IO_ADDRESS + 4096; // 4 KB reserved at start
    let expected_end = IO_ADDRESS + 32 * 1024 - 4; // 4 B for jump at end
    assert_eq!(begin, expected_begin, "context.begin = ioAddress + 4096");
    assert_eq!(end, expected_end, "context.end = ioAddress + 32K - 4");
    assert_eq!(current, begin, "context.current starts at begin");

    // CellGcmControl (GCM.h:5) = put/get/ref BE u32. ref starts
    // 0xffffffff per hardware; put/get 0.
    let put = read_be_u32(&core, GCM_CONTROL_ADDR);
    let get = read_be_u32(&core, GCM_CONTROL_ADDR + 4);
    let ref_ = read_be_u32(&core, GCM_CONTROL_ADDR + 8);
    assert_eq!(put, 0, "control.put = 0 (nothing emitted)");
    assert_eq!(get, 0, "control.get = 0");
    assert_eq!(ref_, 0xFFFF_FFFF, "control.ref starts 0xffffffff");

    // Capture path wired to the REAL init'd buffer: the live stream is
    // [begin .. current). For this init-only fixture current == begin,
    // so the captured stream is empty — and decodes to an empty
    // snapshot. (A non-empty stream comes from an emitting fixture.)
    let put_bytes = current - begin;
    assert_eq!(put_bytes, 0, "init-only fixture emits no command words");

    let mut stream = vec![0u8; put_bytes as usize];
    if put_bytes > 0 {
        core.mem.read(begin, &mut stream).expect("read command buffer");
    }
    let snapshot = replay_gcm(&stream, put_bytes).expect("replay empty stream");
    assert!(
        snapshot.draw_calls.is_empty() && snapshot.effects.is_empty(),
        "empty stream → empty snapshot"
    );

    eprintln!(
        "[R13.1] gcm context initialized: begin=0x{begin:08x} \
         end=0x{end:08x} current=0x{current:08x} ref=0x{ref_:08x} \
         (captured {put_bytes} bytes)"
    );
}
