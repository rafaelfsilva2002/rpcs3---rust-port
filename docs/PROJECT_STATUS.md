# Project Status — R10 CLOSED library layer architecturally complete (LV2 sync primitives — Lv2SyncState owns 8 of 9 primitive families; 109 lib tests; 264 release blocks; 0 regression; 20 SPU oracles intact)

**Authoritative current source of truth for the RPCS3 → Rust port.**

Last updated: **2026-05-26 (R10 CLOSED library layer architecturally complete)**.

R10 wave ported the LV2 sync primitive family into a single
per-`EmuCore` registry (`rpcs3-lv2-sync::Lv2SyncState`):

- **R10.1.a-c** lwmutex (handle pool + Lv2SyncId/Kind/State + 3
  PSL1GHT NIDs wired in EmuCore + typed LwMutexAttribute parser).
- **R10.2+R10.4** kernel sys_mutex + sys_semaphore via SyncTable.
- **R10.3** sys_cond via CondRegistry (mutex-tied atomic release/
  reacquire).
- **R10.5** sys_event_flag via EventFlagRegistry (64-bit bitmask +
  AND/OR + CLEAR/CLEAR_ALL modes).
- **R10.6** sys_event_queue + sys_event_port via EventRegistry.
- **R10.7** sys_rwlock via RwlockRegistry (writer-priority).
- **R10.8** sys_lwcond DEFERRED — no `rpcs3-lv2-lwcond` crate
  exists in workspace.

Fixture / oracle / capture work remains blocked (Docker capture
pipeline offline); library layer validated via 109 lv2-sync
unit tests + 18 emu-core tests. R9 parity (264 release blocks)
preserved across all 8 R10 library commits; 20 SPU oracles
remain green.

See `.planning/R10_LV2_SYNC_CLOSURE.md` for full deliverable
inventory and roadmap. Previous wave status follows below.

---

Previously: **2026-05-25 (R9 CLOSED architecturally complete)**.
R9 wave (LV2/PPU strategic pivot) drove existing 20 CC0 SPU
oracle binaries through the full Rust integration path:
`EmuCore::run_self` parses a PSL1GHT `.self` through
SCE-header → ELF load → PPC64 FD deref → user-mode stack
alloc → TLS init (Linux ELFv1 +0x7000 TP bias) →
sys_process_param + sys_proc_prx_param parse → libstub walk +
import-stub trampoline install + addrs[] population → PPC
interpreter coverage expanded with ~25 new opcodes → lv2
syscall dispatcher with 7 specific arms + permissive catch-all
+ 20 NID-specific import handlers (including stdio family with
mini_printf + the full sys_spu_* lifecycle with REAL SPU
execution via spu-interpreter). PSL1GHT main() runs through
every SPU lifecycle syscall to clean exit; SPU oracle stack
remains untouched. 264 cargo test result blocks pass, ZERO
regression across all 22 R9 commits.

**Honest scope note:** End-user TTY emit from PSL1GHT main()'s
printf path is NOT delivered. R9.1l → R9.1n proved the
constructor chain executes and the `__syscalls` table is
populated, but newlib's `_reent._write_r` linkage is a
separate static-newlib mechanism not exposed by public
PSL1GHT sources. Reaching emitted TTY requires either
(B) a newlib-binding investigation wave (~1-2 sessions) or
(C) a main() bypass via prologue pattern matching. User
selected **Option A — pivot to other subsystems** on
2026-05-25.

See § 11 below for the R9 closure details and
`.planning/R9_FINAL_CLOSURE.md` for the full narrative.

Previously: **2026-05-23 (R8.5e E.6 landing — 20th oracle)**.
R8.5e closes the list-DMA stall-and-notify wave by promoting
`single_spu_dma_putl_stall_v1` to the 20th replay-validated
oracle — the symmetric inverse of R8.5d D.6
`single_spu_dma_getl_stall_v1` (19th). The PUTL stall test
passed on first run because R8.5d D.6's
`initial_mfc_list_stall_mask` plumbing (SpuProgram →
DmaPreReplayPlan → spu-thread) was already direction-agnostic;
the `is_putl=true` branch in `process_mfc_list_cmd` (R8.5c)
and the `is_put=true` branch in
`bridge_dma_list_stall_ack_callback` (R8.5d D.2) were exercised
end-to-end against real-captured JSONL with zero new code.
**Full LS↔EA list-DMA stall coverage** — 8 list-DMA oracles
total across the 6-code family + GETL stall + PUTL stall.
FIXED OUT_MBOX sentinel 0xC0FFEEC3 (vs PPU-computed
ea_status=0xA12FDC1E). No SHA bumps in E.6 (replay test only).
Bridge default OFF unchanged; behavior-freeze contract
preserved. Workspace warnings cleaned: 101 → 84 (zero
dead-code; remainder is fn-pointer comparison correctness in
rpcs3-spu-thread, separate work). spu-runner non-exhaustive
match on `MfcUnsupported` fixed. See § R8.5e at the bottom +
§ 9 for the refreshed roadmap.

Previously: **2026-05-23 (R8.5d D.6 landing — 19th oracle)**.
R8.5d D.6 closes the GETL half by promoting
`single_spu_dma_getl_stall_v1` to the 19th oracle. Fixed an
R8.5c plumbing gap (`initial_mfc_list_stall_mask` was not
threaded through SpuProgram into spu-thread — captured ch25
value never reached SPU runtime; replay-replays diverged at
the ch25 read). Triple-symmetric acceptance: replay + bridge
runtime (R8.5d D.1.b GETL + D.2 PUTL extension) + bridge OFF
(C++ executor unchanged).

Previously: **2026-05-23 (R8.5d D.2 landing — PUTL bridge stall)**.
R8.5d D.2 unifies the C++ bridge stall handshake to handle both
GETL and PUTL via `ListPartialState{is_put: bool}` (renamed
from `GetlPartialState`). Restructures
`bridge_dma_putl_callback` to the Cell BE Sec. 12.5
transfer-then-stall pattern (mirror of D.1.b GETL). Single
`bridge_dma_list_stall_ack_callback` dispatches resume direction
via `partial.is_put`. No Rust changes (replay R8.5c was
already direction-agnostic; FFI D.1.a callback is
direction-agnostic). Bridge SHA `19a81b5452…`; rpcs3.exe
`57D3746A…`.

Previously: **2026-05-23 (R8.5d D.1.a + D.1.b landing — bridge
runtime stall, GETL-only)**. Added Rust FFI scaffolding for ch25
destructive read + ch26 ack callback (`MFC_RD_LIST_STALL_STAT`
const, `MFC_WR_LIST_STALL_ACK` const, `DmaListStallAckCallback`,
`RUST_SPU_DMA_LIST_STALL_PENDING=-2` sentinel) + C++ bridge
stall handshake (`GetlPartialState`, `bridge_save/take_getl_partial`
helpers, `bridge_dma_list_stall_ack_callback`). Scope intentionally
GETL-only to keep risk small; D.2 generalizes to PUTL.

Previously: **2026-05-23 (R8.5c landing — Rust replay handshake)**.
Rust-side state machine for list-DMA stall-and-notify in
`process_mfc_list_cmd` (enforces Cell BE Sec. 12.5
transfer-then-stall: stalled element transferred BEFORE stall
bit raised; ACK resumes at `next_element_index`).
`ListDmaPartialProgress` resume state; 4 new error variants;
7 synthetic tests including `stalled_element_copied_before_stall_not_twice`.

Previously: **2026-05-22 (R8.5b landing — capture surface unlock)**.
Schema A: reused existing `SpuRdch ch=25` / `SpuWrch ch=26`
events (no new EventKind). C++ writer no longer rejects ch25/ch26
on target_spu match; Rust parser no longer triggers
UnsupportedChannel. Writer + parser only — no replay, no bridge.

Previously: **2026-05-21 (R8.4f-b landing)**. R8.4f-b adds
MFC PUTLB (cmd=0x25, PUTL + barrier) and PUTLF (cmd=0x26,
PUTL + fence) end-to-end in a single phase. Same REUSE-PUT
strategy as R8.4f-a's REUSE-GET: per RPCS3 `do_list_transfer`
the barrier/fence bits are stripped before the per-element
copy, so the data path is byte-identical to plain PUTL. Two
new replay-validated oracles (`single_spu_dma_putlb_v1`,
ea_status `0xA12FDA3B`; `single_spu_dma_putlf_v1`, ea_status
`0xA12FDA7F`). Bridge ON delegates both with
`DELEGATED EXECUTION OK total_steps=1394` (identical to PUTL),
no fallback; all 12 triple-symmetry fixtures green. **The
entire 6-code list-DMA family (GETL/GETLB/GETLF/PUTL/PUTLB/
PUTLF) is now end-to-end supported** — `MFC_LIST_CMDS_UNSUPPORTED`
in the parser is now an empty slice. All 3 patch SHAs bumped:
scaffolding `5085c4af…` → `d9d60bfa01a942c0523ac4ae5f8307c9bd89c57efc0736b432dc1e38db1d482c`,
runtime hooks `67bef045…` → `e53518c4393e416d08ad09257ddf0af9c92ff7011a3f0524ff1db9c70593519e`,
bridge `b9e5e977…` → `106ddede745c6487e3b1f4dbe61c272beb3c16835c164a952a0799ed4de3e899`.
rpcs3.exe rebuilt: `f3d4e85f…` →
`85e6fe8d09f7ae02d0cc258f8087a0eb46ab25bde3c66e8eda0050682626f428`.
ZERO new `.dmachunk` / `.dmalistdesc` (perfect pool dedup);
TWO new `.spuimg`. Stall-and-notify (bit 0x80) defers R8.5+.
See § 8.4f-b.

Previously: **2026-05-21 (R8.4f-a landing)**. R8.4f-a adds
MFC GETLB (cmd=0x45, GETL + barrier) and GETLF (cmd=0x46,
GETL + fence) end-to-end in a single phase. Per RPCS3
`do_list_transfer` (`SPUThread.cpp:2887`), barrier/fence bits
are stripped before the per-element copy
(`transfer.cmd = args.cmd & ~0xf`), so the data path is
byte-identical to plain GETL. Per `do_dma_check` ordering
effects on `mfc_barrier`/`mfc_fence` don't surface in
single-SPU fresh-tag fixtures. **Strategy:** REUSE GETL
semantics across parser, replay, and runtime bridge — only
the cmd-code acceptance lists changed; no new callback, no
new state machine path. Two new replay-validated oracles
(`single_spu_dma_getlb_v1`, status `0xDF1EEA3B`;
`single_spu_dma_getlf_v1`, status `0xDF1EEA7F`); bridge ON
delegates both with `DELEGATED EXECUTION OK total_steps=1598`
(identical to GETL), no fallback; all 10 triple-symmetry
fixtures green. All 3 patch SHAs bumped: scaffolding
`402c2d13…` → `5085c4afaa5dd2df7526999b7f7f0ed33b763ce4c66d4decef55a2fa2b427364`,
runtime hooks `3760b78c…` →
`67bef0455eeedc511443c7d283841fd5080d703dac0b8bc11743b97a971a3dc8`,
bridge `e09b9c40…` →
`b9e5e977bc3f97b5e1a86f56a5d6affd79d831f3c9f4b47226511a242a45a713`.
rpcs3.exe rebuilt: `64ff57a1…` →
`f3d4e85f3d2e375bb9d58e8414a3e2f9699c3a25a6210eba998d3a869ee665ac`.
ZERO new `.dmachunk` / `.dmalistdesc` (perfect pool dedup);
TWO new `.spuimg`. PUTLB (0x25) + PUTLF (0x26) defer R8.4f-b;
stall-and-notify (bit 0x80) defers R8.5+.

Previously: **2026-05-21 (R8.4e landing)**. R8.4e closes
the R8.4 list-DMA closing-half by landing PUTL (cmd=0x24,
LS → EA) end-to-end in a single phase: writer + capture +
replay + runtime bridge + triple-symmetry, no engine fixes
required (R8.4c/d's plumbing already supports the symmetric
inverse). New 14th replay-validated oracle
`single_spu_dma_putl_v1`; bridge ON delegates end-to-end with
`DELEGATED EXECUTION OK total_steps=1394`; triple-symmetry
`--fixture put_list` green; all 8 fixtures (get / put /
get_multi / get_any / get_tag_poll / get_tag_immediate /
get_list / put_list) green. The new `bridge_dma_putl_callback`
mirrors `bridge_dma_getl_callback` but uses
`const uint8_t* src_ls_ptr` because PUTL never mutates LS.
All 3 patch SHAs bumped: scaffolding `5c170508…` →
`402c2d139526a4efd592ba6f052f0c59067aaf09b9d079d75d03ca4a09fe4e5a`,
runtime hooks `745945f4…` →
`3760b78c8854dd83157f5ef5e501ae85b4fd9b46dc143ae226fb19703bf4a974`,
bridge `d2d531850f…` →
`e09b9c40b3187f89b559c5fcde949a86491974c836525338d51dd2e99600850e`.
rpcs3.exe rebuilt: `0f5cc2bec9…` →
`64ff57a1248ebb857fcffda2ff392fffa432deb7f0dd75deb07cbb670152cd33`.
ZERO new `.dmachunk` (perfect pool dedup with prior 13
oracles); ZERO new `.dmalistdesc` (lucky EA layout dedup
with R8.4b GETL); ONE new `.spuimg`. PUTLB / PUTLF / GETLB /
GETLF / stall-and-notify defer R8.4f / R8.5+.
See "R8.4e phase closure (2026-05-21)" below.

Previously: **2026-05-21 (R8.4d landing)**. R8.4d completed
the R8.4 GETL roadmap by adding the runtime bridge half: the
Rust SPU bridge now delegates `single_spu_dma_getl_v1.self`
end-to-end with `RPCS3_SPU_RUST_BRIDGE=1`. New C ABI
`rust_spu_set_dma_getl_callback` plus a 7-arg
`bridge_dma_getl_callback` in `SPURustBridge.cpp` walks the
descriptor list in SPU LS, per element memcpys `ts` bytes
from `vm::_ptr<u8>(ea)` to `lsa_dest_base + cumulative_offset`,
queues the tag-stat atomically. Stall-and-notify (`sb & 0x80`)
+ PUTL + GETLB/GETLF are rejected so the bridge falls back
honestly to C++. Triple-symmetry gate extended with the
`get_list` fixture; 7 fixtures green (`total_steps=1598`
for GETL with `DELEGATED EXECUTION OK`, no fallback).
Bridge patch SHA bumped:
`0afda1c69…` → `d2d531850f4743b240fb59573695ea247614d0aeecb8624726206937be52d2e5`.
rpcs3.exe rebuilt: `3f2348de…` → `0f5cc2bec90986da7aa1eb9d601230c8986563a92e5027ac991f5742507b749d`.
See "R8.4d phase closure (2026-05-21)" below.

Previously: **2026-05-21 (R8.4c landing)**. R8.4c promotes
the R8.4b capture-only GETL fixture to the 13th replay-
validated oracle. New `MfcReplayState::process_mfc_list_cmd`
walks the captured `.dmalistdesc` descriptor, parses each
8-byte BE slot, cross-validates per-element ts/ea against
trace event's `element_sizes`/`element_eals`, loads each
chunk from the existing `.dmachunk` pool, copies bytes into
LS at cumulative offsets. R8.4a canary lifted for cmd=0x44
only (GETLB/GETLF + PUTL family still rejected). New
`resolve_dma_listdesc_side_file` mirrors the chunk loader
for the `.dmalistdesc` extension. Replay test
`r8_4c_single_spu_dma_getl_v1_replay_validated_byte_identical`
proves cross-backend byte-identical with canonical
`0xDF1EEA5A`. Rust-core only fix; no C++ patches changed;
rpcs3.exe unchanged. R8.4d adds the runtime bridge GETL
callback (now landed — see § R8.4d above). See "R8.4c phase closure
(2026-05-21)" below.

Previously: **2026-05-21 (R8.4b landing)**. R8.4b lands the
first half of MFC GETL list-DMA support: C++ writer extension
(SPUThread.cpp + SPUTraceJsonl.{h,cpp}), real CC0 GETL
fixture captured (15-event JSONL + new `.dmalistdesc`
side-file + 2 `.dmachunk` files that dedup with the
existing pool), Rust parser additive fields
(`SpuMfcCmdEvent::{descriptor_sha256, descriptor_size,
element_chunks, element_sizes, element_eals}` all `Option<>`).
The trace is a **capture-only sentinel** at R8.4b — the
replay state machine still rejects with
`UnsupportedMfcListCmd` (R8.4a canary preserved). R8.4c
lifts the canary AND adds the state machine; R8.4d adds
the runtime bridge GETL callback + promotes the 13th
oracle. Two patches bumped: scaffolding (added
`record_spu_mfc_getl_cmd` + `write_dma_listdesc_side_file`)
and runtime hooks (SPUThread.cpp ch21 dispatch detects GETL,
walks descriptor, captures per-element EA snapshots).
rpcs3.exe rebuilt: `34ec50d7…` → `3f2348de…`. See "R8.4b
phase closure (2026-05-21)" below.

Previously: **2026-05-20 (R8.3c landing)**. R8.3c (IMMEDIATE
+ overlapping masks + 12th oracle) is the THIRD consecutive
fixture to surface and co-fix a real divergence. The new
fixture `single_spu_dma_tag_immediate_v1` mirrors R8.3b but
uses `WrTagUpdate = IMMEDIATE` (= 0) and overlapping masks
(0x08 ⊂ 0x28). The captured `ts2 = 0x28` proved IMMEDIATE
does NOT clear `completed_tags` (Cell BE canonical). The
fixture surfaced a latent clear-on-read in
`MfcReplayState::process_rdch_tagstat` (legacy from R6.7 A.4
that R8.3b's `SpuChannels::read` fix had already aligned on
the runtime side). Fix removes the matching clear in the
state machine oracle validator. rpcs3.exe does NOT need
rebuild — the fix is in the replay-only layer. All 6
triple-symmetry fixtures green. See "R8.3c closure
(2026-05-20)" below.

Previously: **2026-05-20 (R8.3b landing)**. R8.3b (repeated-
RdTagStat polling + 11th oracle) is the SECOND fixture in a row
that surfaced a real Rust runtime / C++ executor divergence and
co-fixed it. The new fixture `single_spu_dma_tag_poll_v1`
mirrors R8.3a multi-DMA but the SPU performs TWO ch24 reads in
the same session with distinct masks (0x08 then 0x20). The
drain-clear semantic from R8.3a stalled the second read; fix:
added `completed_tags: u32` field on `SpuChannels`, persistent
across reads (mirroring Cell BE / C++ behavior). ch24 read
drains queue OR-into `completed_tags`, returns
`completed_tags & mfc_wr_tag_mask` WITHOUT clearing. All 5
triple-symmetry fixtures green post-fix. See "R8.3b closure
(2026-05-20)" below.

Previously: **2026-05-20 (R8.3a landing)**. R8.3a (ANY wait
mode + 10th oracle) is the first fixture that surfaced a real
divergence between the Rust runtime and the C++ executor — and
the divergence was co-fixed in the same delivery. The new
fixture `single_spu_dma_get_any_v1` mirrors R8.2 multi-DMA
(two queued GETs, tags 3 + 5, distinct EAs / sizes / LSAs) but
uses `WrTagUpdate = ANY` (= 1) and embeds the actual ch24
returned value into the canonical status via `(tag_stat << 24)
^ 0xBEEFBEAD`. Bridge ON first attempt produced
`status = 0xA92FAE2D` instead of canonical `0x892FAE2D` because
the Rust `SpuChannels::read(MFC_RD_TAG_STAT)` was popping single-
tag bits one at a time (only saw `0x08`), while C++ returns
`completed_tags & wr_tag_mask` aggregate (`0x28`). Fix landed
in `rpcs3-spu-thread`: read now drains the queue via bitwise
OR + intersects with `mfc_wr_tag_mask`. rpcs3.exe rebuilt to
relink. All 4 fixtures (GET / PUT / GET_MULTI / GET_ANY) green
post-fix. See "R8.3a closure (2026-05-20)" below.

Previously: **2026-05-20 (R8.2 landing)**. R8.2 (multi-DMA GET
+ ALL wait + 9th oracle) extends R8.1's GET+PUT triple-symmetric
DMA bridge with multi-tag in-flight semantics. The new fixture
`single_spu_dma_get_multi_v1` exercises two queued GETs (tags
3 + 5, distinct EAs / sizes / LSAs) + `WrTagMask = 0x28` +
`WrTagUpdate = ALL` + `RdTagStat = 0x28`. **Zero code changes
landed** — the 8-oracle baseline (parser, state machine, chunk
loader, executor wiring, runtime bridge callback) already
supported everything R8.2 exercises. The bridge ON delegation
log shows `total_steps=1584` (vs 1054 for single-GET, 1049 for
PUT), confirming both DMAs traversed the Rust executor end to
end via the same R7.2 callback invoked twice. See "R8.2 phase
closure (2026-05-20)" below.

Previously: **2026-05-19 (R8.1 landing)**. R8.1 (MFC PUT runtime
+ replay oracle) extends R7's triple-symmetric DMA bridge with the
inverse direction (LS → EA). The runtime bridge now executes BOTH
DMA-bound oracles end-to-end through the Rust executor under
`RPCS3_SPU_RUST_BRIDGE=1` with byte-identical canonical outputs:
GET = `0xdeada12f`, PUT spu sentinel = `0xc0ffeeca` + ea_status =
`0xcafea57e`. The replay oracle for `single_spu_dma_put_v1` is the
8th oracle and gates byte-identical Interpreter ↔ Recompiler
agreement plus a post-replay verification that final LS at the PUT
region matches the captured `.dmachunk`. See "R8.1 phase closure
(2026-05-19)" below.

Previously: **2026-05-18 (R7 closure)**. R7 phases R7.1 (Bridge
Phase B honest fallback) + R7.2 (Bridge Phase D runtime DMA GET) +
R7.3 (triple-symmetry regression gate) all ACEITO. The runtime
bridge now executes the first DMA-bound oracle
`single_spu_dma_get_v1.self` end-to-end through the Rust executor
under `RPCS3_SPU_RUST_BRIDGE=1`, with byte-identical output
(`0xdeada12f`) versus bridge OFF and versus the replay oracle. See
"R7 phase closure (2026-05-18)" below.

Previously: **2026-05-03 (R6 closure)**. The text below describes the
current state as of R6 closure on that date. Long-form R5/R4 history
(R5.9e.7 / R5.11 / R5.11b material and the iteration-by-iteration
R5.4a..p timeline) has been moved verbatim to
[`docs/history/PROJECT_STATUS_R5_ARCHIVE.md`](./history/PROJECT_STATUS_R5_ARCHIVE.md).
Do NOT treat the archive as current.

---

## 1. Executive current status

- **R8.4f-b is LANDED** (2026-05-21). PUTLB (cmd=0x25,
  list+barrier) + PUTLF (cmd=0x26, list+fence) end-to-end
  in one cycle. **Closes the 6-code list-DMA family**:
  GETL/GETLB/GETLF + PUTL/PUTLB/PUTLF all replay-validated +
  bridge-delegated. Same REUSE-PUTL strategy as R8.4f-a's
  REUSE-GETL: per RPCS3 `do_list_transfer` barrier/fence
  bits are stripped before per-element copy. Two new
  oracles: `single_spu_dma_putlb_v1` (SPU sentinel
  `0xC0FFEEBB` + ea_status `0xA12FDA3B`) and
  `single_spu_dma_putlf_v1` (`0xC0FFEEBF` + `0xA12FDA7F`).
  Bridge ON delegates both end-to-end with
  `total_steps=1394` (identical to PUTL). All 12
  triple-symmetry fixtures green (added put_list_b,
  put_list_f). `MFC_LIST_CMDS_UNSUPPORTED` parser slice is
  now empty. All 3 patch SHAs bumped: scaffolding
  `5085c4af…` → `d9d60bfa…`, runtime hooks `67bef045…` →
  `e53518c4…`, bridge `b9e5e977…` → `106ddede…`. rpcs3.exe
  rebuilt `f3d4e85f…` → `85e6fe8d…`. ZERO new `.dmachunk` /
  `.dmalistdesc`; TWO new `.spuimg`. See § 8.4f-b.
- **R8.4f-a is LANDED** (2026-05-21). GETLB (cmd=0x45,
  list+barrier) + GETLF (cmd=0x46, list+fence) end-to-end in
  one cycle: writer + capture + replay + runtime bridge +
  triple-symmetry. **Strategy: REUSE GETL semantics across
  all layers** — per RPCS3 `do_list_transfer` the
  barrier/fence bits are stripped before the per-element copy
  (`transfer.cmd = args.cmd & ~0xf`), so the data path is
  byte-identical to GETL; per `do_dma_check` ordering effects
  don't surface in single-SPU fresh-tag fixtures. Two new
  replay-validated oracles: `single_spu_dma_getlb_v1`
  (canonical status `0xDF1EEA3B`, mask `0xC0DEFABB` last byte
  `0xBB` = Barrier mnemonic) and `single_spu_dma_getlf_v1`
  (canonical status `0xDF1EEA7F`, mask `0xC0DEFAFF` last
  byte `0xFF` = Fence mnemonic). Bridge ON delegates both
  end-to-end with `total_steps=1598` (identical to GETL),
  no fallback. All 10 triple-symmetry fixtures green
  (get / put / get_multi / get_any / get_tag_poll /
  get_tag_immediate / get_list / put_list / get_list_b /
  get_list_f). ZERO new `.dmachunk` / `.dmalistdesc` (perfect
  pool dedup with R8.4b+); TWO new `.spuimg`. All 3 patch
  SHAs bumped: scaffolding `402c2d13…` → `5085c4af…`, runtime
  hooks `3760b78c…` → `67bef045…`, bridge `e09b9c40…` →
  `b9e5e977…`. rpcs3.exe rebuilt `64ff57a1…` → `f3d4e85f…`.
  See § 8.4f-a.
- **R8.4e is LANDED** (2026-05-21). Runtime bridge PUTL
  + 14th replay-validated oracle. Symmetric inverse of
  R8.4d GETL: writer + capture + replay + runtime bridge
  + triple-symmetry all in one cycle (no engine fixes —
  R8.4c/d's plumbing already covers PUTL). New 14th
  oracle `single_spu_dma_putl_v1`; bridge ON delegates
  end-to-end (`DELEGATED EXECUTION OK total_steps=1394`,
  no fallback); all 8 triple-symmetry fixtures green
  (get / put / get_multi / get_any / get_tag_poll /
  get_tag_immediate / get_list / put_list). New
  `bridge_dma_putl_callback` uses `const uint8_t* src_ls_ptr`
  (PUTL only reads LS). All 3 patch SHAs bumped:
  scaffolding `5c170508…` → `402c2d13…`, runtime hooks
  `745945f4…` → `3760b78c…`, bridge `d2d531850f…` →
  `e09b9c40…`. rpcs3.exe rebuilt `0f5cc2bec9…` →
  `64ff57a1…`. ZERO new `.dmachunk` / `.dmalistdesc`
  (perfect pool dedup). See § 8.4e.
- **R8.4d is LANDED** (2026-05-21). Runtime bridge GETL
  + triple-symmetry expansion completes the R8.4 GETL
  roadmap. New C ABI `rust_spu_set_dma_getl_callback` +
  C++ `bridge_dma_getl_callback` (7-arg) walks the
  descriptor in SPU LS and per element memcpys from
  `vm::_ptr<u8>(ea)` to `lsa_dest_base +
  cumulative_offset`. Tag-stat queued atomically.
  Stall-and-notify (`sb & 0x80`) + PUTL + GETLB/GETLF
  rejected (bridge falls back honestly). Bridge ON
  delegates `single_spu_dma_getl_v1.self` end-to-end
  (`DELEGATED EXECUTION OK total_steps=1598`, no
  fallback). Bridge SHA bumped
  `0afda1c69…` → `d2d531850f…`; rpcs3.exe rebuilt
  `3f2348de…` → `0f5cc2bec90986da…`. See § 8.4d.
- **R8.4c is LANDED** (2026-05-21). 13th replay-validated
  oracle: `single_spu_dma_getl_v1` (promoted from R8.4b
  capture-only). New `MfcReplayState::process_mfc_list_cmd`
  for cmd=0x44; new `.dmalistdesc` side-file resolver in
  `dma_chunk.rs`; R8.4a canary lifted only for GETL.
  Rust-core only fix; C++ patches unchanged; rpcs3.exe
  unchanged. Replay test passes cross-backend
  byte-identical with canonical `0xDF1EEA5A`. See § 8.4c.
- **R8.4b is LANDED** (2026-05-21). First half of MFC GETL
  list-DMA support: C++ writer captures GETL (cmd=0x44) with
  new `.dmalistdesc` side-file + per-element `.dmachunk` files
  (REUSE existing pool — dedup works). New `SpuMfcCmdEvent`
  Option<> fields (descriptor_sha256 + 4 list fields) parse
  cleanly. Replay state machine still rejects via R8.4a's
  `UnsupportedMfcListCmd` canary (lifted in R8.4c). Two
  patches bumped (scaffolding + runtime hooks); bridge SHA
  unchanged (no runtime delegation yet). rpcs3.exe rebuilt
  (`3f2348de…`). Captured `single_spu_dma_getl_v1.jsonl` is
  the 13th oracle target — replay test lands in R8.4c. See
  § 8.4b.
- **R8.3c is LANDED** (2026-05-20). First IMMEDIATE-wait-mode
  oracle + 12th oracle. Two queued GETs + TWO ch24 reads with
  `WrTagUpdate = IMMEDIATE` (= 0) and overlapping masks
  (0x08, 0x28). Captured `ts2 = 0x28` (full mask after first
  read of subset mask 0x08) proves IMMEDIATE does NOT clear
  `completed_tags`. Pre-fix prediction: replay test surfaces
  `TagStatMismatch{captured:0x28, oracle:0x20}` because
  `MfcReplayState::process_rdch_tagstat` was clearing the
  observed bit (legacy from R6.7 A.4, latent until overlapping
  masks). Engine fix: remove the clear; state machine now
  aligns with R8.3b's persistent `SpuChannels::read` semantic.
  Fix is replay-layer only; rpcs3.exe does NOT need rebuild.
  All 6 triple-symmetry fixtures green. Canonical
  `0xDD164A9E`.
- **R8.3b is LANDED** (2026-05-20). First repeated-RdTagStat
  polling oracle + 11th oracle. Two queued GETs + TWO ch24
  reads (masks 0x08, 0x20, both ANY) in the same session.
  Both reads must see persistent `completed_tags` bits, with
  per-mask AND filtering. Pre-fix prediction: replay parked
  at pc=140 channel 24 (queue drained); bridge ON stall
  fallback to C++ at total_steps=36. Engine fix: added
  `completed_tags: u32` on `SpuChannels`, persistent across
  reads. Fix is single-file Rust core; C++ patches unchanged.
  rpcs3.exe rebuilt twice (cargo cache gotcha — first build
  used stale lib; touch + rebuild forced fresh). All 5
  triple-symmetry fixtures green. Canonical status
  `0xDD1EAA5C` proven across bridge OFF / bridge ON / replay.
- **R8.3a is LANDED** (2026-05-20). First ANY-wait-mode replay
  oracle + 10th oracle. Same shape as R8.2 multi but with
  `WrTagUpdate = ANY` (= 1) and tag_stat embedded in canonical
  status (`(tag_stat << 24) ^ 0xBEEFBEAD`). Canonical status
  `0x892FAE2D` (captured RPCS3 ch24 = 0x28). Bridge ON first
  attempt diverged (`0xA92FAE2D` ≠ `0x892FAE2D`) — the Rust
  `SpuChannels::read(MFC_RD_TAG_STAT)` was popping single-tag
  bits one at a time, surfacing `0x08` instead of the
  aggregated `0x28`. **Engine fix landed**: ch24 read now
  drains the queue via bitwise OR + intersects with
  `mfc_wr_tag_mask`. rpcs3.exe rebuilt. All 4 triple-symmetry
  fixtures green post-fix. See § 8.3a.
- **R8.2 is LANDED** (2026-05-20). First multi-DMA replay oracle
  + 9th oracle. Two queued GETs (tags 3 + 5) + ALL wait + 0x28
  RdTagStat + canonical status `0xE12DEA4E`. **Zero engine-side
  code changes**: the 8-oracle baseline (parser, state machine,
  loader, executor, bridge callbacks) already covered multi-tag
  in-flight + ALL wait semantics; R8.2 is a pure coverage gain
  on the existing implementation. Triple-symmetry green via
  `check_triple_symmetry.py --fixture get_multi`. See § 8.2.
- **R8.1 is LANDED** (2026-05-19). MFC PUT runtime extension +
  8th replay-validated oracle. The runtime bridge now delegates
  BOTH `single_spu_dma_get_v1.self` (cmd 0x40, R7.2) AND
  `single_spu_dma_put_v1.self` (cmd 0x20, R8.1) end-to-end through
  the Rust executor with byte-identical canonical outputs
  (`0xdeada12f` / `0xcafea57e`) versus bridge OFF and the replay
  oracles. See § 8.1 R8.1 closure summary.
- **R7 is formally CLOSED.** R7.1 (Bridge Phase B honest fallback)
  + R7.2 (Bridge Phase D runtime DMA GET via FFI callback into
  `vm::_ptr<u8>`) + R7.3 (triple-symmetry regression gate) all
  ACEITO 2026-05-18.
- **R6 is formally CLOSED** (2026-05-03). R6.2 first delegation →
  R6.3a/b/c per-oracle delegation → R6.4a outcome contract → R6.4b
  persistent handle → R6.5 / R6.5b bridge acceptance → R6.6 game_like
  cross-path → R6.7 design + A.1-A.5 + Phase C — all landed and
  gated green.
- **Eighteen replay-validated SPU oracles exist**, all
  `diff_snapshots(interp, jit).is_identical()` byte-identical
  across `InterpreterExecutor` and `RecompilerExecutor`. The
  13th-18th oracles cover the entire MFC list-DMA 6-code
  family: GETL (R8.4c) + PUTL (R8.4e) + GETLB/GETLF
  (R8.4f-a) + PUTLB/PUTLF (R8.4f-b). All barrier/fence
  variants reuse their base GETL/PUTL data path per RPCS3
  `do_list_transfer` (bits stripped before per-element
  copy).
- **`single_spu_dma_put_v1` is the 8th oracle** (R8.1 landed
  2026-05-19; runtime-delegated under bridge ON same day).
  Symmetric inverse of GET: SPU fills LS with `i & 0xFF` counting
  pattern, dispatches MFC PUT cmd 0x20, waits via ch22/23/24,
  writes sentinel `0xC0FFEECA` to OUT_MBOX, halts via stop 0x101.
  PPU reads EA back, computes `ea_status = sum_of_ea ^ 0xCAFEBABE
  = 0xCAFEA57E` — both invariants land identically across bridge
  OFF / bridge ON / replay paths.
- **`single_spu_dma_get_v1` is the first DMA-bound oracle**
  (R6.7 A.5, replay-validated 2026-05-03; runtime-delegated under
  bridge ON 2026-05-18 via R7.2). It exercises the full MFC GET
  sequence (ch16-21 wrch + `spu_mfc_cmd` + `mfc_dma_complete` +
  ch22-23 wrch + rdch ch24) and lands the canonical post-DMA
  status `0xDEADA12F` in OUT_MBOX across all three execution paths
  (bridge OFF / bridge ON / replay).
- **DMA / MFC GET+PUT replay pipeline is complete.** Writer
  extension (A.1 GET + R8.1 PUT) + parser (A.2 + R8.1 cmd 0x20
  accept) + chunk loader (A.3) + replay state machine (A.4 +
  R8.1 `process_mfc_cmd_pre_replay`) + executor wiring (Phase C
  + R8.1 PUT callback routing) + GET oracle (A.5) + PUT oracle
  (R8.1) — all landed.
- **The C++ ↔ Rust runtime bridge covers all 8 oracles.**
  Bridge ON byte-identical to bridge OFF on the 6 non-DMA oracles
  (per R6.5 / R6.5b / R6.6 acceptance) and on both DMA oracles
  (per R7.2 + R8.1 acceptance — GET: `DELEGATED EXECUTION OK
  total_steps=1054`; PUT: `DELEGATED EXECUTION OK
  total_steps=1049`). Bridge logs distinguish the two with
  `R7.2 DMA GET dispatched` vs `R8.1 DMA PUT dispatched`.
- **Runtime DMA bridge scope: GET + simple PUT.** List cmds,
  atomic primitives, MFC barriers / fence bits, and multi-SPU
  DMA races on shared EA remain **out of R8.1 scope** — they
  defer to R8.2+. Non-(GET|PUT) MFC ops still surface
  `MfcUnsupported` and the bridge falls back honestly to C++.
- **`tests/data/spurs_test_v3_real.jsonl` and
  `tests/data/spurs_test_v4_real.jsonl` remain diagnostic-only.**
  v4 informed the ISA-coverage push (R5.10a..p) but is now retired;
  R6.7 A.5 + R7 closes the DMA cycle by delivering a fresh CC0
  oracle as the canonical first DMA-bound trace AND making it run
  through the runtime bridge. Commercial SPURS captures are not
  promoted to `behavior-freeze/`.

---

## 2. Current workspace roles

This project lives across two distinct top-level trees under the same
parent directory. **Do NOT merge them — they are complementary, not
duplicates.**

| Tree | Role |
|---|---|
| **`rpcs3-master/`** | The Rust port workspace. Contains the live `docs/` (this file), `behavior-freeze/` harness + fixtures + oracles, the entire `rust/` Cargo workspace (decoder + interpreter + recompiler + thread + differential + FFI), the C++ trace-writer + bridge patches (under `rpcs3/Emu/Cell/`), and historical snapshots. Tracked in git on branch `main`. **Source of truth for everything Rust + behavior-freeze.** |
| **`rpcs3-upstream-clean/`** | The C++ RPCS3 build / capture tree used to produce `rpcs3.exe` with the R6.7 A.1 trace hooks applied. Contains the upstream RPCS3 source + 3rd-party submodules + the MSBuild outputs (`build/lib/Release-x64/`, `bin/rpcs3.exe`). Branch `spu-trace-jsonl-runtime-hooks`. R6.7 A.1 patches are currently applied as unstaged source edits on top of upstream HEAD. **Source of truth for the rpcs3.exe binary that produces captures.** |

`rpcs3.exe` runs on Windows native (MSVC `/MT`). The PSL1GHT/ps3toolchain
side that produces `.self` binaries runs in a Docker image
(`rpcs3-ps3dev-toolchain:local`, sha `ed2167a9ac59…`, content 2.43 GB)
backed up at `C:\docker-backup\rpcs3-ps3dev-toolchain-local.tar`.

---

## 3. Current oracle matrix

All twelve oracles below pass cross-backend byte-identical
(`diff_snapshots(interp, jit).is_identical()`). Each `.jsonl` has a
companion `.notes.md` documenting provenance, toolchain, capture
procedure, engine fixes co-landed, and acceptance criteria.

| # | Fixture | Phase landed | Events | Main behavior covered | OUT_MBOX / status | DMA? | Bridge runtime status |
|---|---|---|---|---|---|---|---|
| 1 | [`single_spu_mailbox_v1`](../behavior-freeze/fixtures/spu/traces/single_spu_mailbox_v1.jsonl) | R5.9e.7 | 5 | IN_MBOX (ch29) + OUT_MBOX (ch28) + stop 0x101 | `0x129` | no | bridge ON validated |
| 2 | [`single_spu_branch_loop_v1`](../behavior-freeze/fixtures/spu/traces/single_spu_branch_loop_v1.jsonl) | R5.11 | 5 | + branch/loop ISA (Fibonacci(10)=89) | `0x59` | no | bridge ON validated |
| 3 | [`single_spu_signal_v1`](../behavior-freeze/fixtures/spu/traces/single_spu_signal_v1.jsonl) | R5.11 | 5 | SNR1 (ch3) signal-notification + OUT_MBOX + stop | `0x129` | no | bridge ON validated (R6.3c Phase 1b SNR forwarding) |
| 4 | [`single_spu_loadstore_v1`](../behavior-freeze/fixtures/spu/traces/single_spu_loadstore_v1.jsonl) | R5.11b | 5 | + LS load/store (stqd/lqd + cwd/shufb/rotqby) | `0x129` | no | bridge ON validated |
| 5 | [`single_spu_mailbox_multi_v1`](../behavior-freeze/fixtures/spu/traces/single_spu_mailbox_multi_v1.jsonl) | R6.4b-replay | 5 | IN_MBOX round 1 + SNR1 round 2 + real park/wake (PPU `sysUsleep(100ms)`) | `0x453` | no | bridge ON validated (R6.4b persistent handle + `pop_wait`) |
| 6 | [`game_like_mailbox_signal_v1`](../behavior-freeze/fixtures/spu/traces/game_like_mailbox_signal_v1.jsonl) | R6.6 | 5 | IN_MBOX + LS load/store + branch/loop + SNR1 + real park/wake (cross-path sentinel) | `0x051A03C9` | no | bridge ON validated (`total_steps=488 stall_iters=1`) |
| 7 | [`single_spu_dma_get_v1`](../behavior-freeze/fixtures/spu/traces/single_spu_dma_get_v1.jsonl) | R6.7 A.5 | 15 | MFC GET (ch16-21 wrch + `spu_mfc_cmd` + `mfc_dma_complete` + ch22-23 + rdch ch24) + post-DMA sum + XOR | `0xDEADA12F` | yes (GET 0x40) | bridge ON delegated via R7.2 (`total_steps=1054 stall_iters=0`); triple-symmetric |
| 8 | [`single_spu_dma_put_v1`](../behavior-freeze/fixtures/spu/traces/single_spu_dma_put_v1.jsonl) | R8.1 | 15 | MFC PUT (symmetric inverse of GET — LS → EA) + 128-byte LS source pattern → ch16-21 + spu_mfc_cmd cmd=0x20 + mfc_dma_complete + ch22-23 + rdch ch24 + ch28 OUT_MBOX sentinel; PPU reads EA back → `ea_status = 0xCAFEA57E` | spu=`0xC0FFEECA`, ea_status=`0xCAFEA57E` | yes (PUT 0x20) | bridge ON delegated via R8.1 (`total_steps=1049 stall_iters=0`); triple-symmetric |
| 9 | [`single_spu_dma_get_multi_v1`](../behavior-freeze/fixtures/spu/traces/single_spu_dma_get_multi_v1.jsonl) | R8.2 | 23 | TWO QUEUED MFC GETs (cmd 0x40, tags 3 + 5, distinct EAs / sizes 128 + 64 / LSAs 0x10000 + 0x10100, both in-flight before any wait) + WrTagMask=0x28 + WrTagUpdate=ALL + RdTagStat=0x28 + ch28 OUT_MBOX status; FIRST multi-DMA oracle | `0xE12DEA4E` | yes (GET 0x40 × 2) | bridge ON delegated via R7.2 callback × 2 dispatches (`total_steps=1584 stall_iters=0`); triple-symmetric; ZERO engine-side code changes |
| 10 | [`single_spu_dma_get_any_v1`](../behavior-freeze/fixtures/spu/traces/single_spu_dma_get_any_v1.jsonl) | R8.3a | 23 | TWO QUEUED MFC GETs (same shape as R8.2 multi) + WrTagUpdate=ANY (= 1) + captured RdTagStat=0x28 + ch28 OUT_MBOX status embedding tag_stat via `(tag_stat << 24) ^ 0xBEEFBEAD`; FIRST ANY-wait-mode oracle | `0x892FAE2D` | yes (GET 0x40 × 2, ANY mode) | bridge ON delegated post-fix (`total_steps=1588`); R8.3a engine fix: `MFC_RD_TAG_STAT` read drains queue + ORs + ANDs with `mfc_wr_tag_mask` |
| 11 | [`single_spu_dma_tag_poll_v1`](../behavior-freeze/fixtures/spu/traces/single_spu_dma_tag_poll_v1.jsonl) | R8.3b | 26 | TWO QUEUED MFC GETs + TWO ch24 reads in the same SPU session, distinct masks 0x08 / 0x20, both ANY mode + ch28 OUT_MBOX status embedding BOTH returned tag_stats via `((tag_stat_1 << 24) \| (tag_stat_2 << 16)) ^ 0xCAFEBADC`; FIRST repeated-RdTagStat polling oracle | `0xDD1EAA5C` | yes (GET 0x40 × 2, ANY × 2 reads) | bridge ON delegated post-fix (`total_steps=1594`); R8.3b engine fix: added persistent `completed_tags: u32` on `SpuChannels`, ch24 read drains queue + ORs into `completed_tags` + ANDs with mask, NEVER clears |
| 12 | **[`single_spu_dma_tag_immediate_v1`](../behavior-freeze/fixtures/spu/traces/single_spu_dma_tag_immediate_v1.jsonl)** | **R8.3c** | **26** | **TWO QUEUED MFC GETs + TWO ch24 reads with WrTagUpdate=IMMEDIATE (= 0), overlapping masks (0x08 ⊂ 0x28); captured `ts2=0x28` proves IMMEDIATE does NOT clear `completed_tags` (Cell BE persistent semantic) + ch28 status embedding both via `((ts1 << 24) \| (ts2 << 16)) ^ 0xCAFE5A1E`; FIRST IMMEDIATE wait-mode oracle** | **`0xDD164A9E`** | **yes (GET 0x40 × 2, IMMEDIATE × 2 reads)** | **bridge ON delegated (`total_steps=1594`); R8.3c engine fix in replay layer only: `MfcReplayState::process_rdch_tagstat` removed legacy clear-on-read (latent from R6.7 A.4), aligning with R8.3b's runtime persistent semantic. rpcs3.exe NOT rebuilt — fix is in `rpcs3-spu-differential`, not in `rpcs3_spu_ffi.lib`** |

Notes on the seventh row:

- `single_spu_dma_get_v1` is **the first** fixture to carry
  `spu_mfc_cmd` + `mfc_dma_complete` events, plus a content-addressed
  `<sha>.dmachunk` side-file at
  `behavior-freeze/fixtures/spu/dma/471fb943…2be5.dmachunk` (128 bytes,
  sum = 8128 = 0x1FC0, counting pattern 0x00..0x7F).
- **Bridge runtime status for this oracle is replay-only.** The Rust
  bridge currently has NO `process_mfc_cmd()` callback; bridge ON
  attempting to delegate a `wrch ch21` would diverge from C++ ground
  truth. R7.1 (Phase B honest-fallback) + R7.2 (Phase D runtime DMA
  opt-in) are the next workstream.

---

## 4. Current verified gates — R6 closure, 2026-05-03

All gates below were re-run locally on 2026-05-03 against the R6
closure commit. Results recorded verbatim from the test runner.

| Command | Result |
|---|---|
| `cargo test -p rpcs3-spu-recompiler --test single_spu_dma_get_v1_replay` | passed (1) |
| `cargo test -p rpcs3-spu-recompiler --test single_spu_mailbox_v1_replay` | passed (1) |
| `cargo test -p rpcs3-spu-recompiler --test single_spu_branch_loop_v1_replay` | passed (1) |
| `cargo test -p rpcs3-spu-recompiler --test single_spu_signal_v1_replay` | passed (1) |
| `cargo test -p rpcs3-spu-recompiler --test single_spu_loadstore_v1_replay` | passed (1) |
| `cargo test -p rpcs3-spu-recompiler --test single_spu_mailbox_multi_v1_replay` | passed (1) |
| `cargo test -p rpcs3-spu-recompiler --test game_like_mailbox_signal_v1_replay` | passed (1) |
| `cargo test -p rpcs3-spu-differential --lib` | passed (137) |
| `cargo test -p rpcs3-spu-thread --lib` | passed (44) |
| `cargo test --workspace --lib --no-fail-fast` | passed (0 failed across all crates) |
| `python behavior-freeze/harness/check_trace_fixtures.py` | exit 0; 7 fixtures listed; `REPLAY_VALIDATED_TRACE_EXISTS = True` |
| `python behavior-freeze/harness/check_patch_separation.py` | exit 0; SHAs match |

C++ trace patches preserved unchanged (sha256, pinned by
`check_patch_separation.py`):

| Patch | sha256 |
|---|---|
| scaffolding (`rpcs3/Emu/Cell/SPUTraceJsonl.{h,cpp}`) | `cda976d7b7bace826b3e8c38475fab5e077c88201bd0b31768541f06635143a1` |
| runtime hooks (`SPUThread.cpp` + writer-side integration) | `95bdcaae4850f3b2a94b5aea59761263589efabeac71bd3cb8464ad59c3a6721` |
| rust bridge (`SPURustBridge.cpp`) | `7d6b6bba3d1c590ec16f2ff175b262a4f95bdf95ace92eb91636824488436c03` |

**`cargo test --workspace --release` is NOT asserted green.** A handful
of HLE crates (`rpcs3-hle-cellsysutilmisc`, `rpcs3-hle-celljpgdec`,
`rpcs3-hle-cellmusicselectioncontext`, `rpcs3-hle-cellvideoexport`)
have a pre-existing `no_std` / `global_allocator` build error that
surfaces under `--release`. This is unrelated to the SPU stack and
predates R4a. **`--workspace --lib` is the scoped green gate.** Do not
promote the workspace as "green" without specifying scope.

---

## 5. Current completed components

The following components are complete and exercised by the gates in § 4:

- **`behavior-freeze/` harness.** Python gates
  (`check_trace_fixtures.py`, `check_patch_separation.py`,
  `build_synthetic_fixtures.py`, `spu_homebrew_runner.py`,
  `test_spu_homebrew_runner.py`) + fixtures + canonical
  `.spuimg` / `.dmachunk` pools.
- **SPU decoder** (`rust/rpcs3-spu-decoder/`). Two-pass leader
  analysis, basic-block graphs, ~107 opcodes covered (including
  R5.10 ISA-coverage additions for LQA/STQA/LQR/STQR/CBD/CWD/CHD/CDD,
  FSM/FSMH/FSMB/FSMBI, ROTQBYI/ROTQMBYI/SHLQBYI/SHLQBII, byte-imm
  RI10s + Class-A RI10s, and the RRR-form rt/rc fix from R5.11b).
- **SPU interpreter** (`rust/rpcs3-spu-interpreter/`). ~70% ISA, FTZ
  denormal flush, halfword/byte ops, channel I/O snapshot, RR-form
  `rotqby` (R5.11b add), corrected C-family default mask byte-order
  (R5.11b fix), corrected RRR-form rt/rc dispatch (R5.11b fix).
- **SPU recompiler** (`rust/rpcs3-spu-recompiler/`). Cranelift-backed
  JIT covering the broad subset (ALU word/halfword/byte, compares,
  shifts, multiplies, float arith/compares/converts, RRR, branches
  direct + indirect, branch hints, qword load/store, byte-imm), plus
  R4a dispatcher loop, R4b safe chained patching with ls_hash guard,
  R4c per-entry SMC scan with exact range hash, R5 partial fallback
  to interpreter from `JitState`. Channel ops jitted via runtime
  helpers (`spu_helper_rdch` / `spu_helper_wrch` / `spu_helper_rchcnt`).
- **`rpcs3-spu-thread`** state machine. `SpuThread` + `SpuChannels`
  (with R6.7 Phase C MFC channel fields and `tag_stat_queue`
  `VecDeque`), park/wake API, `SpuWakeResult`, `SpuExecEvent`,
  single-threaded executor.
- **`rpcs3-spu-differential`** + `SpuExecutor` trait, `SpuProgram` +
  `initial_gpr_overrides` + `initial_mfc_tag_stat_queue`,
  `SpuStateSnapshot`, `diff_snapshots`,
  `InterpreterExecutor` reference oracle, the
  `SpuPpuLockstepDriver`, `replay_per_spu_traces` orchestrator,
  R6.7 modules `dma_chunk.rs` (A.3 loader) and `mfc_replay.rs` (A.4
  state machine), `apply_mfc_dma_pre_replay()` (Phase C helper).
- **FFI crate `rpcs3-spu-ffi`.** Static lib (`/MT`) consumed by the
  C++ bridge in `rpcs3/Emu/Cell/SPURustBridge.cpp`.
- **C++ ↔ Rust SPU bridge** for the **supported non-DMA workloads**
  (oracles 1-6). `try_delegate_execution()` +
  `stop_and_signal()` re-use + persistent
  `unordered_map<lv2_id, BridgeSession>` side-table + multi-round
  loop with `pop_wait` for Stalls (R6.4b). StallWrite ch28
  depth-1 overwrite (R6.5b). Default OFF preserved; opt-in via
  `bin/config/config.yml`.
- **JSONL trace capture pipeline.** Writer
  (`rpcs3/Emu/Cell/SPUTraceJsonl.{h,cpp}` + `SPUThread.cpp` hooks)
  emits the 10-original event kinds + R6.7 A.1 additions
  (`spu_mfc_cmd`, `mfc_dma_complete`). Env-var-gated via
  `RPCS3_SPU_TRACE_JSONL`. Noop when disabled.
- **`.spuimg` side-file pipeline.** Content-addressed by SHA-256;
  canonical pool at `behavior-freeze/fixtures/spu/images/`; loaded by
  `build_spu_program_from_captured_image()` with hash + size +
  entry_pc validation.
- **`.dmachunk` side-file pipeline.** Content-addressed by SHA-256;
  canonical pool at `behavior-freeze/fixtures/spu/dma/`; loaded by
  `resolve_dma_chunk_side_file()` (per-trace `<jsonl>.dma/` precedence
  + canonical fallback); validated against the `ea_chunk_sha256` +
  size fields in the corresponding `spu_mfc_cmd` event before any
  byte is touched.
- **DMA / MFC GET-only replay.** `MfcReplayState` supports
  Immediate / Any / All wait modes; `apply_mfc_dma_pre_replay()`
  walks the captured events, drives the state machine, loads the
  `.dmachunk` via the A.3 loader, and produces a `SpuProgram` whose
  LS already contains the post-DMA bytes plus a pre-populated
  rdch ch24 queue.
- **Seven replay-validated oracles.** Listed in § 3.

---

## 6. Current partially complete components

The following components are partially landed and have defined next
work (mostly R7 / R8+):

- **Runtime DMA bridge.** Bridge currently has NO callback into
  RPCS3's `process_mfc_cmd()`. Bridge ON cannot delegate a
  `wrch ch21` honestly. R7.1 (Phase B honest-fallback) + R7.2
  (Phase D runtime DMA opt-in) cover this. The replay path works
  end-to-end; the runtime path does not.
- **MFC PUT (LS → EA).** Symmetric to GET but requires capturing
  EA-before-PUT bytes for replay determinism. Out of R7. Defer to
  R8+.
- **DMA list commands** (`GETL`, `PUTL`, `GETLB`, `PUTLB`, etc.).
  Need per-list-element event sequencing. Out of R7. Defer to R8+.
- **Atomic primitives** (`GETLLAR`, `PUTLLC`, `PUTLLUC`, `PUTQLLUC`).
  LL/SC reservation tracking is its own work item. Out of R7. Defer.
- **MFC barriers / fence bits.** Defer until ≥2 overlapping DMAs
  are observed in a CC0 fixture.
- **Multi-SPU DMA races on shared EA regions.** R6+R7 are single-SPU
  only. Defer.
- **SPURS / v4 diagnostic traces.** `tests/data/spurs_test_v3_real.jsonl`
  (R5.9d-era multi-SPU SPURS, 6 SPUs) and
  `tests/data/spurs_test_v4_real.jsonl` (R5.10a..p iteration trace,
  DMA-bound at pc=0x74C `wrch ch16 MFC_LSA`) remain
  **diagnostic-only**. The R5.10p analysis catalogued the full MFC
  GET sequence in v4. Both contain commercial code and are never
  promoted. They surface ISA / protocol gaps as diagnostic signals
  only.
- **Production performance.** Speedup numbers reported in the R5
  archive are observed benchmarks against synthetic fixtures; no
  real-workload benchmark has been published. The R4b/R4c chained
  patching + SMC scan work is correct but performance under sustained
  game workloads is not characterized.
- **Broad RPCS3 subsystems outside SPU.** RSX runtime, PPU JIT,
  Qt UI, audio backends, full LV2 syscall fidelity, loader / game
  boot parity — all carry partial Rust scaffolding from earlier
  waves but none are production-ready. None gated.

---

## 7. Current out-of-scope / not yet done

These items are **not** part of R6 closure and are not active workstreams:

- **Runtime DMA execution through the bridge.** Moves to R7.1 + R7.2
  (see § 9).
- **R7 / R8 advanced DMA features** — PUT, list commands, atomics,
  barriers/fence, multi-SPU DMA races on shared EA. R8+ scope.
- **Full PPU JIT.** Out of every R5/R6 wave; no Rust PPU recompiler
  exists. PPU stays on the C++ side.
- **RSX runtime.** Out of scope; the Rust workspace has crates with
  RSX-adjacent helpers but no GS frame execution.
- **Full LV2 / syscall fidelity.** Many syscalls are Rust scaffolded
  for header / signature parity, not execution parity. Out of scope.
- **Complete loader / game boot parity.** PSF / PUP / PKG / SELF /
  decrypt paths are partially Rust-mirrored at the contract level
  (per `docs/history/INVENTORY.md` — moved from `behavior-freeze/docs/`
  in 2026-05-22 consolidation) but full boot of a commercial game
  does not run through the Rust stack.
- **UI / packaging.** No Qt UI port. No installer / packaging story.
- **Commercial game trace promotion.** Hard rule: traces of
  commercial PS3 games NEVER go into `behavior-freeze/`. Only CC0
  homebrew authored for this project. Same for any future DMA / SPURS
  fixture.

---

## 8. R6 closure summary

R6 is formally closed at R6.7 A.5 (2026-05-03). The closure delivers:

1. A C++ ↔ Rust runtime SPU bridge that executes real `.self`
   binaries through the Rust executor **for the supported non-DMA
   workloads** (oracles 1-6). Bridge ON / OFF byte-identical for
   those workloads.
2. A complete DMA capture + replay pipeline (R6.7 A.1-A.5 + Phase C)
   for plain MFC GET commands — writer extension, parser,
   content-addressed `.dmachunk` side-files, state machine, executor
   wiring, and the load-bearing CC0 fixture
   `single_spu_dma_get_v1`. **All 7 oracles are replay byte-identical
   across Interpreter and Recompiler.**
3. The seventh replay-validated oracle (`single_spu_dma_get_v1`) that
   distinguishes "no DMA" from "wrong DMA" from "right DMA" via the
   canonical `0xDEADA12F` status — the only value reachable when
   (a) the GET actually copied 128 bytes from EA into LS at
   lsa=0x10000, AND (b) the SPU computed the deterministic post-DMA
   sum + XOR.

**Wording discipline:**

- We say **"the bridge is validated for the supported non-DMA
  workloads"**. We do NOT say "full runtime bridge".
- We say **"all 7 oracles are replay byte-identical"**. We do NOT
  say "bridge ON / OFF is byte-identical on all 7 runtime
  workloads" — bridge ON has only been validated against the 6
  non-DMA oracles; oracle #7 (`single_spu_dma_get_v1`) is
  replay-valid but the runtime bridge cannot yet honestly execute
  it, because it would need `process_mfc_cmd()` delegation that
  Phase B / Phase D will land in R7.
- We say **"runtime bridge DMA moves to R7"**. Period.

**Trace shape of the seventh oracle** (15 events):

```
seq  0: spu_image       sha=97a38063…  size=0x40000  entry_pc=0
seq  1: spu_wrch ch16=0x10000     (MFC_LSA)
seq  2: spu_wrch ch17=0           (MFC_EAH)
seq  3: spu_wrch ch18=0x10068400  (MFC_EAL)
seq  4: spu_wrch ch19=128         (MFC_Size)
seq  5: spu_wrch ch20=3           (MFC_TagID)
seq  6: spu_wrch ch21=0x40        (MFC_Cmd = GET)
seq  7: spu_mfc_cmd  cmd=0x40 tag=3 size=128 lsa=0x10000 eah=0 eal=0x10068400
                                                  ea_chunk_sha256=471fb943…
seq  8: mfc_dma_complete  tag=3  transferred_bytes=128
seq  9: spu_wrch ch22=0x8         (WrTagMask = 1<<3)
seq 10: spu_wrch ch23=0x2         (WrTagUpdate = MFC_TAG_UPDATE_ALL)
seq 11: spu_rdch ch24=0x8         (RdTagStat returns mask 1<<3)
seq 12: spu_wrch ch28=0xDEADA12F  (OUT_MBOX = canonical post-DMA cs)
seq 13: spu_stop  stop_code=0x101
seq 14: final_state  r18=0x1FC0 r19=0xDEADBEEF r20=0xDEADA12F
```

**Capture requirements for re-capture** (load-bearing, documented in
the fixture's `.notes.md` and in
[`docs/SPU_DMA_MFC_R6_7_DESIGN.md`](./SPU_DMA_MFC_R6_7_DESIGN.md) § 13.3):

1. **`subst R: <repo-root>` active during build.** The MSBuild
   `link.command.1.tlog` for `rpcs3.exe` carries 545 burned-in `R:\`
   paths from a legacy SUBST build configuration. Without an active
   SUBST, the linker silently skips the missing `R:\` `/LIBPATH:`
   directives and falls through to `$(VULKAN_SDK)\Lib\glslang.lib`
   (75 MB, /MD CRT) which is incompatible with the rpcs3 `/MT`
   build → 52 LNK2001 unresolved `spvtools::Optimizer` externals
   from `glslang.lib(SpvTools.obj)`. Fix is one command before the
   build.
2. **`Core: SPU Decoder: Interpreter (static)`** (and PPU Decoder)
   in `bin/config/config.yml` for the CAPTURE run only. RPCS3 LLVM
   JIT bypasses the C++ `set_ch_value()` / `get_ch_value()` for MFC
   channels, and the R6.7 A.1 trace hooks live inside those
   functions — JIT inlining suppresses them. Restore to
   `Recompiler (LLVM)` after capture. Documented in the fixture's
   `.notes.md`.

**Hard rules carried forward to R7 and beyond (unchanged):**

- No fake JSONL.
- No manual JSONL editing after capture.
- No commercial trace promotion.
- No fake DMA — synthesising `MFC_Cmd=0x40` success without
  consulting an oracle (replay) or RPCS3 vm:: (runtime) is a hard
  reject.
- No fake `RdTagStat` — never return a fixed/zero/random tag stat
  for `rdch ch24`.
- No fake LS bytes after a GET — `.dmachunk` content must hash to
  the captured `ea_chunk_sha256`; the R7.2 runtime path reads via
  `vm::_ptr<u8>(eal)` (real RPCS3 memory).
- v4 / SPURS stays diagnostic-only forever.

---

## R7 closure summary (2026-05-18)

R7 closed in a single session: R7.1 (Bridge Phase B honest
fallback) → R7.2 (Bridge Phase D runtime DMA GET) → R7.3 (triple
symmetry regression gate), all ACEITO.

**R7.1 — Bridge Phase B (honest fallback for `MFC_Cmd`).**
Adds `rust_spu_set_refuse_mfc(handle, 1)` FFI + a new outcome
variant `rust_spu_outcome_t_MfcUnsupported`. When the C++ bridge
installs the refuse gate AND no callback is set, the Rust
interpreter short-circuits ANY `wrch ch16..=23` / `rdch ch24/25`
/ `rchcnt` on those channels BEFORE per-channel mutation and
surfaces `MfcUnsupported`. The bridge's outcome switch logs
`"MFC/DMA detected at ch%u (%s), total_steps=%u ...
falling back honestly to C++ executor before Rust-side MFC
mutation. Channel state intact"` and drops the session — RPCS3
state is byte-identical to entry, the C++ executor takes over
from the original PC. Acceptance on `single_spu_dma_get_v1.self`:
bridge ON fell back at **ch16 (MFC_LSA)** at `total_steps=4`
(entry prologue), then C++ ran the .self and produced the
canonical TTY.

**R7.2 — Bridge Phase D (runtime DMA GET via FFI callback).**
Adds `rust_spu_set_dma_get_callback(handle, &fn, &user_data)`
FFI. The C++ bridge installs a static callback
`bridge_dma_get_callback` that reads `size` bytes from RPCS3 EA
`eal` via `vm::_ptr<u8>` (the same path the C++ executor's
`process_mfc_cmd()` simple-GET branch uses at
`rpcs3/Emu/Cell/SPUThread.cpp:2091`) and `memcpy`s them into the
Rust handle's LS at the captured `mfc_lsa`. The Rust interpreter
intercepts `wrch ch21 (MFC_Cmd)` BEFORE delegating to
`SpuChannels::write` and invokes the callback when cmd=0x40.
Validation (cmd=0x40, eah=0, tag<32, size ∈ {1,2,4,8} ∪
multiples of 16 ≤ 16384, lsa+size ≤ 256 KiB) happens in Rust
before the callback fires. On success the interpreter pushes
`1 << tag` into the tag-stat queue and the SPU continues; a
subsequent `rdch ch24` pops the value and the SPU finishes
naturally. Non-GET cmds, validation failures, and NULL EA still
surface `MfcUnsupported` (R7.1 fallback). Acceptance on
`single_spu_dma_get_v1.self`: bridge ON now **delegates
end-to-end** (`DELEGATED EXECUTION OK total_steps=1054
stall_iters=0`), with success log
`"R7.2 DMA GET dispatched: cmd=0x40 eal=0x10011180 size=128
tag=3 ... real EA/LS path (vm::_ptr<u8>); tag-stat 0x8 queued
for subsequent rdch ch24"` and the canonical TTY
`[dma_get_v1] OK cause=0x1 status=0xdeada12f`.

**R7.3 — Triple-symmetry regression gate.** New harness:
`behavior-freeze/harness/check_triple_symmetry.py`. Runs all
three execution paths against `single_spu_dma_get_v1.self` and
asserts they converge on the canonical status `0xdeada12f`:

1. **bridge OFF real binary** — `rpcs3.exe` with bare C++ executor
2. **bridge ON real binary** — `rpcs3.exe` with
   `RPCS3_SPU_RUST_BRIDGE=1`; the Rust bridge delegates
   end-to-end via R7.2 runtime DMA GET (no fallback line in
   the Rust bridge log)
3. **replay oracle** —
   `cargo test single_spu_dma_get_v1_replay --release`
   asserts `diff_snapshots(interp, jit).is_identical() == true`

All three pass.

**Patch SHAs at R7 closure (`check_patch_separation.py` pin):**

| Patch | sha256 | Δ vs R6 closure |
|---|---|---|
| scaffolding | `cda976d7b7bace826b3e8c38475fab5e077c88201bd0b31768541f06635143a1` | unchanged |
| runtime hooks | `95bdcaae4850f3b2a94b5aea59761263589efabeac71bd3cb8464ad59c3a6721` | unchanged |
| rust bridge | `a1e810264d8d9474018c279606111b543eb3f6b6c5845839382e4a657e220e70` | **bumped** (was `7d6b6bba…` at R6, `eeb57616…` at R7.1; final R7.2 sha is the recorded one) |

**rpcs3.exe at R7 closure:** sha256
`81ac5f096b5b9e79d1d466f35f8d986129636c2801093a3a86d30cd65f2a4404`
(64 MB; built 2026-05-18 03:22 with R7.1 + R7.2 surface).

**Out of R7 scope (deferred to R8+):** MFC PUT (now LANDED in
R8.1, see § 8.1 below), DMA list cmds (GETL/PUTL/GETLB/PUTLB),
atomic primitives (GETLLAR/PUTLLC/PUTLLUC/PUTQLLUC), MFC barriers
/ fence bits, multi-SPU DMA races on shared EA, SPURS production
support. The R6 hard rules above carry forward verbatim to R7 and
to R8+.

---

## 8.1 R8.1 closure summary (2026-05-19)

R8.1 landed in a single session: Rust core (parser + state machine
+ channels + interpreter routing + FFI) → Docker .self build →
RPCS3 OFF capture → bridge ON acceptance → 8th replay oracle →
triple-symmetry gate extended → patches regenerated → docs
updated.

**Scope.** First MFC PUT-bound oracle + symmetric runtime extension
on top of R7.2's runtime DMA GET. Mirrors the R6.7 A.5 / R7.2 GET
delivery but inverts DMA direction (LS → EA). The captured
`.dmachunk` now carries the SPU's LS-source bytes at dispatch
time; the runtime bridge writes them to RPCS3 EA via
`vm::_ptr<u8>`; the replay oracle verifies SPU LS at the PUT
region matches the chunk post-execution.

**Rust core extensions.**
- `trace_fmt.rs` parser accepts `spu_mfc_cmd.cmd ∈ {0x40 GET,
  0x20 PUT}`; the defensive subset-rejection canary moves to
  `0x44 GETL` (list variants still out of scope).
- `mfc_replay.rs` adds `MfcReplayError::PutLsBytesMismatch`
  (load-bearing PUT correctness gate) and a new public method
  `process_mfc_cmd_pre_replay`. The PRE-replay variant defers
  the PUT LS-bytes assertion (cannot inspect dispatch-time LS
  before the SPU runs); the AssertNow `process_mfc_cmd` remains
  the canonical at-dispatch state machine for future in-line
  executor wiring.
- `rpcs3-spu-thread::SpuChannels` adds `dma_put_callback:
  Option<DmaPutCallback>` symmetric to the GET callback. The
  `refuse_mfc` gate is RELAXED whenever EITHER callback is
  installed.
- `rpcs3-spu-interpreter` wrch ch21 intercept routes by cmd:
  0x40 → GET callback, 0x20 → PUT callback, other →
  `MfcUnsupported`. PUT source pointer is read via the SPU
  thread's LS at the dispatch lsa.
- `rpcs3-spu-ffi` adds `rust_spu_set_dma_put_callback` +
  `DmaPutCallbackFn` typedef. The C header is updated. FFI
  tests serialized via a new `CALLBACK_TEST_MUTEX` (static
  AtomicU32 observers cross-pollute under cargo's parallel test
  runner without it).

**C++ side extensions.**
- `SPUThread.cpp` writer hook (R6.7 A.1 extended): the same
  `record_spu_mfc_cmd` + `record_mfc_dma_complete` events now
  fire for cmd=0x20 PUT. For PUT the snapshot bytes come from
  `this->ls + mfc_lsa` (vs `vm::_ptr<u8>(mfc_eal)` for GET).
- `SPURustBridge.cpp` bridge: new static
  `bridge_dma_put_callback` reads `src_ls_ptr` size bytes (the
  SPU's LS source) and writes to `vm::_ptr<u8>(eal)`. Installed
  alongside the GET callback on every `rust_spu_new` in
  `try_delegate_execution()`. Success log: `R8.1 DMA PUT
  dispatched: cmd=0x20 eal=0x... size=N tag=T on '...'; real
  LS/EA path (vm::_ptr<u8>); tag-stat 1<<T queued for subsequent
  rdch ch24`.

**Fixture + capture.** `single_spu_dma_put_v1` source (PSL1GHT
homebrew, CC0): PPU allocates a 128-byte BSS buffer zero-filled,
passes EA via `thread_args.arg0`; SPU fills LS at `0x10000` with
counting pattern `i & 0xFF` (sum = 8128 = 0x1FC0), dispatches MFC
PUT tag 3 size 128, waits via ch22/23/24, writes sentinel
`0xC0FFEECA` to OUT_MBOX, halts via stop 0x101. PPU joins, reads
EA back, sums, XORs with `0xCAFEBABE` to produce `ea_status =
0xCAFEA57E`. Built in Docker via `rpcs3-ps3dev-toolchain:local`
(image sha `ed2167a9ac59…`); `.self` 939,475 bytes sha
`761414892bd3757a1a1d8238d6623f7270e5fee49321620b5c47b466e321f3c5`.
Capture run with `Core: SPU/PPU Decoder: Interpreter (static)`
(LLVM JIT bypasses `set_ch_value()` MFC hooks per R6.7 A.5
gotcha); trace is 15 events including the new `spu_mfc_cmd
cmd=0x20` event with `ea_chunk_sha256=471fb943…` — the SAME
content-addressed pool entry as the GET fixture (deduplicated
naturally because both fixtures use the same source pattern).

**Replay oracle.** New
`rust/rpcs3-spu-recompiler/tests/single_spu_dma_put_v1_replay.rs`:
- Parses JSONL via `parse_jsonl_trace`.
- Asserts cmd=0x20, tag=3, size=128, eah=0, lsa=0x10000,
  dma_complete_count=1, ch28 carries `0xC0FFEECA`, stop 0x101.
- Builds `SpuProgram` from `.spuimg`, seeds r3 with the PSL1GHT
  arg0 EA (SPU calling convention places u64 arg0 high in lane
  0, low in lane 1 — without this the final-state ExpectGprWord
  for r3 fails since PUT keeps EA in r3 through exit; GET
  overwrites r3 with tag_stat so doesn't need the seed).
- Calls `apply_mfc_dma_pre_replay` (PRE-replay PUT route via
  `process_mfc_cmd_pre_replay` — chunk SHA validated, LS-bytes
  assertion deferred).
- Runs replay × Interpreter + replay × Recompiler;
  `diff_snapshots.is_identical()`.
- **Post-replay deferred PUT verification:** loads the captured
  chunk via `resolve_dma_chunk_side_file` and asserts that BOTH
  backends' final LS at `[lsa..lsa+size]` matches the chunk
  byte-for-byte. This restores the dispatch-time contract for
  the canonical fixture (the SPU does not touch LS after PUT
  dispatch).

**Triple-symmetry extension.**
`behavior-freeze/harness/check_triple_symmetry.py` was refactored
to parametrize by fixture via `--fixture {get,put}` (default
`get`, R7.3 backwards-compatible). Both fixtures pass green:

| Path | GET | PUT |
|---|---|---|
| bridge OFF TTY | `[dma_get_v1] OK cause=0x1 status=0xdeada12f` ✓ | `[dma_put_v1] OK cause=0x1 spu=0xc0ffeeca ea_status=0xcafea57e` ✓ |
| bridge ON delegation | `R7.2 DMA GET dispatched ... total_steps=1054 stall_iters=0` ✓ | `R8.1 DMA PUT dispatched ... total_steps=1049 stall_iters=0` ✓ |
| replay oracle | `single_spu_dma_get_v1_replay` ok ✓ | `single_spu_dma_put_v1_replay` ok ✓ |

**Patch SHAs at R8.1 landing (`check_patch_separation.py` pin):**

| Patch | sha256 | Δ vs R7 closure |
|---|---|---|
| scaffolding | `cda976d7b7bace826b3e8c38475fab5e077c88201bd0b31768541f06635143a1` | unchanged |
| runtime hooks | `1f598d37c07d30f7e8b43fa0e53018b7c9277f628c2af58ab40bf987ffbd46ff` | **bumped** (was `95bdcaae…`; PUT-extended writer hook in SPUThread.cpp) |
| rust bridge | `0afda1c6943feb5d98329299a57dd68404095efb0a792839779febed13ab8a7e` | **bumped** (was `a1e810264d…`; PUT callback + bridge_dma_put_callback added) |

**rpcs3.exe at R8.1 landing:** sha256
`3ef63a825f9820373bb1df175bc975d5063f531b98206860fab36a50a8cd95d2`
(64 MB; built 2026-05-19 with R7.1 + R7.2 + R8.1 surface).

**Out of R8.1 scope (deferred to R8.2+):** DMA list cmds
(GETL/PUTL/GETLB/PUTLB), atomic primitives (GETLLAR/PUTLLC/
PUTLLUC/PUTQLLUC), MFC barriers / fence bits, multi-SPU DMA
races on shared EA, SPURS production support, in-line state
machine driven by the executor (would restore the dispatch-time
PUT assertion contract — currently deferred to post-replay). The
R6 / R7 hard rules carry forward verbatim to R8.1 and R8.2+.

---

## 8.2 R8.2 closure summary (2026-05-20)

R8.2 landed as the cleanest fixture-only delivery to date. The
9th oracle `single_spu_dma_get_multi_v1` exercises multi-tag
in-flight DMA + ALL wait semantics + multi-bit `WrTagMask`, on
top of the R8.1 baseline. **Zero engine-side code changes** —
the existing 8-oracle implementation already covered every
mechanic R8.2 required.

**Scope.** Two queued MFC GETs (tags 3 + 5, distinct EAs,
distinct sizes 128 + 64, distinct LSAs 0x10000 + 0x10100,
both in-flight before any wait) + `WrTagMask = 0x28` (=
`(1 << 3) | (1 << 5)`) + `WrTagUpdate = ALL` + `RdTagStat
= 0x28` (returned only after both completions fire). The SPU
computes a combined checksum `status = ((sum1 << 16) | sum2)
^ 0xFEEDFACE = 0xE12DEA4E` and halts via stop 0x101.

**Why this validates with no code changes.**

| Concern | Coverage |
|---|---|
| Parser accepts 2 `spu_mfc_cmd` events with cmd=0x40 | R6.7 A.2 already accepts cmd=0x40 unconditionally; events stream through the per-SPU transformer as pure context |
| State machine handles 2 tags in flight | R6.7 A.4 unit test `mfc_replay_handles_wr_tag_mask_update_basic` already covered 2-tag ALL mode (tags 3 + 5, same as R8.2) |
| Chunk loader resolves 2 distinct SHAs | R6.7 A.3 `resolve_dma_chunk_side_file` is content-addressed, indifferent to how many GETs the trace contains |
| Tag-stat queue with multi-bit value | R6.7 Phase C wired `mfc_tag_stat_queue: VecDeque<u32>`; ALL mode pushes exactly one entry (the mask) regardless of how many GETs preceded it |
| Bridge ON multi-dispatch | R7.2 callback is invoked per `wrch ch21` automatically; refuse_mfc gate already relaxed once the callback is installed |
| Executor reads from correct LS regions | `apply_mfc_dma_pre_replay` walks events linearly and copies each chunk into LS at the captured (lsa, size) — distinct lsa per GET means no overwriting |

The empirical "investigar quando acontecer" policy paid off:
implementing PUT in R8.1 also primed the engine for any multi-DMA
GET workload that doesn't introduce list / atomic / fence
semantics.

**Fixture + capture.** `single_spu_dma_get_multi_v1` source
(PSL1GHT homebrew, CC0): PPU allocates two distinct EA buffers
(ea_buf1 = 128 B counting pattern `i & 0xFF`, ea_buf2 = 64 B
constant 0x42), passes EA1 via `thread_args.arg0` and EA2 via
`arg1`. SPU dispatches both GETs back-to-back, waits via
`ch22 = 0x28` + `ch23 = ALL` + `ch24` read, sums both LS
regions, computes combined status, writes OUT_MBOX, halts.
Built in Docker via `rpcs3-ps3dev-toolchain:local` (image sha
`ed2167a9ac59…`); `.self` 940 KB sha
`7eb545af47a2c51e064b4d79090e2930d1cd6058edbd9d29032785d0ad535659`.
Capture run with `Core: SPU/PPU Decoder: Interpreter (static)`
(R8.1 gotcha carries forward); trace is 23 events (vs 15 for
single-DMA fixtures) including 2 `spu_mfc_cmd` + 2
`mfc_dma_complete` events + 2 distinct `.dmachunk` references.

**Content-addressed `.dmachunk` pool dedup.** Chunk #1
(`471fb943aa23c511f6f72f8d1652d9c880cfa392ad80503120547703e56a2be5`,
128 B counting pattern) deduplicates with R6.7 GET v1 + R8.1
PUT v1 — already in the canonical pool, NOT re-committed. Chunk
#2 (`c422e7070cb1cb455b5de9afee0d975e303d0239c72030cd7414ab5c382d3ae8`,
64 B constant 0x42) is new and lands in the pool. The pool now
holds 2 chunks total.

**Replay oracle.** New `rust/rpcs3-spu-recompiler/tests/
single_spu_dma_get_multi_v1_replay.rs`:
- Parses JSONL via `parse_jsonl_trace`.
- Asserts 2 `spu_mfc_cmd` events (tags 3 + 5, sizes 128 + 64,
  cmd 0x40, EAs distinct, chunk SHAs distinct).
- Asserts 2 `mfc_dma_complete` events matching tags + sizes.
- Asserts `ch22 = 0x28`, `ch23 = 2 (ALL)`, `ch24 rdch = 0x28`,
  ch28 = `0xE12DEA4E`, stop 0x101.
- Builds `SpuProgram` from `.spuimg`, seeds r3 = EA1 lane 1 +
  r4 = EA2 lane 1 (PSL1GHT arg0 + arg1 convention).
- Calls `apply_mfc_dma_pre_replay` (both chunks land in LS at
  captured LSAs; pre-application sanity-checks both regions
  carry the expected patterns).
- Runs replay × Interpreter + replay × Recompiler;
  `diff_snapshots.is_identical()`.
- Post-replay verifies BOTH backends' final LS at both regions
  matches the captured chunks byte-for-byte (mirrors the R8.1
  PUT shape).

**Triple-symmetry extension.** `check_triple_symmetry.py`
`FIXTURES` dict gained `get_multi`. Three fixtures green:

| Path | GET | PUT | GET_MULTI |
|---|---|---|---|
| bridge OFF TTY | `0xdeada12f` ✓ | spu=`0xc0ffeeca` ea_status=`0xcafea57e` ✓ | `0xe12dea4e` ✓ |
| bridge ON delegation | R7.2 total_steps=1054 ✓ | R8.1 total_steps=1049 ✓ | R7.2×2 total_steps=1584 ✓ |
| replay oracle | get_v1_replay ✓ | put_v1_replay ✓ | get_multi_v1_replay ✓ |

**Patch SHAs at R8.2 landing (`check_patch_separation.py` pin):**

| Patch | sha256 | Δ vs R8.1 |
|---|---|---|
| scaffolding | `cda976d7b7bace826b3e8c38475fab5e077c88201bd0b31768541f06635143a1` | unchanged |
| runtime hooks | `1f598d37c07d30f7e8b43fa0e53018b7c9277f628c2af58ab40bf987ffbd46ff` | unchanged |
| rust bridge | `0afda1c6943feb5d98329299a57dd68404095efb0a792839779febed13ab8a7e` | unchanged |

**rpcs3.exe at R8.2 landing:** sha256
`3ef63a825f9820373bb1df175bc975d5063f531b98206860fab36a50a8cd95d2`
(same binary as R8.1; no rebuild needed).

**Out of R8.2 scope (deferred to R8.3+):** DMA list cmds
(GETL/PUTL/GETLB/PUTLB), atomic primitives (GETLLAR/PUTLLC/
PUTLLUC/PUTQLLUC), MFC barriers / fence bits, multi-SPU DMA
races on shared EA, SPURS production support, ANY wait mode
(R8.2 covers ALL only; ANY exists in the state machine but
no oracle exercises it yet — **landed in R8.3a, see § 8.3a**),
in-line state-machine executor wiring. The R6 / R7 / R8.1 hard
rules carry forward verbatim.

---

## 8.3a R8.3a closure summary (2026-05-20)

R8.3a landed on the same day as R8.2 as the first DMA fixture
that surfaced a **real runtime/replay divergence** and co-fixed
it in the same delivery. The 10th oracle `single_spu_dma_get_any_v1`
exercises the ANY wait mode that existed in the state machine
since R6.7 A.4 but no oracle had previously tested. The fixture's
canonical status embeds the actual ch24 returned value via a
`(tag_stat << 24) ^ 0xBEEFBEAD` XOR — the embed is what allowed
the divergence to surface.

**Scope.** Two queued MFC GETs (same shape as R8.2 multi: tags
3 + 5, distinct EAs / sizes 128 + 64 / LSAs 0x10000 + 0x10100)
+ `WrTagMask = 0x28` + `WrTagUpdate = ANY` (= 1) +
`RdTagStat = 0x28` (captured RPCS3 sync-DMA return) + SPU
computes `combined = (sum1 << 16) | sum2` + `status = combined
^ (tag_stat << 24) ^ 0xBEEFBEAD = 0x892FAE2D` and halts via
stop 0x101.

**Divergence detected at first bridge ON.** Triple-symmetry's
[2/3] phase reported:

```
expected TTY: '[dma_get_any_v1] OK cause=0x1 status=0x892fae2d'
got TTY     : '[dma_get_any_v1] OK cause=0x1 status=0xa92fae2d'
```

Reverse-engineering the wrong status:

```
0xA92FAE2D ^ 0xBEEFBEAD = 0x17C0_1080
0x17C0_1080 ^ 0x1FC0_1080 = 0x0800_0000  →  tag_stat = 0x08
```

The Rust SPU saw `ch24 = 0x08` (= `1 << 3`, tag 3 only) instead
of the captured aggregate `0x28`. C++ RPCS3 returns
`completed_tags & wr_tag_mask` for ch24 reads; the Rust runtime
was popping the front of `mfc_tag_stat_queue` one entry at a
time, observing only the first GET's `1 << tag` bit. Multi-DMA
ALL mode (R8.2) didn't surface this because R8.2's SPU code
discards `tag_stat` after read (`(void)tag_stat;`) — the bug
was latent until R8.3a's tag_stat embedding exposed it.

**Engine fix (single file, single function).** In
`rust/rpcs3-spu-thread/src/lib.rs`, the `MFC_RD_TAG_STAT` arm
of `SpuChannels::read`:

```rust
ch::MFC_RD_TAG_STAT => {
    if self.mfc_tag_stat_queue.is_empty() {
        return Err(ChannelStatus::WouldStall);
    }
    let mut completed: u32 = 0;
    while let Some(v) = self.mfc_tag_stat_queue.pop_front() {
        completed |= v;
    }
    Ok(completed & self.mfc_wr_tag_mask)
}
```

The drain-OR-AND shape unifies two producer paths:

- **Pre-replay** (R6.7 C.3 `apply_mfc_dma_pre_replay`): pushes
  a single pre-aggregated value per captured `spu_rdch ch24`.
  Drain-OR returns it; mask-AND is a no-op (captured value is
  already mask-filtered).
- **Runtime** (R7.2 GET + R8.1 PUT callbacks): each ch21
  dispatch pushes `1 << tag`. Drain-OR aggregates back-to-back
  dispatches; mask-AND filters per the SPU's mask register.

The C++ `process_mfc_cmd` semantic for ch24 is
`completed_tags & wr_tag_mask` returning a snapshot of
completed bits intersected with the mask. The drain-OR-AND
implementation matches that observationally for one-shot reads.
A future fixture needing repeated ch24 reads (e.g. polling
multiple wait windows) would need persistent `completed_tags`
state — deferred until such a fixture exists per the empirical
scoping policy.

**Companion test update.** The unit test in `rpcs3-spu-thread`
that previously expected pop-one-at-a-time semantics was
rewritten as `mfc_rdtagstat_drains_queue_and_aggregates_with_mask`
covering: empty stall, multi-bit drain, single-aggregate drain,
mask-filtering when bits exceed the mask.

**rpcs3.exe rebuild.** The fix is in `rpcs3-spu-thread`,
linked statically into `rpcs3_spu_ffi.lib`, which rpcs3.exe
links against. `cargo build --release -p rpcs3-spu-ffi` produced
the new lib; copied to
`rpcs3-upstream-clean/rust/target/release/rpcs3_spu_ffi.lib`;
`msbuild rpcs3.sln /t:rpcs3 /p:Configuration=Release /m`
relinked rpcs3.exe in 56 seconds. New binary sha256:
`3d25d7828349e0091b7d0fdb58ce7c5bc88681b1c55f856ffbbdec26ac7524fa`
(was `3ef63a82…` at R8.1 / R8.2).

**Captured trace + replay.** 23 events (same as R8.2). New
`.spuimg` (`33dc6ca4…`, distinct from R8.2's because the SPU C
source changed `ch23 ALL → ANY` plus the tag_stat embed
arithmetic). **Zero new `.dmachunk` files** — both chunks
deduplicate with R8.2 (counting pattern + constant 0x42).
Pool stays at 2 entries total.

**Triple-symmetry post-fix.** All 4 fixtures green:

| Path | GET | PUT | GET_MULTI | GET_ANY |
|---|---|---|---|---|
| bridge OFF TTY | `0xdeada12f` ✓ | spu=`0xc0ffeeca` ✓ | `0xe12dea4e` ✓ | `0x892fae2d` ✓ |
| bridge ON delegation | R7.2 ts=1054 ✓ | R8.1 ts=1049 ✓ | R7.2×2 ts=1584 ✓ | R7.2×2-ANY ts=1588 ✓ |
| replay oracle | get_v1 ✓ | put_v1 ✓ | get_multi_v1 ✓ | get_any_v1 ✓ |

**Patch SHAs at R8.3a landing (`check_patch_separation.py` pin):**

| Patch | sha256 | Δ vs R8.2 |
|---|---|---|
| scaffolding | `cda976d7b7bace826b3e8c38475fab5e077c88201bd0b31768541f06635143a1` | unchanged |
| runtime hooks | `1f598d37c07d30f7e8b43fa0e53018b7c9277f628c2af58ab40bf987ffbd46ff` | unchanged |
| rust bridge | `0afda1c6943feb5d98329299a57dd68404095efb0a792839779febed13ab8a7e` | unchanged |

The fix lives in Rust core only — C++ patches did not change.
rpcs3.exe binary changes because the static lib it links has
new code, but the patch sha pins reference SOURCE CONTENT of
the C++ patches, which are byte-identical to R8.2.

**rpcs3.exe at R8.3a landing:** sha256
`3d25d7828349e0091b7d0fdb58ce7c5bc88681b1c55f856ffbbdec26ac7524fa`
(63,942,656 bytes; built 2026-05-20 with R7.1 + R7.2 + R8.1 +
R8.3a surface).

**Out of R8.3a scope (deferred to R8.3b+):** DMA list cmds
(GETL/PUTL/GETLB/PUTLB), atomic primitives (GETLLAR/PUTLLC/
PUTLLUC/PUTQLLUC), MFC barriers / fence bits, multi-SPU DMA
races on shared EA, SPURS production support, repeated ch24
reads in the same SPU session (current drain-clear semantics
work for the single-read shape but not for polling — **landed
in R8.3b, see § 8.3b**), persistent `completed_tags` state in
`SpuChannels` (**landed in R8.3b**), in-line state-machine
executor wiring. The R6 / R7 / R8.1 / R8.2 hard rules carry
forward verbatim.

---

## 8.3b R8.3b closure summary (2026-05-20)

R8.3b landed on the same day as R8.3a as the SECOND consecutive
fixture to surface and co-fix a real runtime/replay divergence.
The 11th oracle `single_spu_dma_tag_poll_v1` forces persistent
`completed_tags` semantics — exactly the limitation that the
R8.3a closure note documented as "deferred until such a fixture
exists per the empirical-scoping policy".

**Scope.** Two queued MFC GETs (same tags / EAs / sizes as
R8.3a) + `WrTagMask = 0x08, WrTagUpdate = ANY, RdTagStat → 0x08` +
`WrTagMask = 0x20, WrTagUpdate = ANY, RdTagStat → 0x20`. Both
reads execute in the same SPU session, between which the SPU
only changes the mask register. The SPU embeds BOTH returned
values into the canonical via `((tag_stat_1 << 24) | (tag_stat_2
<< 16)) ^ 0xCAFEBADC = 0xDD1EAA5C`. The captured trace contains
two `spu_rdch ch24` events (values 0x08 and 0x20) instead of one.

**Predicted divergences (all confirmed empirically before the
fix landed).**

- **Replay:** parked at pc=140 reason=ChannelRead{channel:24}.
  First read drained the queue, second read found queue empty
  → WouldStall → executor reports park outcome → trace replay
  fails with `UnexpectedSpuState{expected:Finished(0x101),
  actual:Parked{...}}`.
- **Bridge ON:** Rust bridge log:
  `R7.2 DMA GET dispatched ... R7.2 DMA GET dispatched ...
   try_delegate: StallRead stall on unknown channel during read
   (stall_iters=0 total_steps=36) ... falling back to C++
   executor`. The C++ executor takes over from the stall point
  and completes the SPU correctly, producing canonical TTY,
  but the Rust bridge did NOT delegate end-to-end (the
  triple-symmetry [2/3] gate FAILed on "expected DELEGATED
  EXECUTION OK").
- **Bridge OFF:** OK (C++ has persistent `completed_tags`).

**Engine fix.** Single field + single function change in
`rust/rpcs3-spu-thread/src/lib.rs`:

```rust
pub struct SpuChannels {
    // ... existing fields ...
    pub mfc_tag_stat_queue: std::collections::VecDeque<u32>,
    pub completed_tags: u32,  // R8.3b — persistent register
}

// In SpuChannels::read:
ch::MFC_RD_TAG_STAT => {
    while let Some(v) = self.mfc_tag_stat_queue.pop_front() {
        self.completed_tags |= v;
    }
    if self.completed_tags == 0 {
        return Err(ChannelStatus::WouldStall);
    }
    Ok(self.completed_tags & self.mfc_wr_tag_mask)
}
```

Behavior:
- Drain queue OR-into `completed_tags` (absorbs any
  newly-arrived per-tag bits from callbacks or pre-replay).
- Return `WouldStall` only if `completed_tags == 0`.
- Return `completed_tags & mfc_wr_tag_mask` — NEVER clears
  `completed_tags`. Multiple reads observe the same persistent
  state with per-mask filtering.

This matches Cell BE / C++ executor semantics exactly for read
behavior. Clear-on-write (R8.4+) is deferred.

**Companion unit test rewritten.** The R8.3a test
`mfc_rdtagstat_drains_queue_and_aggregates_with_mask` was
replaced by `mfc_rdtagstat_persistent_completed_tags_with_mask_filtering`
covering: empty stall, first-read absorb, second-read same-mask
(no stall — R8.3b invariant), per-mask filtering between reads,
additional callback push absorbed, full-mask read returns
aggregate.

**rpcs3.exe rebuild (twice, cargo cache gotcha).** First build
attempt produced no new `rpcs3_spu_ffi.lib` because cargo's
dependency tracker thought the build was up to date (the diff
between R8.3a and R8.3b was in `rpcs3-spu-thread` but the
package timestamp / hash wasn't refreshed). Symptom: triple-
symmetry's first pass still reported stall fallback for R8.3b.
Fix: `touch rpcs3-spu-thread/src/lib.rs && cargo build --release
-p rpcs3-spu-ffi` forced a rebuild. The fresh `.lib` produced
sha-changed binary; `msbuild rpcs3.sln /t:rpcs3` relinked
rpcs3.exe in 25 seconds. Final sha:
`34ec50d73d22eabb49fc9c6f3ddfebd55d58db76a61ce5e5c6dac14b0d20f851`
(was `3d25d782…` at R8.3a). The gotcha is now documented in
the memory file — for future Rust-core-only fixes, always
`touch` a relevant source before `cargo build` to bypass the
cache.

**Triple-symmetry post-fix.** All 5 fixtures green:

| Path | GET | PUT | GET_MULTI | GET_ANY | GET_TAG_POLL |
|---|---|---|---|---|---|
| bridge OFF TTY | ✓ | ✓ | ✓ | ✓ | ✓ |
| bridge ON delegation | R7.2 ts=1054 | R8.1 ts=1049 | R7.2×2 ts=1584 | R7.2×2-ANY ts=1588 | R7.2×2-poll ts=1594 |
| replay oracle | ✓ | ✓ | ✓ | ✓ | ✓ |

The increasing `total_steps` across the four DMA fixtures
(1054 → 1049 → 1584 → 1588 → 1594) maps cleanly onto the
SPU work: 1 GET + sum loop → 1 PUT + sum loop → 2 GETs + 1
double-sum loop → 2 GETs + 1 wait sequence + 1 double-sum
loop → 2 GETs + 2 wait sequences + 1 double-sum loop.

**Patch SHAs at R8.3b landing:**

| Patch | sha256 | Δ vs R8.3a |
|---|---|---|
| scaffolding | `cda976d7b7bace826b3e8c38475fab5e077c88201bd0b31768541f06635143a1` | unchanged |
| runtime hooks | `1f598d37c07d30f7e8b43fa0e53018b7c9277f628c2af58ab40bf987ffbd46ff` | unchanged |
| rust bridge | `0afda1c6943feb5d98329299a57dd68404095efb0a792839779febed13ab8a7e` | unchanged |

R8.3b is a Rust-core-only fix; all 3 C++ patch SHAs remain
identical to R8.1's pins.

**rpcs3.exe at R8.3b landing:** sha256
`34ec50d73d22eabb49fc9c6f3ddfebd55d58db76a61ce5e5c6dac14b0d20f851`
(63,942,656 bytes; built 2026-05-20 with R7.1 + R7.2 + R8.1 +
R8.3a + R8.3b surface).

**Out of R8.3b scope (deferred to R8.3c+):** DMA list cmds
(GETL/PUTL/GETLB/PUTLB), atomic primitives (GETLLAR/PUTLLC/
PUTLLUC/PUTQLLUC), MFC barriers / fence bits, multi-SPU DMA
races on shared EA, SPURS production support, `MFC_TAG_UPDATE_
IMMEDIATE` (= 0) which R8.3b deferred until an oracle
exercises it (**landed in R8.3c — captured ts2 = 0x28 proves
RPCS3 IMMEDIATE does NOT clear; see § 8.3c**), explicit per-bit
clearing via `WrTagUpdate` write (in some Cell BE
implementations), in-line state-machine executor wiring. The
R6 / R7 / R8.1 / R8.2 / R8.3a hard rules carry forward verbatim.

---

## 8.3c R8.3c closure summary (2026-05-20)

R8.3c is the third consecutive fixture (R8.3a → R8.3b → R8.3c)
to surface a real runtime/replay divergence and co-fix it.
The 12th oracle `single_spu_dma_tag_immediate_v1` confirms
RPCS3 IMMEDIATE behavior (no clear) AND surfaced a legacy
clear-on-read in the replay state machine that R8.3b's runtime
fix had already aligned away from on the executor side.

**Scope.** Same shape as R8.3b but `WrTagUpdate = IMMEDIATE`
(= 0) and overlapping masks (0x08 ⊂ 0x28). Captured `ts1 =
0x08`, `ts2 = 0x28` — the second mask covers BOTH tag bits
including the one observed by the first read. `ts2 = 0x28`
proves RPCS3 IMMEDIATE does NOT clear `completed_tags` on
read (Cell BE canonical semantic).

**Divergence + fix.** Pre-fix replay test surfaced
`TagStatMismatch{captured: 0x28, oracle: 0x20, wr_tag_mask:
0x28, mode: Immediate}`. Cause:
`MfcReplayState::process_rdch_tagstat` had a legacy
`self.completed_tags &= !observed_now` clear-on-read since
R6.7 A.4. R8.3b (separate ANY-mode masks 0x08 + 0x20, no
overlap) didn't surface it because each read cleared bits
unique to its mask. R8.3c's overlapping masks force the
tag-3 bit to PERSIST across reads. Fix: remove the clear in
`process_rdch_tagstat`. State machine now aligned with
R8.3b's `SpuChannels::read` runtime semantic (both
persistent).

**rpcs3.exe does NOT need rebuild.** The fix lives in
`rpcs3-spu-differential` (replay-only layer, consumed by
`rpcs3-spu-recompiler` tests). The runtime bridge path
through `rpcs3_spu_ffi.lib` already had the persistent
semantic from R8.3b's executor fix. Triple-symmetry green
with the R8.3b rpcs3.exe binary unchanged.

**Companion unit test updated.** The R6.7 A.4 test
`mfc_replay_chunk_loader_path` previously asserted
"After observation, the bit is cleared" + `completed_tags ==
0`. Rewritten to assert "completed_tags is unchanged" + a
re-read returns the same value (matching the persistent
semantic).

**Triple-symmetry post-fix.** All 6 fixtures green:

| Path | GET | PUT | GET_MULTI | GET_ANY | GET_TAG_POLL | GET_TAG_IMMEDIATE |
|---|---|---|---|---|---|---|
| bridge OFF TTY | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ |
| bridge ON delegation | R7.2 ts=1054 | R8.1 ts=1049 | R7.2×2 ts=1584 | R7.2×2-ANY ts=1588 | R7.2×2-poll ts=1594 | R7.2×2-IMM ts=1594 |
| replay oracle | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ |

**Patch SHAs at R8.3c landing:**

| Patch | sha256 | Δ vs R8.3b |
|---|---|---|
| scaffolding | `cda976d7b7bace826b3e8c38475fab5e077c88201bd0b31768541f06635143a1` | unchanged |
| runtime hooks | `1f598d37c07d30f7e8b43fa0e53018b7c9277f628c2af58ab40bf987ffbd46ff` | unchanged |
| rust bridge | `0afda1c6943feb5d98329299a57dd68404095efb0a792839779febed13ab8a7e` | unchanged |

**rpcs3.exe at R8.3c landing:** sha256
`34ec50d73d22eabb49fc9c6f3ddfebd55d58db76a61ce5e5c6dac14b0d20f851`
(R8.3b binary unchanged — R8.3c fix is replay-layer only).

**Pattern emerging across R8.3a/b/c.** Three consecutive
oracles each authored to exercise a deferred semantic, each
surfaced + co-fixed a real divergence. The "investigate when
happens + minimum fix after observed failure" policy
continues to yield clean per-oracle deliveries. Next deferred
semantics in the chain are 3+ DMA dispatches, explicit per-bit
clear via WrTagUpdate write (some Cell BE implementations),
then DMA list cmds.

**Out of R8.3c scope (deferred to R8.4+):** DMA list cmds
(landing progressively in **R8.4a (design + canary, done) →
R8.4b (writer + capture, done) → R8.4c (replay state machine,
next) → R8.4d (runtime bridge + 13th oracle)**), atomic
primitives, MFC barriers / fence bits, multi-SPU DMA
races, SPURS, explicit per-bit clear via WrTagUpdate write,
3+ DMA dispatch chains (mechanically in-scope), in-line state-
machine executor wiring. All prior hard rules carry forward.

---

## 8.4a R8.4a closure summary (2026-05-20)

DESIGN-only delivery + granular parser canary. R8.3d slot-fill
probe DEFERRED (RPCS3 model is observationally equivalent to
Rust persistent register for canonical patterns). New error
variant `UnsupportedMfcListCmd` differentiates list-DMA codes
(0x24/0x25/0x26 PUTL family, 0x44/0x45/0x46 GETL family) from
atomic/barrier codes which keep the generic `UnsupportedMfcCmd`.
Full R8.4 GETL roadmap documented in `docs/
SPU_DMA_MFC_R6_7_DESIGN.md` § 19 (4 phases a/b/c/d + e/f
optional).

Patch SHAs UNCHANGED. rpcs3.exe UNCHANGED. Workspace lib
+2 tests (140 differential lib tests). 12 oracles green.

---

## 8.4b R8.4b closure summary (2026-05-21)

R8.4b lands the writer + capture half of GETL list-DMA. The
fixture `single_spu_dma_getl_v1` builds, runs canonical
bridge OFF (`status=0xDF1EEA5A`), and captures a real
15-event JSONL with all schema-additive fields populated.
Replay still rejects via the R8.4a canary; promotion to
13th oracle defers to R8.4c.

**Scope (R8.4b only):**

- **CC0 fixture authoring**: `single_spu_dma_getl_v1.{self,
  jsonl}` + side-files. PPU prepares 2 EA buffers (128 B
  counting + 64 B constant 0x42 — same patterns as R8.2/3
  for max pool dedup). SPU builds a 2-element
  `list_element[]` in LS, dispatches GETL, sums, packs
  canonical `0xDF1EEA5A = ((sum1 << 16) | sum2) ^
  0xC0DEFADA`.
- **C++ writer extension**:
  - `SPUTraceJsonl.h`/`SPUTraceJsonl.cpp`: new public method
    `record_spu_mfc_getl_cmd(target_spu, pc, cmd, tag,
    descriptor_size, lsa_dest_base, eah, descriptor_lsa,
    descriptor_bytes, element_count, element_chunks[],
    element_sizes[], element_eals[])`. Writes
    `<sha>.dmalistdesc` for the descriptor + N
    `<sha>.dmachunk` for elements (REUSE existing pool).
    Emits `spu_mfc_cmd` JSONL with 5 additive fields.
    New private helper `write_dma_listdesc_side_file` —
    same shape as `write_dma_chunk_side_file` but
    `.dmalistdesc` extension.
  - `SPUThread.cpp` ch21 dispatch hook: detects cmd=0x44,
    validates (size % 8, ≤ 256 elements, descriptor in
    LS bounds, eah=0), reads descriptor from LS, parses
    each element (BE u16 ts + BE u32 ea), validates sb
    bit 0x80 not set (reject capture if so — R8.5+ scope),
    validates ts in [1, 0x4000] per element, snapshots
    each EA via `vm::_ptr<u8>(ea)`. Calls
    `record_spu_mfc_getl_cmd` BEFORE `process_mfc_cmd()`,
    emits `mfc_dma_complete` AFTER with
    `transferred_bytes = sum(ts)`.
- **Rust parser additive fields**: `SpuMfcCmdEvent` gains
  five `#[serde(default)] Option<>` fields. Existing
  GET/PUT events deserialize unchanged (all five default
  to `None`). GETL events deserialize all five as `Some(...)`.
  R8.4a canary `UnsupportedMfcListCmd` STILL fires at
  validate (replay/transform reject is preserved).
- **Tests**:
  - `r8_4b_getl_parses_additive_fields_but_still_rejects`:
    GETL JSONL deserializes correctly (all 5 fields
    populated) but `parse_jsonl_trace` still returns
    `UnsupportedMfcListCmd`.
  - `r8_4b_existing_get_put_traces_still_parse_with_none_list_fields`:
    regression guard — old GET/PUT events parse with the
    list fields defaulting to `None`.

**Captured trace shape (15 events):**

| seq | event | notes |
|---|---|---|
| 0 | spu_image (sha `f0878b15…`) | new spuimg pool entry |
| 1-6 | wrch ch16-21 | GETL dispatch params |
| 7 | `spu_mfc_cmd cmd=68` | + descriptor_sha256 (`79238773…`) + descriptor_size=16 + element_chunks=[`471fb943…`, `c422e707…`] + element_sizes=[128, 64] + element_eals=[0x10011180, 0x10011200] |
| 8 | mfc_dma_complete | transferred_bytes=192 (=128+64) |
| 9-11 | ch22/ch23/ch24 | tag wait (mask=0x08, ALL → 0x08) |
| 12 | wrch ch28 = 0xDF1EEA5A | canonical status |
| 13 | spu_stop 0x101 | |
| 14 | final_state | r48=0xDF1EEA5A |

**Pool stats:**
- `behavior-freeze/fixtures/spu/dma/`:
  - `471fb943…dmachunk` (R6.7+/R8.x dedup)
  - `c422e7070…dmachunk` (R8.2+ dedup)
  - **NEW** `79238773…dmalistdesc` (R8.4b descriptor)
- `behavior-freeze/fixtures/spu/images/`: 13 entries (was
  12; new spuimg `f0878b15…`)

**Patch SHAs at R8.4b landing:**

| Patch | sha256 | Δ vs R8.3c |
|---|---|---|
| scaffolding | `5c170508a73e492d42784036d61a972edab7a85b7ea7105d6dde388a5e67d6c0` | **BUMPED** (was `cda976d7…`; added `record_spu_mfc_getl_cmd` + `write_dma_listdesc_side_file` to SPUTraceJsonl.{h,cpp}) |
| runtime hooks | `745945f4872f7d83541aa74d9a065b6a6bc3785af73510d026e6643a0985cd96` | **BUMPED** (was `1f598d37…`; SPUThread.cpp ch21 dispatch detects + captures GETL) |
| rust bridge | `0afda1c6943feb5d98329299a57dd68404095efb0a792839779febed13ab8a7e` | unchanged (no bridge runtime GETL yet — R8.4d scope) |

**rpcs3.exe at R8.4b landing:** sha256
`3f2348de0e50b7dd4aadeac92aaec83b678cd84f7f7be88742835cc0c38a0b72`
(63,942,656 bytes; built 2026-05-21).

**Triple-symmetry regression for prior 6 fixtures**: green
across all (get / put / get_multi / get_any / get_tag_poll /
get_tag_immediate) with the new rpcs3.exe binary — GETL
writer changes are isolated and don't affect non-GETL paths.

**Out of R8.4b scope (deferred to R8.4c+):**

- `MfcReplayState::process_mfc_list_cmd` (R8.4c) — walks
  the captured descriptor, loads each element chunk into
  LS at cumulative offset.
- `apply_mfc_dma_pre_replay` extension for list cmds (R8.4c).
- `.dmalistdesc` side-file resolver in `dma_chunk.rs` (R8.4c).
- Lift `UnsupportedMfcListCmd` canary for cmd 0x44 (R8.4c).
- 13th oracle promotion (replay test against the captured
  trace, R8.4c).
- `rust_spu_set_dma_getl_callback` FFI + bridge handler
  (R8.4d). rpcs3.exe rebuild + bridge SHA bump (R8.4d).
- Triple-symmetry extension for `get_list` (R8.4d).
- PUTL family (R8.4e), GETLB/GETLF (R8.4f).
- Stall-and-notify bit (R8.5+).

---

## 8.4c R8.4c closure summary (2026-05-21)

R8.4c promotes the R8.4b capture-only GETL fixture to the
13th replay-validated oracle. Pure Rust-core delivery — no
C++ changes; rpcs3.exe binary unchanged from R8.4b
(`3f2348de…`). The state machine landed AND its dedicated
side-file resolver AND the parser canary lift AND the
replay test all in one coherent commit.

**Scope:**

- **`dma_chunk.rs` extensions**: new public functions
  `resolve_dma_listdesc_side_file`,
  `per_trace_dma_listdesc_path`,
  `canonical_dma_listdesc_path` + new constants
  `DMA_LISTDESC_EXTENSION = "dmalistdesc"` +
  `DMA_LISTDESC_SIZE_MAX = 0x800` (256 elements × 8). All
  five reuse `DmaChunkLoadError` variants (the failure
  modes are structurally identical). 6 new unit tests
  covering per-trace resolution, canonical fallback, size
  mismatch, SHA mismatch, too-large rejection, path helper
  layout.

- **`trace_fmt.rs` parser lift**: cmd 0x44 GETL now ACCEPTED
  via the additive-fields validation path
  (`validate_getl_additive_fields`); the other 5 list-DMA
  codes (PUTL 0x24, PUTLB 0x25, PUTLF 0x26, GETLB 0x45,
  GETLF 0x46) still surface `UnsupportedMfcListCmd`. New
  helper `validate_sha_hex_field` consolidates the
  64-lower-hex check across `ea_chunk_sha256` +
  `descriptor_sha256` + each `element_chunks[i]`. R8.4c
  enforces: all 5 list fields present, descriptor_size ==
  e.size, descriptor_size % 8 == 0, descriptor_size <=
  0x800, element_chunks/sizes/eals counts match
  descriptor_size / 8, per-element ts in (0, 0x4000], per-
  element chunk SHA = 64-hex. **GET/PUT events with stray
  list fields rejected** (writer-bug defense). 4 new
  parser tests; old `reject_mfc_list_cmds_with_granular_canary`
  test updated to iterate only the 5 still-unsupported
  codes (0x44 removed since GETL now accepts).

- **`mfc_replay.rs` state machine extension**: new public
  method `process_mfc_list_cmd` on `MfcReplayState`.
  Validates GETL cmd + pending-packet cross-check (ch16-20)
  + all 5 list fields present + descriptor side-file
  loaded via `resolve_dma_listdesc_side_file`. Parses each
  8-byte BE slot (sb + pad + u16 ts + u32 ea). Per-element
  defensive checks: sb bit 0x80 (stall-and-notify) MUST
  be 0 (R8.5+ scope), ts in (0, 0x4000], descriptor ts/ea
  MUST match trace event's `element_sizes[i]`/`element_eals[i]`.
  Loads each element chunk via existing
  `resolve_dma_chunk_side_file` (REUSE — content-addressed
  pool dedup applies). Copies bytes into LS at cumulative
  offset (`lsa_base + sum of prior ts`). Registers
  in-flight tag with size = sum of ts (so the matching
  `mfc_dma_complete.transferred_bytes` check matches Cell
  BE atomic-list-completion contract). 4 new error variants
  on `MfcReplayError`: `MissingGetlListFields`,
  `GetlDescriptorElementInvalid`,
  `GetlTransferredBytesMismatch`,
  `GetlDestinationOutOfRange`.

- **`process_mfc_cmd_pre_replay` dispatch**: 0x44 GETL
  routes to `process_mfc_list_cmd`. The existing
  `apply_mfc_dma_pre_replay` helper inherits the routing
  automatically (it already called `process_mfc_cmd_pre_replay`
  for every `SpuMfcCmd` event since R8.1 PUT).

- **`process_wrch` ch::MFC_CMD update**: defensive check
  on the wrch value (ch21) now accepts 0x44 alongside
  0x40 GET and 0x20 PUT. Old canary test moved from
  0x44 GETL → 0x45 GETLB.

- **13th oracle promotion**: new
  `rust/rpcs3-spu-recompiler/tests/single_spu_dma_getl_v1_replay.rs`
  loads the R8.4b-captured JSONL, drives the full
  pipeline (parse → per-SPU transform → SpuProgram with
  r3/r4 seeded for PSL1GHT arg0/arg1 → pre-replay GETL
  via `apply_mfc_dma_pre_replay` → Interpreter +
  Recompiler replay → `diff_snapshots.is_identical()`).
  Validates canonical status `0xDF1EEA5A` and post-replay
  LS at both element offsets matches the captured chunks.

**Triple-symmetry status:**

- **Replay**: passes (Interpreter ↔ Recompiler byte-identical).
- **Bridge OFF**: passes (C++ LLVM executor produces canonical).
- **Bridge ON**: NOT YET (R8.4d scope — Rust bridge has no
  GETL callback installed; would fall back to C++ at the
  GETL dispatch). Triple-symmetry expansion for
  `--fixture get_list` defers to R8.4d.

**Patch SHAs at R8.4c landing:**

| Patch | sha256 | Δ vs R8.4b |
|---|---|---|
| scaffolding | `5c170508a73e492d42784036d61a972edab7a85b7ea7105d6dde388a5e67d6c0` | unchanged |
| runtime hooks | `745945f4872f7d83541aa74d9a065b6a6bc3785af73510d026e6643a0985cd96` | unchanged |
| rust bridge | `0afda1c6943feb5d98329299a57dd68404095efb0a792839779febed13ab8a7e` | unchanged |

R8.4c is Rust-core only. C++ patches and rpcs3.exe binary
both unchanged from R8.4b.

**rpcs3.exe at R8.4c landing:** sha
`3f2348de0e50b7dd4aadeac92aaec83b678cd84f7f7be88742835cc0c38a0b72`
(R8.4b binary unchanged).

**Out of R8.4c scope (deferred to R8.4d+):**

- Runtime bridge: `rust_spu_set_dma_getl_callback` FFI +
  `bridge_dma_getl_callback` C++ handler that reads
  descriptor from SPU's LS and copies each element via
  `vm::_ptr<u8>(ea)`. Bumps bridge SHA + rpcs3.exe
  rebuild.
- `check_triple_symmetry.py --fixture get_list` extension.
- PUTL family (R8.4e), GETLB/GETLF (R8.4f).
- Stall-and-notify bit 0x80 (R8.5+).
- 3+ element GETL fixtures (mechanically in-scope; no
  fixture yet).

---

## 8.4d R8.4d closure summary (2026-05-21)

R8.4d closes the R8.4 GETL roadmap by landing the runtime
bridge half. Bridge ON now delegates
`single_spu_dma_getl_v1.self` end-to-end with no fallback;
all three execution paths (bridge OFF / bridge ON / replay)
converge byte-identical on canonical TTY
`[dma_getl_v1] OK cause=0x1 status=0xdf1eea5a`.

**Scope:**

- **`rpcs3-spu-thread::SpuChannels`** — new
  `dma_getl_callback: Option<DmaGetlCallback>` field (7-arg
  `#[repr(C)]` struct: func ptr + opaque `user_data`). The
  `refuse_mfc` gate is relaxed when ANY of the 3 callbacks
  (GET/PUT/GETL) is installed, so list-GET dispatch routes
  to the new callback instead of MfcUnsupported.

- **`rpcs3-spu-interpreter`** — wrch ch21 dispatch extended:
  `cmd == 0x44` GETL now early-returns through the callback
  path with its own validation (size > 0 && size <= 0x800
  && size % 8 == 0; descriptor_lsa + size <= 256 KiB;
  eah == 0; tag < 32). Callback is invoked with descriptor
  source pointer + destination base pointer + descriptor_size
  + tag. On rc == 0 the interpreter advances pc + 4 and
  queues `1 << tag` into `mfc_tag_stat_queue`. Any unhandled
  cmd / failed callback still surfaces `MfcUnsupported`.

- **`rpcs3-spu-ffi`** — new C ABI
  `rust_spu_set_dma_getl_callback` + new typedef
  `rust_spu_dma_getl_cb_t` (7-arg signature). Returns 0
  success / -1 null handle / -100 panic. `func = NULL`
  clears an installed callback. Two new FFI tests:
  `rust_spu_runtime_dma_getl_callback_round_trip` (install/
  clear/null-handle invariants under `CALLBACK_TEST_MUTEX`)
  and `rust_spu_get_put_getl_callbacks_coexist` (all 3
  callbacks installable simultaneously; refuse_mfc state
  observable). All 28 FFI tests pass (up from 26 pre-R8.4d).

- **`SPURustBridge.cpp`** — new
  `bridge_dma_getl_callback` (7-arg). Validates
  `descriptor_ls_ptr` + `descriptor_size % 8 == 0` then walks
  each 8-byte BE slot: rejects `sb & 0x80` (stall-and-notify,
  R8.5+ scope), `ts == 0`, `ts > 0x4000`, cumulative LS
  overflow, null `vm::_ptr<u8>(ea)`. On success, memcpys
  `ts` bytes from EA to `dest_ls_ptr + cumulative_offset`
  per element. Logs
  `[Rust SPU bridge] R8.4d DMA GETL dispatched: cmd=0x44 ...
  transferred_bytes=%u tag=%u ... tag-stat 0x%x queued`.
  Installed in `try_delegate_execution` alongside GET + PUT
  (rejected install fails the delegation gracefully).

**Patch SHAs at R8.4d landing:**

| patch | sha256 | status |
|---|---|---|
| scaffolding | `5c170508a73e492d42784036d61a972edab7a85b7ea7105d6dde388a5e67d6c0` | unchanged |
| runtime hooks | `745945f4872f7d83541aa74d9a065b6a6bc3785af73510d026e6643a0985cd96` | unchanged |
| rust bridge | `d2d531850f4743b240fb59573695ea247614d0aeecb8624726206937be52d2e5` | **bumped** (was `0afda1c69…`) |

**rpcs3.exe at R8.4d landing:** sha256
`0f5cc2bec90986da7aa1eb9d601230c8986563a92e5027ac991f5742507b749d`
(`3f2348de…` → `0f5cc2bec90986da…` after R8.4d rebuild with
the new bridge patch applied). `rpcs3_spu_ffi.lib` rebuilt
with R8.4d additions and mirrored from rpcs3-master to
upstream-clean.

**Triple-symmetry gate (`check_triple_symmetry.py`):**

- New `GET_LIST_FIXTURE` entry (canonical TTY
  `[dma_getl_v1] OK cause=0x1 status=0xdf1eea5a`,
  delegation marker `DMA GETL dispatched`,
  rust_log_intro `R8.4d DMA GETL`).
- All 7 fixtures green (get / put / get_multi / get_any /
  get_tag_poll / get_tag_immediate / get_list). Bridge ON
  GETL: `DELEGATED EXECUTION OK (total_steps=1598)`, no
  fallback. R6.7 A.5 + R8.1 + R8.2 + R8.3a/b/c + R8.4c
  oracles unchanged.

**Hard rules preserved (per R6.7 / R7 / R8.1 charter):**

- No fake DMA, no fake list descriptor, no manual JSONL
  edits, no fake tag-stat.
- No v4 / SPURS promotion.
- No PUTL, no GETLB/GETLF, no stall-and-notify (sb bit 0x80
  is REJECTED, bridge falls back honestly).
- Runtime scope limited to GETL (cmd 0x44) only; all other
  list / atomic / barrier / multi-SPU paths surface
  `MfcUnsupported`.
- Default bridge OFF preserved; R8.4d only activates on
  `RPCS3_SPU_RUST_BRIDGE=1`.

**Out of R8.4d scope (deferred to R8.4e+):**

- PUTL (R8.4e) — **LANDED** (see § 8.4e below).
- GETLB / GETLF / PUTLB / PUTLF (R8.4f) — barrier / fence
  variants of GETL/PUTL.
- Stall-and-notify bit 0x80 (R8.5+) — needs SPU-to-PPU
  signaling integration via `mfc_notify` channel.
- 3+ element fixtures + descriptor-size edge cases (256-
  element max).

---

## 8.4e R8.4e closure summary (2026-05-21)

R8.4e lands MFC PUTL list-DMA (cmd=0x24, LS → EA) end-to-end
in one cycle: writer + capture + replay + runtime bridge +
triple-symmetry, NO engine fixes (R8.4c/d's plumbing already
covers the symmetric inverse direction). New 14th replay-
validated oracle `single_spu_dma_putl_v1`; bridge ON delegates
`single_spu_dma_putl_v1.self` end-to-end producing canonical
TTY `[dma_putl_v1] OK cause=0x1 spu=0xc0ffeeba
ea_status=0xa12fda7e`, byte-identical to bridge OFF + replay.

**Scope:**

- **SPUTraceJsonl.{h,cpp}** — relaxed
  `record_spu_mfc_getl_cmd` cmd guard from `cmd != 0x44u`
  to `cmd != 0x44u && cmd != 0x24u` (wire format identical
  between directions). Doc comments updated to mention the
  R8.4e PUTL extension.

- **SPUThread.cpp** — ch21 dispatcher: `capture_getl_pre`
  predicate extended to `getl_or_putl` (matches 0x44 OR
  0x24). Per-element snapshot path branches: GETL reads
  from `vm::_ptr<u8>(ea)`, PUTL reads from
  `this->ls + mfc_lsa + cumulative_offset` (LS source bytes)
  with cumulative LS bounds check. `mfc_dma_complete`
  `transferred_bytes` computation already worked
  unchanged (re-parses descriptor for sum of ts).

- **`rpcs3-spu-differential::trace_fmt`** — added
  `MFC_PUTL_CMD = 0x24`; removed from
  `MFC_LIST_CMDS_UNSUPPORTED`. Accept clause now reads
  `cmd != GET && cmd != PUT && cmd != GETL && cmd != PUTL`.
  `validate_getl_additive_fields` invoked for BOTH GETL and
  PUTL (wire format identical). 1 new test
  `r8_4e_putl_parses_and_validates`; canary test iterator
  drops 0x24.

- **`rpcs3-spu-differential::mfc_replay`** — wrch ch21 cmd
  guard accepts 0x24; `process_mfc_cmd_pre_replay` dispatch
  routes cmd 0x24 to `process_mfc_list_cmd`. Inside
  `process_mfc_list_cmd`: new local `is_putl` flag;
  cmd-validation defensively accepts 0x24 alongside 0x44;
  per-element processing loads + validates the .dmachunk
  via the existing loader for BOTH directions; the LS write
  step `ls[lo..hi].copy_from_slice(&chunk_bytes)` is gated
  by `if !is_putl` — PUTL does NOT mutate LS (the SPU's own
  bytecode populates LS source pre-dispatch; replay verifies
  the source bytes match the captured chunk POST-execution).

- **`rpcs3-spu-thread`** — new `dma_putl_callback:
  Option<DmaPutlCallback>` field on `SpuChannels` (#[repr(C)]
  7-arg struct: func ptr + opaque user_data). The
  `refuse_mfc` gate is relaxed when ANY of 4 callbacks
  (GET/PUT/GETL/PUTL) is installed.

- **`rpcs3-spu-interpreter`** — wrch ch21 dispatch extended:
  `cmd == 0x24` PUTL routes through the same validation
  envelope as GETL (size > 0, size <= 0x800, size % 8 == 0,
  descriptor_lsa + size <= 256 KiB, eah == 0, tag < 32) and
  invokes `dma_putl_callback` with `src_ls_ptr` (a
  `*const u8` pointing at the LS source region).

- **`rpcs3-spu-ffi`** — new C ABI
  `rust_spu_set_dma_putl_callback` + typedef
  `rust_spu_dma_putl_cb_t` (7-arg with `const uint8_t*
  src_ls_ptr`). Returns 0 success / -1 null handle / -100
  panic. `func = NULL` clears. Two new FFI tests:
  `rust_spu_runtime_dma_putl_callback_round_trip` and
  `rust_spu_get_put_getl_putl_callbacks_coexist`. All 30
  FFI tests pass (up from 28 pre-R8.4e).

- **`SPURustBridge.cpp`** — new
  `bridge_dma_putl_callback` (7-arg, mirrors
  `bridge_dma_getl_callback` but copy direction is LS → EA).
  Validates `descriptor_ls_ptr` / `src_ls_ptr` / each 8-byte
  BE slot (rejects `sb & 0x80`, `ts == 0`, `ts > 0x4000`,
  cumulative LS overflow, null `vm::_ptr<u8>(ea)`).
  memcpys per element from `src_ls_ptr +
  cumulative_offset` to `vm::_ptr<u8>(ea)`. Logs
  `R8.4e DMA PUTL dispatched ... transferred_bytes=192
  tag=...`. Installed in `try_delegate_execution`
  alongside GET + PUT + GETL.

**SHAs at R8.4e landing:**

| patch | sha256 | status |
|---|---|---|
| scaffolding | `402c2d139526a4efd592ba6f052f0c59067aaf09b9d079d75d03ca4a09fe4e5a` | **bumped** (was `5c170508…`) |
| runtime hooks | `3760b78c8854dd83157f5ef5e501ae85b4fd9b46dc143ae226fb19703bf4a974` | **bumped** (was `745945f4…`) |
| rust bridge | `e09b9c40b3187f89b559c5fcde949a86491974c836525338d51dd2e99600850e` | **bumped** (was `d2d531850f…`) |

**rpcs3.exe at R8.4e landing:** sha256
`64ff57a1248ebb857fcffda2ff392fffa432deb7f0dd75deb07cbb670152cd33`
(`0f5cc2bec9…` → `64ff57a1…` after R8.4e rebuild).

**Fixture artifacts:**

- `.self`: 939,511 bytes sha
  `d7efc5629cca9fdfb05d07271d4b1813d7cf40c45a6066c1135acd27a9ae76b9`
  (NEW).
- `.jsonl`: 15 events, ~3.1 KB (NEW).
- `.spuimg`:
  `3474dea93b83f18920eced5d37725ac19b3ffda6de67c0227a8496bd3a1189dd`
  (NEW — different SPU bytecode).
- `.dmalistdesc`:
  `79238773912c38db59bf192072b2d89fcb1757d7be59870765cc2be911271126`
  (REUSES R8.4b GETL's descriptor — lucky EA layout match).
- `.dmachunk` element 0 (128 B counting):
  `471fb943aa23c511f6f72f8d1652d9c880cfa392ad80503120547703e56a2be5`
  (REUSES R6.7 GET / R8.1 PUT / R8.2 / R8.3a-c / R8.4b-d).
- `.dmachunk` element 1 (64 B constant 0x42):
  `c422e7070cb1cb455b5de9afee0d975e303d0239c72030cd7414ab5c382d3ae8`
  (REUSES R8.2 / R8.3a-c / R8.4b-d).

ZERO new `.dmachunk` files (perfect content-addressed pool
dedup). ZERO new `.dmalistdesc` files (lucky EA layout match
with R8.4b GETL). ONE new `.spuimg` (different SPU bytecode).

**Triple-symmetry gate (`check_triple_symmetry.py`):**

- New `PUT_LIST_FIXTURE` entry (canonical TTY
  `[dma_putl_v1] OK cause=0x1 spu=0xc0ffeeba ea_status=0xa12fda7e`,
  delegation marker `DMA PUTL dispatched`, rust_log_intro
  `R8.4e DMA PUTL`).
- All 8 fixtures green (get / put / get_multi / get_any /
  get_tag_poll / get_tag_immediate / get_list / put_list).
  Bridge ON PUTL: `DELEGATED EXECUTION OK
  total_steps=1394`, no fallback.

**Hard rules preserved (per R6.7 / R7 / R8.1 / R8.4d charter):**

- No fake JSONL, no manual trace edits, no fake DMA, no fake
  list descriptor, no fake tag-stat.
- No v4 / SPURS promotion.
- No GETLB / GETLF / PUTLB / PUTLF (still surface
  `UnsupportedMfcListCmd`).
- No stall-and-notify (sb bit 0x80 is REJECTED, bridge falls
  back honestly).
- Runtime scope limited to GETL + PUTL (cmds 0x44 / 0x24)
  only; all other list / atomic / barrier / multi-SPU paths
  surface `MfcUnsupported`.
- Default bridge OFF preserved; R8.4e only activates on
  `RPCS3_SPU_RUST_BRIDGE=1`.
- Existing 13 oracles remain green.

**Workspace tests:** 5673 pass, 0 fail (up from 5670 pre-R8.4e
— 3 new tests added: `r8_4e_putl_parses_and_validates`,
`rust_spu_runtime_dma_putl_callback_round_trip`,
`rust_spu_get_put_getl_putl_callbacks_coexist`).

**Out of R8.4e scope (deferred to R8.4f+):**

- PUTLB (0x25) / PUTLF (0x26) / GETLB (0x45) / GETLF (0x46)
  — list + barrier/fence variants.
- Stall-and-notify bit 0x80 (R8.5+) — needs SPU-to-PPU
  signaling integration via `mfc_notify` channel.
- 3+ element fixtures + descriptor-size edge cases.

---

## 8.4f-a R8.4f-a closure summary (2026-05-21)

R8.4f-a adds MFC GETLB (cmd=0x45) and GETLF (cmd=0x46) — the
barrier/fence variants of GETL — end-to-end in a single phase
with NO new state machine code, NO new bridge callback, and
NO new FFI function. The work is purely a cmd-code acceptance
extension across all layers, justified by direct inspection
of RPCS3's reference C++ implementation.

**Inspection findings (justification for the reuse strategy):**

1. **`MFC.h:14`**: `MFC_GETL_CMD = 0x44, MFC_GETLB_CMD = 0x45,
   MFC_GETLF_CMD = 0x46`. Barrier/fence are bit modifiers on
   top of GETL (`MFC_BARRIER_MASK = 0x01`,
   `MFC_FENCE_MASK = 0x02`).
2. **`SPUThread.cpp:4935-4937`**: GETL/GETLB/GETLF share the
   same `case` block in `process_mfc_cmd()` — identical
   dispatch path through `do_list_transfer`.
3. **`SPUThread.cpp:2887`** (`do_list_transfer`):
   `transfer.cmd = MFC{static_cast<u8>(args.cmd & ~0xf)}` —
   strips the bottom 4 bits (barrier 0x01, fence 0x02, list
   0x04, start 0x08) before performing the per-element copy.
   The per-element data path is byte-identical for all three
   GETL variants (all become base cmd=0x40 GET).
4. **`SPUThread.cpp:2819`** (`do_dma_check`): barrier/fence
   bits affect `mfc_barrier`/`mfc_fence` register persistence,
   which gates SUBSEQUENT commands on the same tag. For a
   single-SPU fresh-tag single-dispatch fixture, `mfc_barrier`
   = `mfc_fence` = 0 at entry; the check passes immediately
   and there are no subsequent commands to be affected.

**Conclusion:** For the R8.4f-a fixture pattern (one
list-DMA dispatch on a fresh tag, no other in-flight commands
on the same tag), GETLB/GETLF are byte-identical to GETL.

**Scope:**

- **`SPUTraceJsonl.{h,cpp}`** — relaxed
  `record_spu_mfc_getl_cmd` cmd guard to accept cmd 0x44
  (GETL) / 0x24 (PUTL) / 0x45 (GETLB) / 0x46 (GETLF). Doc
  comment updated.

- **`SPUThread.cpp`** — ch21 dispatcher: `getl_family`
  predicate now matches 0x44/0x45/0x46; per-element snapshot
  routes to the EA-source branch (GET base direction).

- **`rpcs3-spu-differential::trace_fmt`** — added
  `MFC_GETLB_CMD = 0x45`, `MFC_GETLF_CMD = 0x46`; removed
  both from `MFC_LIST_CMDS_UNSUPPORTED` (now `[0x25, 0x26]`).
  New helper `is_mfc_list_supported_cmd` consolidates the
  4-cmd accept set. Validate predicates updated to use the
  helper. 2 new parser tests
  (`r8_4f_a_getlb_parses_and_validates`,
  `r8_4f_a_getlf_parses_and_validates`); canary iterator
  reduced to `[0x25, 0x26]`.

- **`rpcs3-spu-differential::mfc_replay`** — wrch ch21 cmd
  guard accepts 0x45/0x46; `process_mfc_cmd_pre_replay`
  dispatches 0x45/0x46 to `process_mfc_list_cmd` (same as
  GETL); `process_mfc_list_cmd` defensive cmd guard accepts
  the GETL-family set; LS-write branch unchanged (GETLB/GETLF
  use GETL's `is_putl == false` path). New canary in the
  unsupported-cmd test is PUTLB (0x25).

- **`rpcs3-spu-interpreter`** — ch21 dispatch routes 0x45 and
  0x46 through the same GETL validation envelope and invokes
  `dma_getl_callback` (no separate FFI for GETLB/GETLF).

- **`SPURustBridge.cpp`** — `bridge_dma_getl_callback`
  unchanged at the data-path level; log message generalized
  to mention all 3 supported variants (GETL/GETLB/GETLF route
  here per R8.4f-a). No new bridge function; no FFI changes.

**Patch SHAs at R8.4f-a landing:**

| patch | sha256 | status |
|---|---|---|
| scaffolding | `5085c4afaa5dd2df7526999b7f7f0ed33b763ce4c66d4decef55a2fa2b427364` | **bumped** (was `402c2d13…`) |
| runtime hooks | `67bef0455eeedc511443c7d283841fd5080d703dac0b8bc11743b97a971a3dc8` | **bumped** (was `3760b78c…`) |
| rust bridge | `b9e5e977bc3f97b5e1a86f56a5d6affd79d831f3c9f4b47226511a242a45a713` | **bumped** (was `e09b9c40…`) |

**rpcs3.exe at R8.4f-a landing:** sha256
`f3d4e85f3d2e375bb9d58e8414a3e2f9699c3a25a6210eba998d3a869ee665ac`
(`64ff57a1…` → `f3d4e85f…` after R8.4f-a rebuild).

**Fixture artifacts (per fixture):**

GETLB:
- `.self`: 939,514 bytes sha `f490be23d1af05f8…`
- `.jsonl`: 15 events, ~3 KB
- `.spuimg`:
  `9ab0058de577e6fd8aa1caa6fde58b3c8f744a7d0afc018cab724df05c19df99`
  (NEW)
- `.dmalistdesc`: REUSES R8.4b/c GETL's `79238773…`
- `.dmachunk` elements: REUSE canonical pool (471fb943… +
  c422e7070cb1…)

GETLF:
- `.self`: 939,514 bytes sha `f3201485f5266327…`
- `.jsonl`: 15 events, ~3 KB
- `.spuimg`:
  `3bdc07e4bf7c5a05505b73d800431b9d5cf46b126fdf45474f26f3777ea66b0d`
  (NEW)
- `.dmalistdesc`: REUSES same as GETLB
- `.dmachunk` elements: REUSE same as GETLB

**Triple-symmetry gate (`check_triple_symmetry.py`):**

- New `GET_LIST_B_FIXTURE` + `GET_LIST_F_FIXTURE` entries.
- All 10 fixtures green (get / put / get_multi / get_any /
  get_tag_poll / get_tag_immediate / get_list / put_list /
  get_list_b / get_list_f). Bridge ON GETLB + GETLF:
  `DELEGATED EXECUTION OK total_steps=1598` (identical to
  GETL — same SPU bytecode shape, only cmd code differs at
  the single ch21 wrch).

**Hard rules preserved:**

- No fake JSONL / no manual trace edits / no fake DMA / no
  fake barrier/fence behavior.
- No v4 / SPURS promotion.
- No PUTLB / PUTLF (R8.4f-b).
- No stall-and-notify (R8.5+).
- Existing 14 oracles unchanged + green.

**Workspace tests:** 5675 pass, 0 fail (+2 R8.4f-a tests).

**Out of R8.4f-a scope (deferred):**

- PUTLB (cmd=0x25) / PUTLF (cmd=0x26) — symmetric inverse,
  R8.4f-b. Same one-cycle pattern expected: extend writer +
  parser + replay + interpreter to accept 0x25/0x26, route
  through the existing PUTL callback. Test the symmetry
  assumption by direct C++ inspection first.
- Stall-and-notify bit 0x80 (R8.5+).
- 3+ element fixtures + descriptor edge cases.

---

## 8.4f-b R8.4f-b closure summary (2026-05-21)

R8.4f-b adds MFC PUTLB (cmd=0x25) and PUTLF (cmd=0x26) —
the barrier/fence variants of PUTL — end-to-end in a single
phase. Mirrors R8.4f-a's REUSE-GETL strategy on the symmetric
inverse direction (LS → EA). **Closes the entire 6-code MFC
list-DMA family**: GETL (R8.4c) + GETLB/GETLF (R8.4f-a) +
PUTL (R8.4e) + PUTLB/PUTLF (R8.4f-b) — all replay-validated +
bridge-delegated end-to-end.

**Scope:**

- **SPUTraceJsonl.{h,cpp}** — `record_spu_mfc_getl_cmd` cmd
  guard relaxed to accept 0x44/0x24/0x45/0x46/0x25/0x26.
- **SPUThread.cpp** — ch21 dispatcher: new `putl_family`
  predicate matches 0x24/0x25/0x26; per-element `is_putl`
  flag covers all 3 PUTL-family codes to take the LS-source
  snapshot branch.
- **`rpcs3-spu-differential::trace_fmt`** — added
  `MFC_PUTLB_CMD = 0x25`, `MFC_PUTLF_CMD = 0x26`;
  `MFC_LIST_CMDS_UNSUPPORTED = &[]` (empty). Helper
  `is_mfc_list_supported_cmd` now covers all 6 codes.
  2 new parser tests (`r8_4f_b_putlb_parses_and_validates`,
  `r8_4f_b_putlf_parses_and_validates`); canary test
  iterator is now empty by design.
- **`rpcs3-spu-differential::mfc_replay`** — wrch ch21 guard
  accepts 0x25/0x26; `process_mfc_cmd_pre_replay` dispatches
  0x25/0x26 to `process_mfc_list_cmd`; `process_mfc_list_cmd`
  defensive guard via `is_putl_family ||
  is_getl_family`; `is_putl` bound to family flag to keep
  the existing LS-handling branch logic. Canary in
  `process_wrch_rejects_unsupported_cmd_code` moved to
  GETLLAR (0xD0).
- **`rpcs3-spu-interpreter`** — ch21 dispatch accepts 0x25/
  0x26 alongside 0x24; `is_putl` extended; routes through
  `dma_putl_callback`.
- **`SPURustBridge.cpp`** — `bridge_dma_putl_callback` data
  path unchanged; log message generalized to mention all 3
  PUTL variants (PUTL/PUTLB/PUTLF route here per R8.4f-b).
  NO new bridge function; NO new FFI.

**Patch SHAs at R8.4f-b landing:**

| patch | sha256 | status |
|---|---|---|
| scaffolding | `d9d60bfa01a942c0523ac4ae5f8307c9bd89c57efc0736b432dc1e38db1d482c` | **bumped** (was `5085c4af…`) |
| runtime hooks | `e53518c4393e416d08ad09257ddf0af9c92ff7011a3f0524ff1db9c70593519e` | **bumped** (was `67bef045…`) |
| rust bridge | `106ddede745c6487e3b1f4dbe61c272beb3c16835c164a952a0799ed4de3e899` | **bumped** (was `b9e5e977…`) |

**rpcs3.exe at R8.4f-b landing:** sha256
`85e6fe8d09f7ae02d0cc258f8087a0eb46ab25bde3c66e8eda0050682626f428`
(`f3d4e85f…` → `85e6fe8d…` after R8.4f-b rebuild).

**Fixture artifacts:**

PUTLB (17th):
- `.self`: 939,514 bytes sha `120f43ba27eb2123…`
- `.jsonl`: 15 events, ~3 KB
- `.spuimg`: NEW
  `1a659f3225c59df282aa2d17c99404cf46a23ac42bb54b800d5b7b369dab6126`
- `.dmalistdesc` + `.dmachunk`: REUSE canonical pool

PUTLF (18th):
- `.self`: 939,514 bytes sha `94e5474e8f62a4e0…`
- `.jsonl`: 15 events
- `.spuimg`: NEW
  `54640ed6b7fc956453be233b3239b2a8787fdd0001a65a962b2d60070706ab17`
- `.dmalistdesc` + `.dmachunk`: REUSE canonical pool

ZERO new `.dmachunk` / `.dmalistdesc` (perfect content-
addressed pool dedup); TWO new `.spuimg`.

**Triple-symmetry gate:**

- New `PUT_LIST_B_FIXTURE` + `PUT_LIST_F_FIXTURE` entries.
- All 12 fixtures green. Bridge ON PUTLB + PUTLF:
  `DELEGATED EXECUTION OK total_steps=1394` (identical to
  PUTL), no fallback.

**Workspace tests:** 5677 pass, 0 fail (+2 R8.4f-b tests).

**Out of R8.4f-b scope (deferred to R8.5+):**

- Stall-and-notify bit 0x80 (descriptor `sb` bit) — needs
  SPU↔PPU signaling integration via `mfc_notify` channel.
- 3+ element list fixtures (mechanically in-scope; no
  fixture authored yet).
- PUTRL family (0x34/0x35/0x36) and PUTRL barrier/fence
  combos — REPLACED PUT variants (atomic-replace; different
  semantics from plain PUT, may not be a simple REUSE-PUTL
  extension).
- Atomic MFC primitives (GETLLAR 0xD0, PUTLLC 0xB4, etc).
- Barrier / EIEIO / SYNC sync cmds (0xC0/0xC8/0xCC).
- Multi-SPU DMA + SPURS workloads.

---

## 9. R8+ recommended next scope (refreshed 2026-05-23 post R8.5e — wave R8.5 closed)

> **Authoritative roadmap.** This section was originally written at R7
> closure (2026-05-18) and predated the actual landed sequence
> (R8.1 PUT → R8.2 multi-DMA GET → R8.3a/b/c TagStat modes →
> R8.4 list-DMA family → R8.5 stall-and-notify). The numbering
> below was rewritten to match what has landed and the workstreams
> currently contemplated. **Do not begin R8.6+ work without
> re-reading the hard rules in § 9.5 and § 20.7 of
> `SPU_DMA_MFC_R6_7_DESIGN.md`.**

### 9.1 Recap of what landed (R8.1 → R8.5e)

| Phase | Scope | Oracle # | Closure date |
|-------|-------|----------|--------------|
| **R8.1** | MFC PUT (LS → EA) — symmetric inverse of R7 GET | 8th | 2026-05-19 |
| **R8.2** | Multi-DMA GET (2 in-flight tags + ALL wait) | 9th | 2026-05-20 |
| **R8.3a** | TagStat ANY-wait + ch24 drain-aggregate engine fix | 10th | 2026-05-20 |
| **R8.3b** | Repeated RdTagStat polling + persistent `completed_tags` engine fix | 11th | 2026-05-20 |
| **R8.3c** | TagStat IMMEDIATE + replay clear-on-read engine fix | 12th | 2026-05-20 |
| **R8.4a** | Design + granular parser canary for list-DMA cmds | — | 2026-05-20 |
| **R8.4b** | GETL writer extension + first real list-DMA capture (cmd 0x44) | (capture) | 2026-05-21 |
| **R8.4c** | GETL replay state machine + 13th oracle promotion | 13th | 2026-05-21 |
| **R8.4d** | Runtime bridge GETL callback + triple-symmetry | (delegation) | 2026-05-21 |
| **R8.4e** | PUTL end-to-end (writer + replay + bridge in one cycle) | 14th | 2026-05-21 |
| **R8.4f-a** | GETLB + GETLF via REUSE-GETL (barrier/fence variants) | 15th + 16th | 2026-05-21 |
| **R8.4f-b** | PUTLB + PUTLF via REUSE-PUTL — **6-code list-DMA family complete** | 17th + 18th | 2026-05-21 |
| **R8.5a** | Research-only stall-and-notify feasibility | — | 2026-05-22 |
| **R8.5b** | Writer/parser capture surface unlock (Schema A: reuse SpuRdch ch25 / SpuWrch ch26) | — | 2026-05-22 |
| **R8.5c** | Rust replay state machine for stall handshake (`process_mfc_list_cmd` transfer-then-stall + `process_spu_rdch_list_stall_stat` + `process_spu_wrch_list_stall_ack`) | — | 2026-05-23 |
| **R8.5d D.1.a + D.1.b** | Rust FFI + C++ bridge runtime GETL stall surface | — | 2026-05-23 |
| **R8.5d D.2** | Bridge stall handshake extended to PUTL family (unify `GetlPartialState` → `ListPartialState{is_put}`) | — | 2026-05-23 |
| **R8.5d D.3** | `single_spu_dma_getl_stall_v1` CC0 source authored | (source) | 2026-05-23 |
| **R8.5d D.6** | 19th oracle promotion (`getl_stall_v1` replay) + `initial_mfc_list_stall_mask` plumbing fix | 19th | 2026-05-23 |
| **R8.5e E.3** | `single_spu_dma_putl_stall_v1` CC0 source authored | (source) | 2026-05-23 |
| **R8.5e E.6** | 20th oracle promotion (`putl_stall_v1` replay) — D.6 plumbing carried over zero-code-change | 20th | 2026-05-23 |

**Current state:** **20 replay-validated SPU oracles.** Full 6-code
MFC list-DMA family (GETL/GETLB/GETLF/PUTL/PUTLB/PUTLF) +
stall-and-notify (GETL stall + PUTL stall) end-to-end supported
across parser, replay state machine, and runtime bridge (bridge
default OFF; bridge ON validated for the 6-code family triple-
symmetric on 8 fixtures; stall fixtures replay-validated only —
bridge-ON triple-symmetry on stall fixtures deferred until a
real workload exercises that path). `MFC_LIST_CMDS_UNSUPPORTED`
parser slice is empty.

### 9.2 R8.5 wave — list-DMA stall-and-notify (CLOSED)

R8.5 covered the `sb & 0x80` stall-and-notify bit on list
descriptor elements — the single semantic divergence R8.4
explicitly deferred. SPU↔MFC handshake via `ch25
MFC_RdListStallStat` (destructive read, returns bitmask of
stalled tags) and `ch26 MFC_WrListStallAck` (write, takes a tag
id). **No PPU handshake required** in the minimal CC0 fixture
— the SPU acks itself. Full design preserved in
`SPU_DMA_MFC_R6_7_DESIGN.md` § 20.

| Phase | Scope | Outcome |
|-------|-------|---------|
| R8.5a | Research-only feasibility study | ✅ complete |
| R8.5b | Writer + parser unlock (Schema A — reuse ch25/ch26 SpuRdch/SpuWrch event kinds) | ✅ landed `1f5450b56` |
| R8.5c | Replay state machine (transfer-then-stall per Cell BE § 12.5; resume from `next_element_index`) | ✅ landed `a4d0d58f7` |
| R8.5d D.1.a + D.1.b | Rust FFI + C++ bridge runtime (GETL stall partial state + ack callback) | ✅ landed `b6c717b55` |
| R8.5d D.2 | Bridge PUTL stall handshake (unify `ListPartialState{is_put}`) | ✅ landed `8e4039677` |
| R8.5d D.3 + D.4 + D.5 + D.6 | `single_spu_dma_getl_stall_v1` source + Docker build + JSONL capture + 19th oracle | ✅ landed `35d637a3a` + `6936d7900` |
| R8.5e E.3 + E.4 + E.5 + E.6 | `single_spu_dma_putl_stall_v1` source + build + capture + 20th oracle | ✅ landed `c171c09b0` + `6e650c363` |

**Key architectural insight:** R8.5d D.6's `initial_mfc_list_stall_mask`
plumbing (SpuProgram → DmaPreReplayPlan → spu-thread, capturing
ch25 value before destructive read consumes it) was direction-
agnostic from the start. R8.5e E.6 PUTL stall test **passed on
first run with zero new code** — R8.5c's `is_putl=true` branch in
`process_mfc_list_cmd` and R8.5d D.2's `is_put=true` branch in
`bridge_dma_list_stall_ack_callback` were exercised end-to-end
against real-captured PUTL JSONL with only a new replay test file.

**Side-file pool stats post-R8.5e:**
- 20 `.jsonl` traces, 20 `.notes.md`
- Canonical chunks: 24 (1 new from PUTL stall: only the .spuimg)
- Canonical listdescs: shared between getl_stall + putl_stall
  (the descriptor BYTES are identical — cmd is not in the
  descriptor per Cell BE encoding)

### 9.3 Subsequent SPU scope (R8.6+)

| Phase | Scope | Notes |
|-------|-------|-------|
| **R8.6** | Multi-SPU DMA races on shared EA | The bridge's persistent-handle table is keyed by `lv2_id`; multi-SPU is a separate workstream. Defer until a CC0 multi-SPU fixture authored for the purpose forces the path. |
| **R8.7** | MFC atomic primitives (GETLLAR / PUTLLC / PUTLLUC / PUTQLLUC) | LL/SC reservation tracking — bridge needs a per-`raddr` reservation map shared with RPCS3's C++ atomic infrastructure |
| **R8.8** | Sync commands (BARRIER / EIEIO / SYNC — cmd codes 0xC0 / 0xC8 / 0xCC) | Likely incremental |
| **R8.9** | PUTRL family (atomic-REPLACE, cmd 0x34 / 0x35 / 0x36) | Semantic divergence vs PUT; may force a separate state machine path |
| — | **Stall fixtures bridge-ON triple-symmetry** | Bridge runtime stall path is landed (R8.5d D.1.b + D.2); promoting `getl_stall_v1` / `putl_stall_v1` to bridge-ON triple-symmetric oracles is a small follow-up if a real workload needs it. Currently the stall fixtures are replay-only validated. |
| — | **SPURS production support** | OUT OF SCOPE. SPURS captures contain commercial code. Defer to a separate CC0 multi-SPU fixture authored for the purpose. **v4 / SPURS traces remain diagnostic-only forever.** |

R8.6+ gate only on real workload demand or a CC0 fixture that
forces the path.

### 9.3 Subsequent SPU scope (R8.6+)

| Phase | Scope | Notes |
|-------|-------|-------|
| **R8.6** | Multi-SPU DMA races on shared EA | The bridge's persistent-handle table is keyed by `lv2_id`; multi-SPU is a separate workstream. **This was numbered R8.5 in the prior § 9; renumbered because R8.5 is now stall-and-notify.** |
| **R8.7** | MFC atomic primitives (GETLLAR / PUTLLC / PUTLLUC / PUTQLLUC) | LL/SC reservation tracking — bridge needs a per-`raddr` reservation map shared with RPCS3's C++ atomic infrastructure |
| **R8.8** | Sync commands (BARRIER / EIEIO / SYNC — cmd codes 0xC0 / 0xC8 / 0xCC) | Likely incremental |
| **R8.9** | PUTRL family (atomic-REPLACE, cmd 0x34 / 0x35 / 0x36) | Semantic divergence vs PUT; may force a separate state machine path |
| — | **SPURS production support** | OUT OF SCOPE. SPURS captures contain commercial code. Defer to a separate CC0 multi-SPU fixture authored for the purpose. **v4 / SPURS traces remain diagnostic-only forever.** |

R8.6+ gate only on real workload demand or a CC0 fixture that
forces the path.

### 9.4 Strategic pivot recommendation (post-R8.5e) — broader RPCS3 integration

The SPU foundation is **MVP-complete at R8.5e**: 20 oracles
covering mailbox / signal / loadstore / GET / PUT / multi-DMA /
TagStat modes / full 6-code list-DMA family / stall-and-notify
(both directions), with triple-symmetric acceptance (bridge OFF
/ bridge ON / replay) on 8 DMA fixtures + replay-validated
stall handshake on 2 stall fixtures. The R8.5 wave's empirical
result — D.6 plumbing carrying over to PUTL stall with zero
code — validates that the SPU stack's abstractions have stabilized.

**The strategic pivot is now broader RPCS3 integration** rather
than deeper SPU depth:

1. **LV2 / SPU group syscalls** — the kernel-side surface that
   creates / dispatches / synchronizes SPU threads. Currently
   scaffolded for headers only (no execution parity). Concrete
   first deliverable: end-to-end `sys_spu_thread_group_create` →
   `sysSpuThreadInitialize` → `sysSpuThreadGroupStart` →
   `sysSpuThreadGroupJoin` lifecycle in Rust, with the captured
   CC0 oracle binaries (`single_spu_*_v1.self`) as the integration
   test corpus. Today these run only via real RPCS3 `--headless`
   for capture; a Rust-side driver that replicates the lv2 path
   would let the existing 20 oracles double as end-to-end
   integration tests.
2. **PPU minimal interpreter** — enough to execute the PPU half
   of an SPU-using CC0 homebrew. The PPU side of every existing
   fixture is ~150 lines of PSL1GHT calls; covering that surface
   lets the existing fixtures drive their own end-to-end runs
   from Rust without RPCS3.
3. **Loader / FS / VFS** — required for the PPU path
   (`.self` / `.sprx` parsing, lv2 process startup, FS mounts).

R8.6, R8.7, R8.8, R8.9 and stall-fixture bridge-ON promotion
should defer until a real workload demands them. Without a
PPU+LV2 path that drives SPUs from the Rust stack, those
features can only be validated against synthetic CC0 fixtures —
diminishing-returns risk is real and the R8.5 wave just
demonstrated it (R8.5e validated PUTL stall with literally zero
new architectural surface — incrementally polishing a saturated
abstraction).

### 9.5 Hard rules carried forward to R8.5+

Same hard rules from § 8 apply throughout R8.5+:

- No fake JSONL, no fake DMA, no fake LS bytes, no fake
  `RdTagStat`, no fake `RdListStallStat`, no fake
  `WrListStallAck`.
- No commercial trace promotion.
- No manual JSONL editing after capture.
- The **behavior-freeze contract remains active** — the
  behavior-freeze infrastructure is complete and operational,
  but every new artifact still passes the same gates
  (`check_patch_separation.py`, `check_trace_fixtures.py`,
  `check_triple_symmetry.py` where applicable).
- v4 / SPURS stays diagnostic-only forever.
- Behavior-freeze fixtures stay CC0-only; commercial SPU captures
  never enter `behavior-freeze/`.

---

## 11. R9 — LV2/PPU integration (CLOSED architecturally complete 2026-05-25)

R9 wave (LV2/PPU integration to drive existing 20 CC0 SPU oracle
binaries end-to-end via Rust) is closed as **architecturally
complete**. 22 commits across R9.1.a → R9.1.n (R9.1m + R9.1n
folded as diagnostic-only changes to `tests/run_self_smoke.rs`).
The loader/process-bootstrap pipeline, PPC interpreter coverage,
lv2 syscall dispatcher, NID-specific import handlers, mini_printf
format resolver, and SPU lifecycle wiring all landed. End-user
TTY emit from PSL1GHT main()'s printf path deferred to a future
newlib-binding wave (see § 11.5 and `.planning/R9_FINAL_CLOSURE.md`).

Zero regression on the 264 cargo test result blocks across every
slice. The 20 SPU oracle replay tests remained green throughout
the entire R9 wave.

### 11.1 Commit timeline

| Commit | Slice | Scope |
|--------|-------|-------|
| `958a5f48e` | R9.1g.2 | PT_SCE_PPU_PROCESS_PARAM parser (sys_process_param, 32 bytes) |
| `b14af94f9` | R9.1g.3 | PT_SCE_PPU_PROC_PARAM parser (initially wrong field names) |
| `64a5f10a0` | R9.1g.4 | TLS init (PT_TLS walker + r13 seed) |
| `eba063916` | R9.1g.5 | libstub `PpuPrxModuleInfo` parser + correct `SysProcPrxParam` field names |
| `ef8c48659` | R9.1g.6 | install import-stub trampolines + populate `addrs[]` table |
| `f1ade4f1e` | R9.1g.7 | dispatcher arm for unimplemented imports (CIA-in-stub-region fast path) |
| `e05e5c24b` | R9.1g.8 | wire all init into `EmuCore::run_self` before _start |
| `47bf39fb3` | R9.1g.9 iter1 | syscall #330 (mmapper_allocate_address) + ldu + subfic/addic/rlwimi + stack/TLS layout |
| `340e6a438` | R9.1g.9 iter2 | TLS bias fix + with-update L/S + P31 ALU batch + mmapper family + heap pre-alloc |
| `d162a46c0` | R9.1g.9 iter3 | sys_spu_* stubs (full SPU lifecycle) + permissive catch-all |
| `9794837ff` | R9.1g.10 | wire real SPU execution into sys_spu_thread_group_start |
| `700303684` | R9.1g.11 | sys_process_exit (NID 0xe6f2c1e7) terminates run + NID lookups documented |
| `b4b764906` | R9.1h | NID-specific stubs for sys_spinlock + sys_mmapper imports |
| `b2558fe5f` | R9.1h | handoff doc — autonomous loop paused |
| `a854e431e` | R9.1h slice 2-4 | PSL1GHT sysPrxForUser import handlers + import dump |
| `c4bbb312b` | R9.1i | 8 stdio NID handlers + sys_fs_fstat/write + full NID map |
| `a73c17b4d` | R9.1j | post-fstat disassembly proves static-newlib blocker |
| `18f22d3bd` | R9.1k | __syscalls .data scan (incorrect — limited to p_filesz) |
| `(folded)` | R9.1m | __syscalls .data scan corrected to p_memsz — table IS populated at 0x100511A0 |
| `(folded)` | R9.1n | 31 FD codes mapped; slot[04] = __librt_write_r @ 0x11168 |
| `5b51b7b46` | R9 closure | folds R9.1m + R9.1n diagnostic changes + R9_FINAL_CLOSURE.md + this PROJECT_STATUS rewrite + R9_1L_ROOT_CAUSE SUPERSEDED stamp |

### 11.2 What landed

**SCE/SELF + ELF loading** (extending R9.1a/.1b):
- PT_SCE_PPU_PROCESS_PARAM (`0x60000001`) parser exposes the
  binary-declared stack size + priority + sdk version.
- PT_SCE_PPU_PROC_PARAM (`0x60000002`) parser exposes the
  `.libent` / `.libstub` ranges. Field names corrected per
  RPCS3 C++ reference (`PPUModule.cpp:2479`).
- `PpuPrxModuleInfo` (44 bytes) parser for each `.libstub`
  entry — describes one imported PRX module.
- PT_TLS walker + `EmuCore::init_tls` allocates the per-
  thread storage with the Linux ELFv1 +0x7000 TP bias.

**Memory layout** (PPU user-mode VM):
- `USER_STACK_TOP = 0xD0000000`, 1 MB
- `USER_TLS_VADDR = 0xCFE00000`, 9-page window
- `USER_IMPORT_STUB_VADDR = 0xD0010000`, 64 KB
- Heap pre-allocated at `[0xB0000000, 0xB2000000)`, 32 MB

**PLT thunk resolution** via import stubs:
- `EmuCore::init_imports` walks `.libstub`, allocates a stub
  region above the user stack, and for each imported NID
  writes: 4 byte `sc` trampoline + 8 byte function descriptor
  (`{u32 code, u32 toc}`), then patches the `.got` addrs[]
  slot to point at the FD.
- `EmuCore.import_plan` records each stub's `(module_name,
  nid, trampoline_vaddr, fd_vaddr, addrs_slot)` for the
  dispatcher's reverse lookup.

**lv2 syscall dispatcher** (`EmuCore::dispatch_syscall`):
- New stub-region check: when `sc` fires from inside the
  import-stub region, look up the NID and either terminate
  (sys_process_exit, NID `0xe6f2c1e7`) or return r3=0 +
  jump to LR.
- ~50 lv2 syscall arms covering: sys_mmapper_* family
  (324-339), full sys_spu_* family (155-194 incl. 169
  sys_spu_initialize, 170 group_create, 172 thread_initialize,
  173 group_start with REAL spu-interpreter execution, 178
  group_join returning captured OUT_MBOX), `sys_tty_write`
  (#403, from R9.1a), plus a permissive catch-all that
  logs+returns 0 for unknown numbers.

**PPU interpreter coverage** (25+ new opcodes total across
R9.1g.9 iterations):
- D-form with-update L/S: lwzu, lbzu, lhzu, lhau, stwu,
  stbu, sthu.
- DS-form: ldu (P58 XO=1).
- Primary integers: subfic, addic, addic., rlwimi.
- P31 XO ALU: sradi, addze, mtcrf, stdx, lbzx, lwzx, nor,
  ldx, mulld, mfcr, lfdx, andc, nand, eqv, orc, subfc,
  subfe, adde, subfze, addme, subfme.

**Real SPU execution** (R9.1g.10):
- `_sys_spu_image_import` (157) parses an SPU ELF blob from
  PPU memory, builds an `SpuImage` via `rpcs3-lv2-spu-image`,
  writes the 16-byte `sysSpuImage` struct back to the
  caller, and stashes the image on `EmuCore.spu_image`.
- `sys_spu_thread_initialize` (172) reads the 32-byte
  `sysSpuThreadArgument` struct (4× BE u64) and stashes the
  args on `EmuCore.spu_thread_args`.
- `sys_spu_thread_group_start` (173) allocates a fresh
  `SpuThread`, deploys the captured image into LS,
  marshals arg0..arg3 into SPU r3..r6 (preferred-slot
  convention), and runs `spu_run_n` (the existing
  rpcs3-spu-interpreter) until a stop instruction. Captures
  the OUT_MBOX value (which mailbox_v1's SPU code writes
  before stopping) and stashes it on `EmuCore.spu_exit_status`.
- `sys_spu_thread_group_join` (178) writes `cause=1`
  (JOIN_GROUP_EXIT) + the captured OUT_MBOX value to the
  caller's `*cause` / `*status` pointers.

### 11.3 What did NOT land

End-user TTY emit from PSL1GHT main()'s printf path. The
investigation in R9.1l → R9.1n proves the constructor chain
executes correctly (R9.1m FD-pointer scan over PHDR[3]'s full
p_memsz=0x414D8 found 31 sequential u64 FD pointers at vaddr
`0x100511A0` — the populated `__syscalls` table) and
`__librt_write_r` exists at code 0x11168 (R9.1n identified
slot[04] of `__syscalls` → FD@0x30F40 → routes `fd<=1` to
`sys_tty_write` (#403) and `fd>1` to `sys_fs_write` (#803)).
Neither sys_tty_write nor sys_fs_write ever fires during the
smoke run.

Therefore the PPU never reaches `__librt_write_r`. The
disconnection is in newlib's internal `_reent` struct
(specifically the `_write_r` function pointer). PSL1GHT's
`_reent` init path is NOT `__syscalls_init`; it's a separate
newlib mechanism that the public PSL1GHT repo does not expose
(the relevant `<sys/reent.h>` lives only in newlib's installed
headers).

See `.planning/R9_FINAL_CLOSURE.md` for the full closure
narrative.

### 11.4 What this enables

- **Architectural pipeline complete.** Any future R9.x slice
  that fixes the crt0-bail or jumps to main() inherits the
  full sys_spu_* lifecycle wiring, including REAL SPU
  execution.
- **R9.1g.10's SPU execution wire-up is exercised** the
  moment PSL1GHT actually reaches `sys_spu_thread_group_start`.
  No additional code change is needed there.
- **Future PSL1GHT-built `.self` binaries** can be analyzed via
  the audit + parser infrastructure (`r9_opcode_audit.py`,
  `SysProcessParam`, `SysProcPrxParam`, `PpuPrxModuleInfo`).
- **20 SPU oracle replay tests remain green** — the existing
  SPU stack (R5-R8.5e) is untouched.

### 11.5 Strategic recommendation post-R9 closure

R9 closed architecturally complete. Three next directions:

1. **Option A (recommended) — Pivot away from R9.** SPU stack
   is MVP-complete at R8.5e (20 oracles validated). R9's
   architectural integration of LV2/PPU with the SPU layer
   is complete. Further progress on PSL1GHT TTY emit is a
   specialized newlib-internal investigation that does not
   enable other project work. Better near-term value comes
   from RSX scaffolding, audio, filesystem, or additional
   lv2 sync primitives.
2. **Option B — Continue into R9.2 newlib-binding wave.**
   Bounded (~1-2 sessions) but specialized: reverse-engineer
   PSL1GHT's `_reent` init mechanism (locate `_reent` in
   `.bss`, disassemble path populating `_reent._write_r`,
   patch slot at load time). Delivers TTY emit for
   mailbox_v1 specifically.
3. **Option C — main() bypass.** Skip PSL1GHT crt0 by
   locating `main()` via prologue byte-pattern matching
   (`mflr r0; std r0, 16(r1); stdu r1, -N(r1)`) and jumping
   CIA directly. Less faithful but delivers TTY for ALL 20
   fixtures simultaneously if the prologue pattern is
   consistent.

User selected **Option A** on 2026-05-25; next wave selection
is open.

---

## 10. Historical archive

R5 / R4 long-form material — the full iteration-by-iteration timeline
from R4a through R5.9e.7 and the R5.11 / R5.11b additive expansions —
has been moved to:

- [`docs/history/PROJECT_STATUS_R5_ARCHIVE.md`](./history/PROJECT_STATUS_R5_ARCHIVE.md)

That archive carries the verbatim text as it stood on 2026-04-29 at
R5 closure plus the R5.11 / R5.11b expansions. The archive includes:

- The full R5 closure section (delivered components, what stayed out,
  confirmations at R5 closure).
- The full R5.4a..p ISA-coverage iteration log (R5.10a → R5.10p), which
  ended at the DMA / MFC boundary that R6.7 has since crossed.
- The full R5.8 A.1 / A.2 / A.3 capture pipeline narrative.
- The full R5.9a..R5.9e.7 multi-SPU schema + first replay-validated
  fixture story.
- All R5.11 + R5.11b additive fixture entries (`single_spu_branch_loop_v1`,
  `single_spu_signal_v1`, `single_spu_loadstore_v1`).
- The original "Next recommended phase" sections that recommended
  R5.8 / R6 — those are **obsolete / historical**. The current "next
  steps" are in § 9 above.

Path layout (as of 2026-05-22 consolidation):
- `behavior-freeze/docs/` — retains ONLY the two path-locked
  operational stubs: `AUTONOMOUS_LOG.md` (Claude Code Stop /
  SessionStart hook target) and `SPU_RECOMPILER_PLAN.md`
  (referenced from `rust/rpcs3-spu-recompiler/src/lib.rs` doc-comment).
  See `behavior-freeze/docs/README.md` for the lock-in rationale.
- [`docs/history/`](./history/) — single archive location for the
  legacy stubs moved out of `behavior-freeze/docs/` in 2026-05-22:
  `INVENTORY.md`, `DECISIONS.md`, `DEFERRED.md`, `BACKLOG_RESIDUAL.md`,
  `HOMEBREW_PLAN.md`, plus the original `PROJECT_STATUS_R5_ARCHIVE.md`.
- [`historico/pre-r4b-2026-04-25/`](../historico/pre-r4b-2026-04-25/) —
  pre-R4b verbatim snapshots (older still).
