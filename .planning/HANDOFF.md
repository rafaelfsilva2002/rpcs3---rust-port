# Handoff — R13 sub-wave critical-path closed (2026-05-28)

This doc is the operational baton for continuing the RPCS3 → Rust
port from a fresh session (e.g., new terminal session, new model).
Read this top-to-bottom — it's the minimum context to start the
next slice without re-discovering things.

## Audit snapshot (2026-05-28)

A full 6-agent code audit (verified against a green gate) produced
**[`docs/PORT_STATUS_AND_ROADMAP.md`](../docs/PORT_STATUS_AND_ROADMAP.md)** —
the consolidated done/not-done matrix + the "run on a toaster"
optimization roadmap. Key code-verified facts:

- Gate GREEN: `cargo test --workspace --tests --release` = 280 blocks /
  0 fail / 6015 tests. (The PROJECT_STATUS §4 "`--release` breaks on
  HLE crates" note is STALE.)
- 242 workspace crates; **137 `rpcs3-hle-*` are real ABI ports but NOT
  wired into emu-core** (unconsumed islands).
- 12 RSX/GCM crates (docs said 3); `gl/vk-decompiler` are name-tables,
  not shader decompilers.
- PPU is interpreter-only; the SPU Cranelift JIT is real but **dead on
  the live path** (emu-core/FFI use the interpreter).
- OE-arith overflow IS implemented (R11.4b) despite "deferred" notes.
- **`main` is 120 commits ahead of `origin` (UNPUSHED)** — back up
  before further work.
- Top-3 cheapest perf wins: wire the SPU JIT in, flat-mmap memory
  backend, aggressive release profile (`panic="abort"`/`strip`).

## Where the port stands

**HEAD:** `6e7639ad1 docs: PROJECT_STATUS mirrors R13.4 full
flip-path landing` on `main`.

**Gate:** `cargo test --workspace --tests --release` = **280
blocks, 0 fail, 6015 asserts**. 20 SPU oracles intact across the
whole session.

**Sub-waves status:**
- **R5–R8** — SPU subsystem, 20 oracles total. Closed.
- **R9** — LV2/PPU integration via PSL1GHT runtime init pipeline.
  Closed architecturally 2026-05-25 (`5b51b7b46`).
- **R10** — LV2 sync primitives library + PPU-only fixture
  pipeline. Closed 2026-05-26.
- **R11** — PPU interpreter completion (scalar + VMX/AltiVec).
  Closed 2026-05-26.
- **R12** — RSX pure pipeline (FIFO decode + state register file +
  resource descriptors + 3-tier replay oracles incl. real
  PSL1GHT-libgcm capture). Closed 2026-05-27.
- **R13** — cellGcm HLE (this wave). R13.1–R13.4 landed
  2026-05-27..28; critical-path goal closed.

**R13 session output (8 commits, 4 slices):**

| Slice | Code SHA | Docs SHA | Headline |
|---|---|---|---|
| R13.1 | `f0ef80774` | `2e6802c8a` | cellGcm init HLE; `_cellGcmInitBody` + `cellGcmGetConfiguration`; `rsxInit` runs to 0xC0DE |
| R13.2 | `97cb7bb1a` | `326c72554` | first NON-EMPTY real-libgcm capture; 10 words / `ClearSurface(0xF3)` |
| R13.3 | `9d31643ed` | `9eeb33f06` | first real `DrawCall{primitive=5, kind=Arrays, ranges=[(0,3)]}`; 20w / 80B |
| R13.4 | `356828b37` | `6e7639ad1` | full clear+draw+FLIP frame end-to-end; `cellGcmGetControlRegister` + `cellGcmAddressToOffset` |

The behavior-freezable half of the RSX pipeline (decode → state →
DrawTracker) is now replay-validated against REAL PSL1GHT libgcm
output for the complete basic frame. The GPU backend (Camadas
C/D/E: shader decompile, texture pixel-decode, Vulkan/GL, display)
stays deferred — needs GPU, not behavior-freezable.

## Living documents (read order)

1. **`docs/PROJECT_STATUS.md`** — canonical source of truth; title
   + leading callout reflect R13.4. Long-form historical sections
   below are legacy R6–R9 closures.
2. **`.planning/R13_CELLGCM_HLE_PLAN.md`** — full R13 plan with
   slice-by-slice status (R13.1 → R13.4 LANDED).
3. **`.planning/R12_RSX_GPU_PLAN.md`** — R12 closure doc, defines
   R13 as the next-advance sub-wave.
4. **`.planning/.loop_active`** — autonomous-loop sentinel. If you
   want auto-resume on next SessionStart, leave it. To finish the
   loop manually, `rm` it.

## What's queued (R13.5+ candidates)

All incremental enrichment now that the critical-path frame works.
No NID walls expected — these are pure inline emission slices:

| Candidate | What it exercises | Why |
|---|---|---|
| **R13.5a multi-draw** | DrawTracker handling N draws in one frame | 2+ DrawCalls in snapshot; currently only 1 ever asserted |
| **R13.5b indexed draw** | `rsxDrawIndexArray` → `DrawKind::Indexed` | exercises the indexed code path in DrawTracker |
| **R13.5c surface setup** | `rsxSetSurface(&surface)` populating `SurfaceDescriptor` | validates Camada B `SurfaceDescriptor` against real libgcm |
| **R13.5d viewport / depth state** | `rsxSetViewport` / `rsxSetDepthFunc` / `rsxSetScissor` | populates `MethodEffect`-class state changes |
| **R13.5e texture binding** | `rsxLoadTexture` populating `TextureDescriptor` | validates Camada B `TextureDescriptor` against real libgcm |
| **R13.5f cross-frame** | `rsxFlip × 2` with reset between | exercises the flip-reset cycle |

**R13.5c (surface) LANDED 2026-05-29** (commit `7538323a5`) — `single_gcm_surface_v1`
validates the full `SurfaceDescriptor` against real libgcm AND found+fixed an
NV4097 PITCH_A wrong-address decode bug (0x0218→0x020C) in BOTH `rpcs3-rsx-state`
and `rpcs3-rsx-gcm` (self-referential unit tests missed it; the real capture +
round-trip oracle caught it). Gate 282/0/6017, pushed.

**R13.5e (texture) LANDED 2026-05-29** (commit `c25cf3c59`) — `single_gcm_texture_v1`
validates `TextureDescriptor` against real libgcm (format_code=0xA5, TwoD,
256×128, offset 0x200000, RSX). The texture method addresses already matched
RPCS3 gcm_enums.h, so no bug — confirmed the decode. Gate 283/0/6018, pushed.

**R13.5b (indexed draw) LANDED 2026-05-29** (commit `a4410dc90`) —
`single_gcm_indexdraw_v1` validates `DrawKind::Indexed` + `IndexArray`
{address 0x10000, U16, RSX} against real libgcm; no decode bug (index-array
addresses matched RPCS3). Gate 284/0/6019, pushed.

**R13.5a (multi-draw) LANDED 2026-05-29** (commit `76d55f24f`) —
`single_gcm_multidraw_v1` (two rsxDrawVertexArray) → 2 DrawCalls [(0,3)] +
[(10,6)]. Gate 285/0/6020, pushed.

**R13.5d (viewport) LANDED 2026-05-29** (commit `48cd326e5`) —
`single_gcm_viewport_v1` validates `SET_VIEWPORT_HORIZONTAL`=0x02800000 (640) +
`VERTICAL`=0x01e00000 (480) by decoding the stream into an `RsxState`
(test-only; needed a `rpcs3-rsx-fifo` dev-dep on emu-core). Gate 286/0/6021, pushed.

**R13.5 descriptor wave COMPLETE: c/e/b/a/d DONE (5 slices + the PITCH_A decode
bug fix). Gate 282 → 286.** Only **R13.5f (cross-frame flip×2)** remains, and it
is NOT cleanly mechanical: the flip updates the control-memory PUT pointer, not
the cmd buffer, so there's no new snapshot/register surface to assert (R13.4
already proved the single-flip path; a "×2" adds no decode coverage). NEEDS A
DECISION — skip it, or take a different validation angle.

**R13.6 (first HLE integration) LANDED 2026-05-29** (commit `ea6ee3988`) — wired
the FIRST of the 137 unconsumed `rpcs3-hle-*` crates: `cellSysutil` (NID
`0x40e895d3` → `rpcs3-hle-cellsysutil::cell_sysutil_get_system_param_int`, backed
by an `EmuSysutilConfig` fixed-config provider) runs end-to-end via
`single_sysutil_param_v1` (NEW `behavior-freeze/fixtures/hle/` category).
Gate 287/0/6022. **HLE NID-dispatch pattern established** (Cargo dep + state
provider impl + `match nid` arm; confirm the NID at runtime from the
`[R9.1g.7] unimplemented import: cellModule::0xNNNN` log, r3..r10 = args, write
OUT ptrs BE, r3 = return) — directly replicable for the other 136 crates.

**R13.7 (cellSysModule — 2nd HLE crate, STATEFUL pattern) LANDED 2026-05-29**
(stage-1 commit on `main`) — wired `cellSysModule`: `cellSysmoduleLoadModule`
(NID `0x32267a31`) + `cellSysmoduleIsLoaded` (NID `0x5a59e258`) →
`rpcs3-hle-cellsysmodule` against a **persistent `SysmoduleManager` field on
`EmuCore`** (init in `new()`, mirrors `lv2_sync_state`) — establishing the
**stateful** HLE variant (state survives across guest calls, unlike the
stateless `EmuSysutilConfig` provider). NIDs captured at runtime, not guessed.
Fixture `single_sysmodule_v1` packs the lifecycle into the exit code
(`bit0 not-loaded-before | bit1 load-ok | bit2 loaded-after`) → **0x7 post-wire
vs 0x6 pre-wire** (the return-0 stub wrongly reads loaded before the load).
Also fixed a latent R13.6 gap: the global `.gitignore` `Makefile` rule swallowed
the `hle/` fixture Makefiles (negation existed for spu/lv2/rsx, not hle) — added
the hle negation and committed both the cellSysModule and cellSysutil Makefiles.
Gate **288/0/6023**.

**R13.8 (cellSysutil string param) LANDED 2026-05-29** — extends the cellSysutil
integration to the **string** path, reusing the dep + `EmuSysutilConfig`.
`cellSysutilGetSystemParamString` (NID `0x938013a0`, runtime-captured) → the arm
copies the returned string into the guest buffer (truncated to bufsize-1 +
NUL-terminated); `EmuSysutilConfig::get_param_string` returns a default nickname
`"RPCS3"` for Nickname/CurrentUsername. Fixture `single_sysutil_string_v1`
byte-sums the buffer → **363 ("RPCS3") post-wire vs 0 pre-wire** (stub never
writes), proving CONTENT end-to-end (not just that the call returned). Gate
**289/0/6024**.

Useful gotcha learned: the import-trampoline path (`[R9.1g.7]`) ALWAYS logs the
NID + returns 0 regardless of `permissive_unknown_syscalls` — that flag only
gates the *numbered* syscalls. So a probe test can run with `permissive=false`;
the unwired NID still prints in the log (and the strict assert just fails until
wired).

**R13.9 (cellVideoOut — first STATELESS HLE crate) LANDED 2026-05-29** —
`cellVideoOutGetResolution` (NID `0xe558748d`, runtime-captured; PSL1GHT links it
into libsysutil so the log tags it `cellSysutil::`) → `rpcs3-hle-cellvideoout::
cell_video_out_get_resolution`, a pure id→width/height table lookup with **no
EmuCore state field and no provider** (unlike cellSysutil/cellSysModule). The arm
writes width/height as BE `u16` into the guest `videoResolution{u16 w;u16 h}`.
Fixture `single_videoout_resolution_v1` packs `(w<<16)|h` → **0x050002D0
(1280×720) post-wire vs 0 pre-wire**. Gate **290/0/6025**. This completes the
trio of HLE wiring shapes: **provider** (cellSysutil fixed config), **stateful
EmuCore field** (cellSysModule SysmoduleManager), **stateless lookup**
(cellVideoOut).

**R13.10 (cellVideoOutGetNumberOfDevice) LANDED 2026-05-29** — 2nd cellVideoOut
fn (NID `0x75bbb672`), reuses the GetResolution dep; stateless count returned
directly in r3. Fixture `single_videoout_numdevices_v1` → **1 post-wire vs 0
pre-wire**. Shows the dep-reuse path (a 2nd fn from an already-wired crate = one
more arm). Gate **291/0/6026**.

**R13.11 (sys_net inet_addr) + HLE backlog LANDED 2026-05-29** — an ULTRACODE
discovery workflow (12 cluster agents + adversarial verify + synth) produced
**`.planning/HLE_BACKLOG.md`**: a ranked, vetted list of next NID-dispatch
targets. First item executed: `sys_net inet_addr` (NID `0xdabbc2c0`) →
`rpcs3-hle-sys-net-user::inet_addr_stub` = `0xFFFFFFFF` (INET_ADDR_NONE, byte-exact
firmware stub). Fixture `single_net_inet_addr_v1` → **1 post-wire vs 0 pre-wire**
(confirmed inet_addr dispatches a real NID, not inlined). Gate **292/0/6027**.

**CAUTION on the backlog:** its PSL1GHT-exposure claims are UNRELIABLE — always
re-verify with `docker run ... grep /opt/ps3dev/psl1ght/ppu/include` before
authoring. The cellAudioOut family it ranked #2/#3/#7/#8/#9 is NOT exposed
(audio.h exposes only cellAudio *rendering*: audioInit/PortOpen/GetPortConfig).
Confirmed-exposed viable next: netCtl (netctl.h: netCtlInit/GetState/GetInfo),
jpgdec, font (fontGetStubRevisionFlags).

**R13.12 (cellNetCtl init + get-state) LANDED 2026-05-29** — NIDs `0xbd5a59fc`
(Init) + `0x8b3eba69` (GetState) → `rpcs3-hle-cellnetctl`. Adds a `netctl:
NetCtlManager` field (stateful gate) + a fixed `StubConnectedBackend` provider
(emulated console reports a connected link) — **combines the stateful + provider
shapes**. GetState writes IPOBTAINED (3) BE-s32 to the OUT ptr (which is in **r3**,
not r4 as the backlog guessed — runtime capture corrected it). Fixture
`single_netctl_state_v1` → **3 post-wire vs 0x55 pre-wire**. Gate **293/0/6028**.

**R13.13 (cellNetCtlGetInfo MTU) LANDED 2026-05-29** — NID `0x1e585b5d` (r3=code,
r4=&union) → `cell_net_ctl_get_info` against the SHARED netctl field+backend (one
more arm, no new dep/field — the field-reuse path for stateful crates). INFO_MTU=3
→ `NetInfo::Mtu(1500)` written BE-u32 to the union @0. Fixture
`single_netctl_mtu_v1` → **1500 (0x5DC) post-wire vs 0 pre-wire**. Gate
**294/0/6029**.

HLE wave so far (10 functions / 5 crates / 7 modules this run): cellSysutil
int+string, cellSysModule load+isloaded, cellVideoOut resolution+numdevices,
sys_net inet_addr, cellNetCtl init+getstate+getinfo-MTU. All 3 wiring shapes
(provider/stateful/stateless + combos) + the dep/field-reuse path proven.
Remaining backlog (`.planning/HLE_BACKLOG.md`) thins out: jpgdec (multi-arm
Create+Open, check for callbacks), font (verify NID-dispatch vs inlined), cellGame
(verify exposure + lifecycle). If those need callbacks/inline/lifecycle, that's
the natural STOP point for the wave.

Next options: (a) continue the HLE wave — next PSL1GHT-exposed module with a
NON-ZERO distinguishable return (`cellVideoOutGetState`, `cellGameGetParamInt`,
`cellAudioOutGetSoundAvailability`; NOTE cellPad/cellAudioInit return zeros = not
distinguishable from the return-0 stub, skip); (b) the other big waves — GPU
rendering backend, or commercial-game / SELF-decrypt boot. See
docs/PORT_STATUS_AND_ROADMAP.md.

Out of scope (still deferred): shader decompilation, texture pixel
decode, Vulkan/GL backend, actual rendering. These need a GPU and
are not byte-exact behavior-freezable.

## Slice playbook (proven 4× this session)

1. **Write fixture** at
   `behavior-freeze/fixtures/rsx/sources/single_gcm_<name>_v1/`
   (main.c + Makefile, CC0 header). Copy Makefile from
   `single_gcm_setdisplay_v1` and change `TARGET`.
2. **Build via Docker** (PSL1GHT toolchain
   `rpcs3-ps3dev-toolchain:local`):
   ```
   MSYS_NO_PATHCONV=1 docker run --rm \
     -v "<literal-windows-path>:/work" \
     -w /work rpcs3-ps3dev-toolchain:local bash -lc 'make'
   ```
   Use the literal Windows path (with spaces+comma quoted) — Docker
   29.5+ **dropped `subst` drive support** in `-v` mounts.
3. **Write capture test** at
   `rust/rpcs3-emu-core/tests/rsx_gcm_<name>.rs` (mirror
   `rsx_gcm_draw.rs` / `rsx_gcm_flip.rs`):
   - run via `EmuCore::run_self`, assert exit 0xC0DE
   - read `[context.begin .. current)` from EmuCore memory
   - decode with `replay_gcm`
   - assert snapshot contents (effects, draw_calls, descriptors)
4. **If the fixture FAULTS instead of returning 0xC0DE** — write a
   diagnostic probe (mirror `rsx_setdisplay_probe.rs`) and run it.
   It dumps PPU state + nearby import-stub NIDs.
5. **Identify unimplemented NIDs** by hashing names from the
   relevant RPCS3 `REG_FUNC` list against
   `ppu_generate_id(name) = SHA1(name + suffix)[..4]` LE, where
   `suffix` is the 16-byte string
   `\x67\x59\x65\x99\x04\x25\x04\x90\x56\x64\x27\x49\x94\x89\x74\x1A`
   from `rpcs3/Emu/Cell/PPUModule.cpp:55`. One-liner:
   ```python
   import hashlib
   suffix = bytes([0x67,0x59,0x65,0x99,0x04,0x25,0x04,0x90,
                   0x56,0x64,0x27,0x49,0x94,0x89,0x74,0x1A])
   for n in NAMES:
       nid = int.from_bytes(
           hashlib.sha1(n.encode()+suffix).digest()[:4], 'little')
       print(f"0x{nid:08x} {n}")
   ```
   **DO NOT GUESS NIDs.** Always hash + verify.
6. **Implement handlers** in `rust/rpcs3-emu-core/src/lib.rs` —
   look for the existing `0xe315a0b2 => { ... }` arm and add new
   ones in the same pattern. Mirror RPCS3 `cellGcmSys.cpp`
   semantics exactly — particularly for OUT pointer parameters
   (write the result to `*r4` etc. before returning). Functions
   that only return status can usually stay as silent-0 returns
   (the unimplemented-import fast-path).
7. **Run gate + commit two-stage**:
   - commit 1 = code slice: `rust/...`, `behavior-freeze/...`,
     `.planning/R13_CELLGCM_HLE_PLAN.md`.
   - commit 2 = `docs/PROJECT_STATUS.md` sync.
8. **Save memory** at
   `~/.claude/projects/.../memory/project_r13_<n>_<slug>.md` and
   add a one-line index entry to `MEMORY.md`.
9. **Update sentinel** `.planning/.loop_active` if continuing the
   loop. Remove it to end the loop cleanly.

## Key facts to keep in your head

- **CWD gotcha:** session opens in
  `c:\Users\manod\Downloads\Emulador Ps2, ps1 e ps3 nativos\`
  (parent); the real root is `rpcs3-master/`. Scripts expect
  `rpcs3-master/` as pwd.
- **Hook-blocked dirs:** `rpcs3/` (except `rpcs3/tests/`),
  `Utilities/`, `3rdparty/`, root `CMakeLists.txt`. Do NOT try to
  bypass `.claude/hooks/pretool-block-legacy.sh`.
- **R: drive is OBSOLETE for Docker mounts.** Old fixtures used
  `subst R: <path>` then `-v "R:\sub:/work"`; that broke at Docker
  29.5. Always use the literal Windows path quoted, with
  `MSYS_NO_PATHCONV=1` under Git Bash.
- **PowerShell vs Bash for Docker:** `MSYS_NO_PATHCONV=1` prefix is
  Bash-only (it suppresses MSYS path translation). In PowerShell,
  drop the prefix entirely.
- **Docker daemon instability:** flipped to hung state twice this
  session. Recovery = Quit Docker Desktop via tray → relaunch from
  Start menu → accept UAC → wait for tray icon green (~30–60s).
- **EmuCore key fields** (`pub` on `EmuCore`):
  `gcm_context_addr` (0x30000000), `gcm_control_addr` (0x30000040),
  `gcm_io_address` (= the homebrew's host_buffer guest addr),
  `gcm_io_size` (= ioSize passed to `rsxInit`).
- **CellGcmContextData layout** (`GCM.h:26`, 4 BE u32):
  `begin` / `end` / `current` / `callback` at offsets 0/4/8/12.
- **CellGcmControl layout** (`GCM.h:5`, 3 BE u32):
  `put` / `get` / `ref` at 0/4/8.

## Useful one-liners

```bash
# Full workspace gate (the canonical sign-off)
cd rust && cargo test --workspace --tests --release

# Single fixture test (fastest iteration)
cd rust && cargo test -p rpcs3-emu-core --test rsx_gcm_flip --release -- --nocapture

# Probe a faulting fixture
cd rust && cargo test -p rpcs3-emu-core --test rsx_setdisplay_probe --release -- --nocapture

# Hash a NID
python3 -c "import hashlib; suffix=bytes([0x67,0x59,0x65,0x99,0x04,0x25,0x04,0x90,0x56,0x64,0x27,0x49,0x94,0x89,0x74,0x1A]); print(hex(int.from_bytes(hashlib.sha1(b'cellGcmFOO'+suffix).digest()[:4],'little')))"
```

## Files that matter most for R13.5+

- `rust/rpcs3-emu-core/src/lib.rs` — the cellGcmSys NID match arm
  (search for `0x15bae46b =>` to find the block).
- `rust/rpcs3-rsx-state/src/lib.rs` — `RsxState`, `RsxSnapshot`,
  `replay_gcm`, descriptor structs (`VertexAttribute`, `IndexArray`,
  `TextureDescriptor`, `SurfaceDescriptor`), `DrawTracker`.
- `rust/rpcs3-emu-core/tests/rsx_gcm_*.rs` — capture test patterns.
- `rpcs3/Emu/Cell/Modules/cellGcmSys.cpp` — RPCS3 reference for
  cellGcm semantics. **No guessing.**
- `rpcs3/Emu/RSX/GCM.h` — `CellGcmContextData` / `CellGcmControl`
  / `CellGcmConfig` layouts.

## Memory pointers (auto-memory at `~/.claude/projects/.../memory/`)

- `project_r13_1_cellgcm_init_unblock.md` — R13.1 detail.
- `project_r13_2_real_libgcm_capture.md` — R13.2 detail.
- `project_r13_3_real_draw_capture.md` — R13.3 detail.
- `project_r13_4_full_flip_path.md` — R13.4 detail (this slice).
- `MEMORY.md` — chronological index of everything.
