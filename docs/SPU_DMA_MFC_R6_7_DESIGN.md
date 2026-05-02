# SPU DMA / MFC support — R6.7 design (no implementation)

This is a **design-only** document. It does NOT introduce any
DMA/MFC code path into the writer, the Rust replay engine, the
Rust SPU interpreter, or the C++ bridge. All references to "shall"
or "would" describe what a future implementation phase would
deliver; nothing here ships executable behaviour.

The document exists so the project can commit to a coherent
DMA/MFC architecture before any code lands. The 6 existing
replay-validated oracles + the bridge feature set must remain
intact through every implementation phase that follows.

---

## 1. Current state (2026-05-01)

### 1.1 Writer

`rpcs3/Emu/Cell/SPUTraceJsonl.{h,cpp}` records exactly these
event kinds (per `docs/SPU_TRACE_CAPTURE.md` § "Memory / DMA
capture"):

- `ppu_push_inmbox`, `ppu_signal`, `ppu_pop_outmbox`
- `spu_image`, `spu_rdch`, `spu_wrch`, `spu_park`, `spu_wake`,
  `spu_stop`, `final_state`

The `spu_wrch` events are emitted for ANY channel index; in
practice the captured fixtures only have ch28 (OUT_MBOX). MFC
channels (16–24) are not emitted because the writer has no hook
on the SPU's `wrch ch16…ch21` paths in `SPUThread.cpp`. **This is
a writer gap, not a deliberate filter** — the C++ executor
genuinely runs `wrch ch16` etc. but the trace hooks were only
added to the mailbox/signal/stop paths in R5.8 A.3.

### 1.2 Replay engine (`rust/rpcs3-spu-differential/src/trace_fmt.rs`)

The parser already has a defense-in-depth gate that fires on any
`spu_wrch ch21 (MFC_Cmd)` event: returns
`TraceParseError::UnsupportedDmaInTrace { target_spu,
event_index, channel }`. The check is at line ~653; tested at
~2280-2312. **Today the gate never fires** because the writer
never emits ch21 events; if a future writer extension surfaces
them, the gate catches them at parse time.

### 1.3 Rust SPU thread state (`rust/rpcs3-spu-thread/src/lib.rs`)

The data model is already partially in place:

```rust
pub struct SpuThread {
    pub ch_mfc_cmd: SpuMfcCmd,             // current cmd packet
    pub mfc_queue: [SpuMfcCmd; MFC_QUEUE_DEPTH], // 16-entry queue
    pub mfc_size: u32, pub mfc_barrier: u32, pub mfc_fence: u32,
    pub raddr: u32, pub rtime: u64,        // LL/SC reservation
    pub rdata: [u8; RESERVATION_BLOCK],
    ...
}

pub struct SpuMfcCmd {
    pub cmd: u8, pub tag: u8, pub size: u16,
    pub lsa: u32, pub eal: u32, pub eah: u32,
}

pub enum MfcCommand { Put, Get, GetLlar, ... }
pub enum MfcAtomicStatus { PutllcSuccess, PutllcFailure, ... }
pub enum MfcTagUpdate { Immediate, Any, All }
```

The interpreter's `ch::` module however does **NOT** include
MFC channel constants (16-24); it stops at `SPU_RDSIGNOTIFY2 = 4`,
`SPU_WRDEC = 7`, then jumps straight to `SPU_RDEVENTMASK = 22`,
`SPU_RDMACHSTAT = 23`, `SPU_WROUTMBOX = 28`, etc. So `wrch ch16`
through `ch21` would currently surface as an "unknown channel"
runtime error in the interpreter (per `R5.10p` v4 diagnostic note).

### 1.4 Bridge (`R:\rpcs3\Emu\Cell\SPURustBridge.cpp`, sha `7d6b6bba…`)

`classify_stall_channel(21)` returns
`{ "MFC_Cmd (ch21, DMA)", potentially_retryable=false }`. The
multi-round loop's stall handler only matches `StallRead` on
`{29, 3, 4}` and `StallWrite` on `{28}` — any MFC channel stall
falls into the "Any other outcome … drop session, fall back" arm.
This is the correct behaviour today: the C++ executor handles MFC
correctly via `do_mfc()` and `vm::` accessors, so the bridge
yielding to it is the safe default.

### 1.5 Existing oracle suite (6 fixtures)

All six are deliberately non-DMA. Their `.notes.md` companions
each assert "zero `spu_wrch ch21`" as an acceptance criterion. The
replay tests under `rust/rpcs3-spu-recompiler/tests/` enforce this
via `assert!(mfc_cmd_events.is_empty())`.

### 1.6 v4 diagnostic trace

`rust/rpcs3-spu-differential/tests/data/spurs_test_v4_real.jsonl`
(40061 events, 100% ch28) is the **DMA-bound diagnostic-only**
trace. The R5.10p note (line 285 of
`SPU_TRACE_R5_9E_REPLAY_PLAN.md`) catalogued the DMA structure
the SPU image actually contains:

- pc 0x720..0x7AC: textbook MFC GET sequence
  - `wrch ch16 (MFC_LSA)` at 0x74C
  - `wrch ch17 (MFC_EAH)` at 0x758
  - `wrch ch18 (MFC_EAL)` at 0x764
  - `wrch ch19 (MFC_Size)` at 0x774
  - `wrch ch20 (MFC_TagID)` at 0x780
  - **`wrch ch21 (MFC_Cmd) = 0x40 (GET)` at 0x79C**
  - `wrch ch22 (WrTagMask)` + `wrch ch23 (WrTagUpdate)`
  - `rdch ch24 (RdTagStat)` blocking-wait at 0x7A8
  - `lqa r4, [0x3FFE0]` consumes the DMA'd data at 0x7AC

- v4 image contains 28 MFC WRCH + 4 MFC RDCH + 4 distinct
  MFC_Cmd dispatches. ZERO non-MFC channel ops besides ch28.

v4 has fully exited replay-valid scope and stays under
`tests/data/` as `#[ignore]`d diagnostic. **R6.7 must not
promote v4** to the replay-validated bucket without first
landing the trace + side-file + replay support designed below
AND a separate fresh, CC0-clean DMA homebrew fixture.

---

## 2. Non-goals (R6.7 + first implementation slice)

- ❌ MFC PUT (LS → EA writes). Only GET (EA → LS) is in the
  initial scope.
- ❌ DMA list commands (GETL, PUTL, GETLB, PUTLB, etc.). Only
  the simple non-list GET is in scope.
- ❌ MFC barriers (BARRIER, FENCE bits in cmd). Only plain GET
  with no synchronisation flags.
- ❌ Atomic primitives (GETLLAR, PUTLLC, PUTLLUC, PUTQLLUC) —
  these need LL/SC reservation tracking, an entire separate
  work item.
- ❌ Multi-SPU DMA races on shared EA regions. R6.7 single-SPU
  only.
- ❌ Self-modifying code via DMA-to-LS overlay (the same
  `target_spu` re-loading its own LS via DMA). Defer to a
  future schema-version bump.
- ❌ RSX / IO memory side effects (DMA into framebuffer / IO
  registers). Out of scope; trace would refuse to capture.
- ❌ SPURS runtime support. SPURS-using games will trigger DMA
  but their captures contain commercial code; CC0 homebrew
  for our acceptance is mandatory.
- ❌ Promoting v4 to replay-validated. v4 stays diagnostic-only;
  the future DMA fixture is a separate, fresh CC0 capture.
- ❌ Fake DMA. Any commit that returns synthetic success for
  `MFC_Cmd` without the captured oracle bytes is a hard reject.

---

## 3. Minimal supported MFC subset

Channels (per `rpcs3/Emu/Cell/SPUThread.cpp:6244+`):

| Ch  | Name             | Direction | Required? | Notes |
|-----|------------------|-----------|-----------|-------|
| 16  | `MFC_LSA`        | wrch      | yes       | LS address. Stored in `ch_mfc_cmd.lsa`. |
| 17  | `MFC_EAH`        | wrch      | yes       | Effective address high. (32-bit PS3 PPU = always 0.) |
| 18  | `MFC_EAL`        | wrch      | yes       | Effective address low. Stored in `eal`. |
| 19  | `MFC_Size`       | wrch      | yes       | Transfer size, bytes. Stored in `size`. Must be 1, 2, 4, 8 OR a multiple of 16 up to 16384. 16-byte alignment when ≥16. |
| 20  | `MFC_TagID`      | wrch      | yes       | Tag (0-31). Stored in `tag`. |
| 21  | `MFC_Cmd`        | wrch      | yes       | Cmd code. Initial scope: ONLY `0x40 (GET)`. Any other code = `UnsupportedMfcCmd`. |
| 22  | `WrTagMask`      | wrch      | yes       | Mask of tags to wait on (1 << tag). |
| 23  | `WrTagUpdate`    | wrch      | yes       | Update mode (`MfcTagUpdate::Immediate`/`Any`/`All`). |
| 24  | `RdTagStat`      | rdch      | yes       | Returns the bitmask of completed tags. Blocking when no tag in mask has completed. |
| 25  | `RdTagMask`      | rdch      | optional  | Returns the current `WrTagMask` value. Stateless. |
| 26  | `RdListStallStat`| rdch      | NO        | List DMA only — out of scope. |
| 27  | `WrListStallAck` | wrch      | NO        | List DMA only — out of scope. |

**Cmd code subset (R6.7 + first slice):**

| Code | Mnemonic | Direction | Status |
|------|----------|-----------|--------|
| 0x40 | `GET`    | EA → LS   | **In scope.** |
| 0x20 | `PUT`    | LS → EA   | Out of R6.7, planned R7. |
| All other GET/PUT variants (`GETB`, `GETL`, atomic, etc.) | — | — | Out of scope; replay rejects with `UnsupportedMfcCmd`. |

---

## 4. JSONL schema proposal

The schema additions are **strict additive** — no existing event
kind changes, no field renames. The 6 existing oracle traces
remain valid under the new parser.

### 4.1 New event kinds

#### `spu_wrch` to channels 16-21 (already-emittable; writer gap fix)

The existing `spu_wrch` event already has the right shape:

```json
{"seq":N,"side":"spu","kind":"spu_wrch","target_spu":...,"pc":...,
 "channel":16,"value":0x12340,"would_stall":false}
```

The **writer extension** is to add hooks at the `case
SPU_WrLSA`, `case SPU_WrEAH`, ..., `case SPU_WrTagUpdate` arms in
`SPUThread.cpp` (line 6244+). The hooks emit the same shape as
the existing `case SPU_WrOutMbox` hook (`record_spu_wrch`).

#### `spu_rdch` to channel 24 (`RdTagStat`)

Already-emittable — same shape as existing `rdch ch3` hooks. The
hook lives in `SPUThread.cpp:5441` (`get_ch_value` / `read_channel`).
For ch24 `RdTagStat` the hook records the returned status bitmask
in `value`; on stall (no tag completed yet) it emits `would_stall=true`
followed by `spu_park reason=channel_read channel=24`.

#### `spu_mfc_cmd` (NEW) — issued at the moment of `wrch ch21`

```json
{"seq":N,"side":"spu","kind":"spu_mfc_cmd","target_spu":...,
 "pc":...,
 "cmd":64,"tag":3,"size":128,"lsa":0x3FFE0,
 "eah":0,"eal":0xD0010000,
 "ea_chunk_sha256":"<sha256 of the captured EA bytes>"}
```

**Fields:**
- `cmd` — `MFC_Cmd` value as written (0x40 = GET in scope).
- `tag` — same as `ch_mfc_cmd.tag` at the moment the cmd fires.
- `size` — same as `ch_mfc_cmd.size`.
- `lsa` — same as `ch_mfc_cmd.lsa`. Where the SPU expects the
  bytes to land.
- `eah`/`eal` — split as the SPU writes them (EAH always 0 on
  PS3 PPU). Combined: `ea = (eah << 32) | eal`.
- `ea_chunk_sha256` — content-addressed reference to the side-
  file under `behavior-freeze/fixtures/spu/dma/<sha256>.dmachunk`.

**Ordering invariant:** `spu_mfc_cmd` MUST appear immediately
after the `spu_wrch ch21` event for the same `target_spu`, with
`seq` strictly greater. The writer emits both atomically inside
the `SPU_WrChannel` handler. Replay parser asserts this ordering
and rejects misordered traces.

#### `mfc_dma_complete` (NEW) — fires when the DMA actually completes on the C++ side

```json
{"seq":N,"side":"spu","kind":"mfc_dma_complete","target_spu":...,
 "tag":3,"transferred_bytes":128}
```

**Fields:**
- `tag` — which tag completed.
- `transferred_bytes` — how many bytes were actually moved
  (should equal the cmd's `size` for plain GET; differs for
  partial transfers in future scope).

**Ordering invariant:** `mfc_dma_complete` MUST appear strictly
between the `spu_mfc_cmd` event with the same tag AND the next
`spu_rdch ch24` that observes the tag completed. Replay uses
this event to advance its oracle DMA state.

Open question — whether `mfc_dma_complete` is needed in scope A
(synchronous GET) or only in scope B (async / multi-tag).
For a single-shot synchronous GET the parser can synthesise the
completion as occurring "between" the `spu_mfc_cmd` and the
`rdch ch24`. Including the explicit event makes the trace more
robust against future async cases without a second schema bump.

### 4.2 Source of truth + ordering

| Event                  | Source of truth                                              | Emits at                                                  |
|------------------------|--------------------------------------------------------------|-----------------------------------------------------------|
| `spu_wrch ch16-23`     | C++ `SPUThread::set_ch_value` (line 6244+)                   | After `ch_mfc_cmd.<field>` is updated                     |
| `spu_mfc_cmd`          | C++ `do_mfc()` entry (after cmd dispatch decided)            | Strictly after `spu_wrch ch21`, same lock                 |
| `mfc_dma_complete`     | C++ `do_mfc()` post-transfer (after `vm::write/read`)        | Before any `RdTagStat` returns the new bit                |
| `spu_rdch ch24`        | C++ `get_ch_value` for `RdTagStat`                            | Same convention as existing rdch hooks                    |

### 4.3 Failure behaviour

| Condition                                              | Parser action                                                                   |
|--------------------------------------------------------|---------------------------------------------------------------------------------|
| `spu_mfc_cmd` not preceded by ch16-21 wrches           | `MalformedMfcSequence { event_index, missing_channel }` — hard reject           |
| `cmd` field not in `{0x40}`                            | `UnsupportedMfcCmd { cmd, event_index }` — hard reject                          |
| `size` not in `{1,2,4,8}` or not 16-byte aligned ≥16   | `UnsupportedMfcSize { size, event_index }` — hard reject                        |
| `size > 16384`                                         | `UnsupportedMfcSize` — hard reject                                              |
| `lsa + size > SPU_LS_SIZE`                             | `MfcLsaOutOfBounds { lsa, size, event_index }` — hard reject                    |
| `ea_chunk_sha256` references missing side-file         | `MissingDmaChunk { sha, event_index }` — hard reject                            |
| `mfc_dma_complete` for unknown tag                     | `UnknownMfcTag { tag, event_index }` — hard reject                              |
| `rdch ch24` returns a bit for a tag never dispatched   | `OrphanTagStat { tag_mask, event_index }` — hard reject                         |

The existing `UnsupportedDmaInTrace` rejection is **renamed**
(or supplemented) in the implementation phase: it currently
fires for ANY ch21, but the new model accepts ch21 with a valid
`spu_mfc_cmd` follow-up. `UnsupportedDmaInTrace` becomes
`UnsupportedMfcCmd` for the cmd-code-not-supported case.

---

## 5. EA-memory side-file design

### 5.1 Layout

```
behavior-freeze/fixtures/spu/dma/
├── README.md                                 # explains the layout
└── <sha256>.dmachunk                         # raw bytes
```

Mirrors the existing `behavior-freeze/fixtures/spu/images/<sha>.spuimg`
convention. Content-addressed by SHA-256 of the EA bytes captured
at the moment of the GET dispatch.

### 5.2 What the side-file contains

The `<sha>.dmachunk` is the **raw bytes** at the EA at the moment
the `wrch ch21 (MFC_Cmd=GET)` was dispatched, exactly `size`
bytes long. Big-endian SPU byte order is preserved as-is — the
bytes are what the SPU's subsequent `lqa` will read.

### 5.3 Why content-addressed

- **Deduplication.** Multiple traces sharing the same EA snapshot
  (e.g. a homebrew that DMA-loads a constant data table)
  reference the same `.dmachunk`.
- **Tamper detection.** Any silent edit changes the SHA, breaks
  the reference, and the parser surfaces `MissingDmaChunk`.
- **Reproducibility.** Two independent captures of the same
  workload + same EA state → bit-identical `.dmachunk` files.

### 5.4 Endianness

PS3 = big-endian for both PPU and SPU. The bytes stored in
`.dmachunk` are the same byte order as the SPU's LS would see
after the GET completes. No swap is needed in the writer or the
replay engine — bytes are copied verbatim from EA → `.dmachunk`
→ LS.

### 5.5 Size + alignment

- Min: 1 byte (legal MFC sizes include 1, 2, 4, 8).
- Max: 16384 bytes (= `0x4000`). Larger transfers need DMA
  list commands which are out of scope.
- Alignment: 16-byte aligned for sizes ≥16 (per Cell BE spec).
- The `.dmachunk` file size MUST equal the `size` field in the
  `spu_mfc_cmd` event that references it. Mismatch → parser
  rejects with `DmaChunkSizeMismatch`.

### 5.6 Avoiding copyrighted / commercial content

**License-clean discipline mirrors the `.self` fixture rule:**

- Only DMA chunks captured from CC0 homebrew authored for this
  project commit to `behavior-freeze/fixtures/spu/dma/`.
- Diagnostic-only DMA chunks captured from real games (e.g. a
  hypothetical future re-capture of v4 with full DMA writer
  hooks) live under `rust/rpcs3-spu-differential/tests/data/<trace>.dma/<sha>.dmachunk`,
  matching the per-trace layout used for `<trace>.images/`.

**Practical risk:** an EA chunk could contain code, strings, or
compiled assets that are copyrighted. CC0 fixtures sidestep this
by definition; commercial-game traces stay diagnostic-only and
NEVER move to `behavior-freeze/fixtures/`.

### 5.7 Initial fixture: `single_spu_dma_get_v1` (planned)

A CC0 homebrew that:

1. PPU pushes seed via IN_MBOX.
2. PPU writes a small data table (e.g. 16 u32 values) to a
   known EA.
3. SPU reads seed, sets up MFC GET to copy the table from EA
   to LS, waits on TagStat, then computes a checksum from the
   loaded LS bytes.
4. SPU writes checksum to OUT_MBOX, halts with stop 0x101.

Canonical inputs + outputs documented in the fixture's
`README.md` like the existing 6 oracles.

---

## 6. Replay state machine

### 6.1 Per-SPU MFC state

```rust
pub struct MfcReplayState {
    /// Cmd packet being assembled by ch16-20 wrches.
    pub pending_cmd: SpuMfcCmd,
    /// Tags currently in flight (issued via wrch ch21, not
    /// yet observed completed via rdch ch24).
    pub in_flight: BTreeMap<u8, MfcInFlight>,
    /// Tags whose `mfc_dma_complete` has fired but `rdch ch24`
    /// has not yet observed.
    pub completed_pending_observation: u32, // bitmask
    /// Current WrTagMask + WrTagUpdate waiting state.
    pub current_wait: Option<MfcTagWait>,
}

pub struct MfcInFlight {
    pub cmd: u8,           // 0x40 for GET
    pub size: u32,
    pub lsa: u32,
    pub ea: u64,
    pub chunk_sha256: [u8; 32],
}

pub struct MfcTagWait {
    pub mask: u32,         // from wrch ch22
    pub mode: MfcTagUpdate, // from wrch ch23
}
```

### 6.2 Event dispatch rules

```
on spu_wrch ch16: state.pending_cmd.lsa = value
on spu_wrch ch17: state.pending_cmd.eah = value
on spu_wrch ch18: state.pending_cmd.eal = value
on spu_wrch ch19: state.pending_cmd.size = value as u16
on spu_wrch ch20: state.pending_cmd.tag  = value as u8
on spu_wrch ch21:
    require next event is spu_mfc_cmd matching pending_cmd
    on cmd == 0x40 (GET):
        load <sha>.dmachunk, copy to ls[lsa..lsa+size]
        record in_flight[tag] = MfcInFlight { ... }
    on cmd != 0x40: hard reject

on spu_wrch ch22: state.current_wait = Some(MfcTagWait { mask, mode: <pending until ch23> })
on spu_wrch ch23: state.current_wait.as_mut().mode = MfcTagUpdate::from(value)

on mfc_dma_complete { tag, transferred_bytes }:
    require in_flight[tag] exists
    state.completed_pending_observation |= 1 << tag

on spu_rdch ch24:
    require state.current_wait.is_some()
    let wait = state.current_wait.take().unwrap()
    let observed = state.completed_pending_observation & wait.mask
    require observed matches expected (per Immediate/Any/All semantics)
    return observed; clear corresponding bits in completed_pending_observation;
    drop entries from in_flight whose tag is now observed
```

### 6.3 Interpreter vs Recompiler sharing oracle state

The `MfcReplayState` lives **next to** `SpuThread` in the replay
driver, not inside `SpuThread`. The replay engine
(`replay_per_spu_traces` / `replay_per_spu_traces_with`) wraps
both the Rust interpreter AND the Rust recompiler with the same
`MfcReplayState` instance. When the SPU executes `wrch ch21`,
the replay engine intercepts the cmd dispatch BEFORE the
backend's actual channel handler (which would error on unknown
channel today) — copies the bytes from `.dmachunk` to `ls`,
then continues.

This way both backends see the **same LS post-DMA**, and
`diff_snapshots(interp, jit).is_identical()` remains the
load-bearing acceptance gate.

### 6.4 LS comparison post-DMA

`diff_snapshots` already compares the full 256 KiB LS
byte-for-byte. After a GET, both backends' LS must match the
`.dmachunk` bytes at `[lsa..lsa+size]`. If interpreter and
recompiler disagree, the diff fires — exactly the regression
sentinel we want.

### 6.5 RdTagStat blocking semantics

In the C++ executor, `rdch ch24` blocks when no tag in the wait
mask has completed yet. In the replay engine, the trace's
`spu_rdch ch24` event (with `would_stall=true`) carries an
implicit ordering: by the time the replay reaches that event,
all `mfc_dma_complete` events for tags in the mask must have
already fired (otherwise the trace is malformed and the parser
rejects with `OrphanTagStat`). So replay never actually blocks —
it asserts state.

Bridge runtime is different (see § 7) — there real blocking
matters.

---

## 7. Bridge policy

### 7.1 Phase B (initial implementation): honest fallback

The bridge SHOULD continue to fall back honestly when the SPU
issues any MFC channel write. Specifically:

- Add ch16-23 to `classify_stall_channel` with
  `potentially_retryable=false` and informative names.
- The Rust SPU executor returns `Error` (or a new `UnsupportedChannel`
  outcome) when the SPU's `wrch ch16-21` executes.
  - Alternative: Rust executor adds the MFC channels but errors
    out at `wrch ch21` because RPCS3-side DMA machinery isn't
    callable from Rust.
- Bridge's existing fallback log line surfaces the channel
  classification, so the operator can see "SPU wants DMA → C++
  takes over for this thread".

**Why this is safe:** the C++ executor handles MFC correctly via
`do_mfc()` + `vm::` accessors. RPCS3's existing MFC infrastructure
doesn't need bridge support; it just needs the bridge to YIELD on
the first MFC operation.

### 7.2 Phase D (later): runtime DMA via FFI

Two architectures are plausible for the eventual runtime DMA:

**Option D1 — Rust calls back into C++ vm:: at wrch ch21:**
```cpp
// In bridge's StallWrite ch21 handler:
SpuMfcCmd cmd = rust_spu_get_mfc_cmd(h);
if (cmd.cmd == 0x40) {
    // Use RPCS3's vm:: accessors to read EA bytes.
    std::vector<u8> bytes(cmd.size);
    vm::read(cmd.ea, bytes.data(), cmd.size);
    rust_spu_load_ls_at(h, cmd.lsa, bytes.data(), cmd.size);
    // Mark tag completed.
    rust_spu_complete_tag(h, cmd.tag);
    continue;  // resume Rust executor
}
```

**Option D2 — Rust executor calls FFI back to C++ on MFC:**
```rust
// In Rust SPU's wrch ch21 handler:
unsafe { rpcs3_dma_get(self.eah, self.eal, self.size, lsa_ptr) };
```

D1 is cleaner architecturally: keeps Rust executor pure (no FFI
out), bridge mediates. D2 is simpler but couples the Rust SPU
executor tightly to RPCS3 host APIs.

**Phase D is out of R6.7 design scope.** R6.7 commits only to
Phase B (honest fallback) for the bridge.

### 7.3 Non-corruption guarantee

The Phase B fallback path MUST keep the load-bearing R6.4b
invariant: when the bridge falls back on the first MFC
operation, RPCS3 state must be byte-identical to entry. The
existing `try_delegate_execution()` already guarantees this
because:
- Phase 0/1/1b only mutate Rust state, never RPCS3 state.
- The first `rust_spu_run_until_event` returns BEFORE any DMA
  channel state is committed.
- On non-Stop outcome, drop session, `return false` — RPCS3
  state intact, C++ executor takes over from `spu_thread::pc`.

### 7.4 What the bridge must NOT do

- Pre-emptively peek `ch_mfc_cmd` and try to "pre-DMA" before
  running. The MFC params + cmd are SPU-side state; peeking
  RPCS3's mirrored state is not safe (it might not be in sync).
- Synthesise tag completion. The C++ executor's `do_mfc()` is
  the source of truth; faking completion would corrupt LS.
- Fall back AFTER any partial commit. If anything mutated RPCS3
  state (drained mailbox, set_value on out_mbox), we're committed
  — falling back at MFC means C++ executor continues from a
  half-mutated state, which violates the ownership contract.

---

## 8. Acceptance plan

### Phase A — trace + parser + replay state machine

| Step | Deliverable |
|------|-------------|
| A.1 | Writer extension: hooks at ch16-23 wrch + ch24 rdch + new `spu_mfc_cmd` + `mfc_dma_complete` events. Pinned via new sha256 in `check_patch_separation.py` with strict separation against the runtime hooks patch. **DONE 2026-05-02** — scaffolding sha `cda976d7…`, runtime_hooks sha `95bdcaae…`, bridge unchanged at `7d6b6bba…`. |
| A.2 | Parser extension in `trace_fmt.rs`: new `CapturedEvent::SpuMfcCmd` and `CapturedEvent::MfcDmaComplete` variants. New rejection codes per § 4.3. The `UnsupportedDmaInTrace` becomes `UnsupportedMfcCmd` (for non-GET cmds) — existing tests update accordingly. **DONE 2026-05-02** — new variants + 8 new `TraceParseError` codes (`UnsupportedMfcCmd`, `UnsupportedMfcEah`, `BadDmaSize`, `BadDmaLsa`, `BadDmaSha`, `BadMfcTag`, `BadDmaTransferredBytes`, `MalformedMfcSequence`); ordering invariant (`spu_wrch ch21` immediately followed by matching `spu_mfc_cmd`) enforced in post-pass walk; bare `spu_wrch ch21` without follow-up still rejects with legacy `UnsupportedDmaInTrace`. Transformer adds `TraceTransformError::UnsupportedDmaInTrace` so MFC traces are NOT silently ignored — explicit hard-reject preserved until A.4 lands the replay state machine. 12 new unit tests (positive + negative + transformer rejection); 6 oracle replay tests stay green; `cargo test --workspace --lib` 5609 pass. `.dmachunk` loader + replay state machine deferred to A.3 / A.4. |
| A.3 | Side-file loader: utility to resolve `<sha>.dmachunk` from `behavior-freeze/fixtures/spu/dma/` AND `<trace>.dma/` per-trace path (same fallback as `.spuimg`). **DONE 2026-05-02** — new `rust/rpcs3-spu-differential/src/dma_chunk.rs` module exposes `resolve_dma_chunk_side_file(trace_path, canonical_dma_dir, sha256, expected_size) -> Result<Vec<u8>, DmaChunkLoadError>` with per-trace precedence over canonical, defensive 64-lowercase-hex sha re-validation, empty / too-large / size-mismatch / sha-mismatch / missing / I/O failure variants, and the public `per_trace_dma_chunk_path` + `canonical_dma_chunk_path` builders for pre-flight checks. 11 new unit tests using `tempfile`; including `loader_does_not_change_transformer_policy` which proves `captured_events_to_traces_per_spu` STILL returns `TraceTransformError::UnsupportedDmaInTrace` for any MFC trace — A.3 does NOT relax the A.2 transformer policy. `.dmachunk` bytes are NEVER copied into LS at this phase (deferred to A.4). |
| A.4 | Replay state machine in `MfcReplayState` next to the per-SPU executor. Both Interpreter and Recompiler wrappers consult the same instance. **DONE 2026-05-02 — ACEITO PARCIAL.** New `rust/rpcs3-spu-differential/src/mfc_replay.rs` lands the standalone state machine (PendingMfcCmd + MfcReplayState + MfcTagUpdate enum + 13 `MfcReplayError` variants) covering: ch16-23 wrch dispatch, ch21 GET-only arming, `process_mfc_cmd` (validates against pending packet + invokes A.3 loader + copies bytes into caller-supplied LS buffer), `process_mfc_dma_complete` (validates `transferred_bytes == size` + tag in flight), `process_rdch_tagstat` (Immediate / Any / All wait-mode oracle). 13 new unit tests using `tempfile` cover happy path + every error-class transition. **Wiring into the actual `replay_trace` flow is DEFERRED to Phase C (C.1-C.4)** — the Rust SPU thread (`rpcs3-spu-thread::ch::`) does NOT yet handle MFC channels (16-25), so the SPU executor would error on `wrch ch16` before reaching `MfcReplayState`. The transformer continues to hard-reject MFC traces with `TraceTransformError::UnsupportedDmaInTrace` (test `transformer_still_rejects_valid_get_mfc_trace_until_executor_supports_mfc_channels` pins the policy). 6 existing oracle replay fixtures unaffected (none contain MFC events). |
| A.5 | First CC0 fixture `single_spu_dma_get_v1`: PPU writes table to EA, SPU GETs, computes checksum, halts. .self built via Docker. JSONL captured. .dmachunk + .spuimg produced. |
| A.6 | Replay test `single_spu_dma_get_v1_replay.rs` mirroring the 6 existing tests. Acceptance: byte-identical interpreter+recompiler, OUT_MBOX matches canonical computation. |

### Phase B — bridge honest fallback

| Step | Deliverable |
|------|-------------|
| B.1 | Bridge update: `classify_stall_channel` recognises 16-25 as MFC family with informative names + `potentially_retryable=false`. Activation log mentions DMA fallback explicitly. |
| B.2 | Verify regression: 6 existing fixtures (no DMA) + 1 new DMA fixture all run with bridge OFF (status canonical) AND ON (DMA fixture's bridge ON falls back to C++ on first MFC op without corruption; the 6 non-DMA fixtures unchanged). |

### Phase C — Rust SPU MFC channel handling (replay-only)

| Step | Deliverable |
|------|-------------|
| C.1 | Add ch16-25 to `rust/rpcs3-spu-thread/src/lib.rs` `ch::` module. |
| C.2 | Wire wrch dispatcher to update `ch_mfc_cmd` fields. |
| C.3 | wrch ch21 dispatcher: in REPLAY mode, replay state machine handles it (copies from `.dmachunk`). In NORMAL/runtime mode, returns `UnsupportedChannel` (bridge falls back). |
| C.4 | rdch ch24 dispatcher: in REPLAY mode, returns the oracle tag stat. In NORMAL/runtime mode, returns `UnsupportedChannel`. |

### Phase D — bridge runtime DMA opt-in (much later)

Out of R6.7 design scope. Marked here as the natural sequel.

---

## 9. Risks + open questions

### 9.1 Where the writer hooks fire

The C++ `wrch ch16-23` handlers in `SPUThread.cpp:6244+` are
mostly direct stores to `ch_mfc_cmd.<field>`. Adding hooks is
mechanical (mirror the existing `case SPU_WrOutMbox` hook), but
**ch21 specifically calls `do_mfc()` synchronously**. The hook
ordering must be:

1. `record_spu_wrch(ch=21, value=cmd_code)` — emits the wrch event
2. Atomically: snapshot EA bytes via `vm::read(ea, buf, size)`
   BEFORE `do_mfc()` mutates LS
3. `record_spu_mfc_cmd(...)` with the snapshot's SHA
4. `do_mfc()` (existing C++ logic)
5. `record_mfc_dma_complete(tag, transferred_bytes)`

The risk: between step 2 and step 4, the EA could be mutated by
another thread. For PSL1GHT cooperative single-SPU workloads this
isn't a real concern (PPU is blocked in Join while SPU runs, no
concurrent mutator). For multi-SPU or PPU-concurrent scenarios,
the snapshot might race. **R6.7 scope is single-SPU only**; the
race is documented as a future concern.

### 9.2 EAH always 0 on PS3?

PS3's PPU is 32-bit user-space. `MFC_EAH` is always 0 in PSL1GHT
homebrew. Real games might use it for higher-half addressing
(64-bit lv2 kernel space), which is out of scope. Replay parser
asserts `eah == 0` for in-scope traces.

### 9.3 Tag wait modes (Immediate/Any/All)

`MfcTagUpdate::Immediate` returns immediately with the current
status (no wait). `Any` waits for at least one tag in the mask
to complete. `All` waits for ALL tags in the mask to complete.

For a single-tag GET fixture, the mode doesn't matter — there's
only one tag in flight. The replay state machine needs to
implement all three for general correctness; the fixture only
exercises `All` (simpler).

### 9.4 Out-of-order tag completion

A real game might issue tag 1, then tag 2, and observe tag 2
completes before tag 1. The trace event ordering already captures
this (`mfc_dma_complete` fires in the order RPCS3 actually
completes them). Replay state machine's `BTreeMap<u8, MfcInFlight>`
handles arbitrary order.

### 9.5 Schema-version field?

The current JSONL format has no version field. Adding new event
kinds is **additive** — old parsers will treat unknown `kind`
values as parse errors today. Two options:

- **A:** Bump the writer to emit a `header { version: 2 }` event
  at the start of every trace. Old parsers reject; new parsers
  branch on version.
- **B:** Accept that R6.7's parser strictly supersedes the
  current one — old traces (no MFC events) parse fine in the
  new parser; new traces (with MFC events) need the new parser.

**Recommendation: Option B.** The 6 existing oracle traces
contain ZERO MFC events; they parse clean under the new schema
without changes. Adding a header event would force regenerating
all 6 captures.

### 9.6 `.dmachunk` storage cost

A homebrew DMA fixture might GET a few hundred bytes. Real games
DMA megabytes. Single fixtures: small. SPURS-game diagnostic
captures (if ever): could be hundreds of MB of `.dmachunk`s. The
content-addressed dedup helps, but commercial-game DMA captures
WILL be large. Mitigation: `behavior-freeze/fixtures/spu/dma/`
stays CC0 + small; large captures stay diagnostic-only under
`tests/data/`.

### 9.7 PUT (LS → EA) capture model

PUT writes from LS (which we already capture in `.spuimg`) to
EA. The "interesting" oracle state is the EA contents BEFORE the
PUT (so future GET-after-PUT replay can reconstruct the read).

For pure GET-only fixtures, PUTs aren't captured. For mixed
GET+PUT workloads, the writer needs to also snapshot EA bytes
BEFORE each PUT. Schema would add a symmetric `ea_chunk_sha256`
field to the same `spu_mfc_cmd` event for cmd=0x20.

**Out of R6.7 scope.**

### 9.8 What about `lqa`/`stqa` to/from EA?

`lqa` and `stqa` are LS-only opcodes (operate on LS, not EA).
They're already captured indirectly via `final_state` GPRs +
LS deltas. Not a DMA concern.

---

## 10. Migration plan from diagnostic v4 to future replay-valid fixture

v4 stays diagnostic-only **forever**. R6.7's DMA implementation
does NOT promote v4 to `behavior-freeze/`. Instead:

1. v4's diagnostic test (`real_trace_diagnostic.rs`) is updated
   to assert the new `UnsupportedMfcCmd` (for non-GET cmds) OR
   `MissingDmaChunk` (for traces lacking side-files) — depending
   on what the writer-extension capture of v4 produces.
2. A **fresh CC0 homebrew capture** (`single_spu_dma_get_v1` per
   § 8 phase A.5) becomes the first replay-validated DMA fixture.
3. Future SPURS-using captures (if attempted) stay under
   `tests/data/` as diagnostic. SPURS contains commercial code
   and is excluded from `behavior-freeze/`.

The fresh capture is the canonical DMA oracle. v4 informed the
ISA-coverage push (R5.10a..p) but is otherwise retired.

---

## 11. Explicit refusal of fake DMA

The R6.7 design and any implementation that follows MUST refuse
to introduce ANY of:

- Synthesising `MFC_Cmd=0x40` success without consulting an
  oracle (replay) or RPCS3 vm:: (runtime).
- Returning a fixed/zero/random tag stat for `rdch ch24` instead
  of the real wait result.
- Faking LS bytes after a GET (e.g. zeros, or pseudo-random).
- Promoting v4 to `behavior-freeze/` without a real
  writer-recapture + side-file pipeline.
- Editing existing JSONL traces to "remove" MFC events.

Any commit attempting these is a hard reject. The whole point
of the oracle suite is byte-identical agreement against captured
truth; faking destroys the contract.

---

## 12. Acceptance gates (R6.7 design phase)

This document being committed + reviewed is the R6.7 acceptance.
No code changes ship in R6.7. The implementation phases (A, B,
C, D) each have their own acceptance gates listed in § 8.

**R6.7-as-design** must satisfy:

- Document committed at `docs/SPU_DMA_MFC_R6_7_DESIGN.md` (this
  file).
- 6 existing oracles intact: the 6 replay tests still pass.
- Bridge unchanged: sha256 `7d6b6bba…` pinned.
- Trace patches unchanged: `d65aec91`/`8f253d7d` pinned.
- v4 still diagnostic-only.
- No `.dmachunk` files exist yet anywhere.
- No new schema variants in code.
