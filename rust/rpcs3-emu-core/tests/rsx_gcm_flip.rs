//! R13.4 — first full PSL1GHT flip-path fixture (gcmSetDisplayBuffer
//! → gcmSetFlip → rsxFlushBuffer → gcmGetFlipStatus spin →
//! gcmResetFlipStatus) running end-to-end through EmuCore.
//!
//! Reaching this point needed two new cellGcmSys NID handlers in
//! addition to R13.1's init pair:
//!   - cellGcmGetControlRegister (0xa547adde) — returns the guest
//!     pointer of the CellGcmControl block placed by _cellGcmInitBody.
//!     libgcm's flip wrappers deref this for put/get/ref; silent
//!     return of 0 was the R13.4 wall.
//!   - cellGcmAddressToOffset (0x21ac3697) — translates a PPU effective
//!     address to the RSX IO offset that the GPU uses. The function
//!     returns the result via an OUT pointer; silent return of 0
//!     without writing left the caller reading garbage.
//!
//! Other cellGcmSys NIDs that the fixture calls are tolerated as
//! silent-0 returns (no OUT pointers, status-only callers):
//!   - cellGcmSetDisplayBuffer (0xa53d12ae)
//!   - cellGcmSetFlip          (0xdc09357e)
//!   - cellGcmGetFlipStatus    (0x72a577ce) — 0 means "flip done";
//!     the homebrew's spin loop exits immediately.
//!   - cellGcmResetFlipStatus  (0xb2e761d4)
//!
//! Captured stream assertions: the clear + draw + label words from
//! R13.3 must still be present; the snapshot keeps its ClearSurface
//! effect and a TRIANGLES DrawCall. rsxFlushBuffer's inline effect on
//! the captured [begin..current) range is small (a NOP or a SET_REF
//! word at most) — we do NOT pin the exact word count to avoid
//! over-constraining the test against libgcm internals.
//!
//! Skips gracefully when the `.self` is absent.

use std::path::PathBuf;

use rpcs3_emu_core::EmuCore;
use rpcs3_rsx_state::{replay_gcm, DrawKind, MethodEffect};

fn fixture_self() -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.pop();
    p.pop();
    p.push("behavior-freeze");
    p.push("fixtures");
    p.push("rsx");
    p.push("sources");
    p.push("single_gcm_setdisplay_v1");
    p.push("single_gcm_setdisplay_v1.self");
    p
}

fn read_be_u32(core: &EmuCore, addr: u32) -> u32 {
    let mut b = [0u8; 4];
    core.mem.read(addr, &mut b).expect("read gcm struct");
    u32::from_be_bytes(b)
}

#[test]
fn flip_path_runs_end_to_end_and_stream_decodes() {
    let path = fixture_self();
    if !path.exists() {
        eprintln!(
            "[R13.4] skip: {} not present (build via Docker PSL1GHT)",
            path.display()
        );
        return;
    }

    let bytes = std::fs::read(&path).expect("read .self");
    let mut core = EmuCore::new();
    core.step_budget = 10_000_000;
    core.permissive_unknown_syscalls = false;

    let report = core.run_self(&bytes).expect("run_self");
    assert_eq!(
        report.exit_status.status, 0xC0DE,
        "full flip path must run to completion: init -> clear -> draw \
         -> setDisplayBuffer -> setFlip -> flushBuffer -> getFlipStatus \
         spin -> resetFlipStatus -> label -> 0xC0DE"
    );

    assert_ne!(core.gcm_context_addr, 0);
    let begin = read_be_u32(&core, core.gcm_context_addr);
    let current = read_be_u32(&core, core.gcm_context_addr + 8);
    assert!(current > begin, "stream must be non-empty");
    let put_bytes = current - begin;
    assert_eq!(put_bytes % 4, 0);

    let mut stream = vec![0u8; put_bytes as usize];
    core.mem
        .read(begin, &mut stream)
        .expect("read command buffer");
    let snap = replay_gcm(&stream, put_bytes).expect("replay stream");

    // ClearSurface + DrawCall from R13.3 still present.
    assert!(
        snap.effects
            .iter()
            .any(|e| matches!(e, MethodEffect::ClearSurface(0xF3))),
        "stream must still contain ClearSurface(0xF3); effects={:?}",
        snap.effects
    );
    assert!(
        !snap.draw_calls.is_empty(),
        "stream must still contain the TRIANGLES DrawCall"
    );
    let draw = &snap.draw_calls[0];
    assert_eq!(draw.primitive, 5);
    assert_eq!(draw.kind, DrawKind::Arrays);
    assert!(draw.ranges.iter().any(|&(f, c)| f == 0 && c == 3));

    eprintln!(
        "[R13.4] flip-path fixture ran to 0xC0DE; captured {} words \
         ({} bytes); effects={} draw_calls={}",
        put_bytes / 4,
        put_bytes,
        snap.effects.len(),
        snap.draw_calls.len()
    );
}
