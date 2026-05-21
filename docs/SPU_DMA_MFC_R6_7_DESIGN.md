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

### 1.5 Oracle suite (7 fixtures since R6.7 A.5)

Six fixtures (`single_spu_mailbox_v1`, `single_spu_branch_loop_v1`,
`single_spu_signal_v1`, `single_spu_loadstore_v1`,
`single_spu_mailbox_multi_v1`, `game_like_mailbox_signal_v1`) are
deliberately non-DMA. Their `.notes.md` companions each assert
"zero `spu_wrch ch21`" as an acceptance criterion. The replay tests
under `rust/rpcs3-spu-recompiler/tests/` enforce this via
`assert!(mfc_cmd_events.is_empty())`.

The seventh oracle, **`single_spu_dma_get_v1`** (landed R6.7 A.5,
2026-05-03), is the project's first DMA-bound replay-validated
fixture. It asserts the inverse: **exactly one** `spu_mfc_cmd`
event (cmd=0x40 GET, tag=3, size=128, lsa=0x10000) plus exactly
one `mfc_dma_complete` event. See § 11 closure section for the
full landing record.

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
| A.5 | First CC0 fixture `single_spu_dma_get_v1`: PPU writes table to EA, SPU GETs, computes checksum, halts. .self built via Docker. JSONL captured. .dmachunk + .spuimg produced. **2026-05-02 v2 — PARTIAL: `.self` built + RPCS3 OFF canonical TTY confirmed; trace capture BLOCKED on rpcs3.exe rebuild**. PSL1GHT toolchain via Docker (`rpcs3-ps3dev-toolchain:local`) successfully built `single_spu_dma_get_v1.self` (939,475 bytes, sha256 `7b0761849ff64048dd4852d8fa9361cb70cec2dfe08ec5ef54e911fc53b333a0`, committed at `behavior-freeze/fixtures/spu/sources/single_spu_dma_get_v1/build/`). RPCS3 OFF runs the .self and reproduces the canonical TTY `[dma_get_v1] OK cause=0x1 status=0xdeada12f` exactly as designed. **However** the RPCS3 binary at `rpcs3-upstream-clean/bin/rpcs3.exe` is the R6.5b build (predates R6.7 A.1 patches) — capturing a trace with it produces JSONL events for `spu_image` / `spu_wrch ch28` / `spu_stop` / `final_state` but NO MFC events (no `spu_wrch ch16-23` / `spu_mfc_cmd` / `mfc_dma_complete` / `spu_rdch ch24`). To produce a full A.5 trace, the rpcs3.exe must be rebuilt with the R6.7 A.1 scaffolding patch (`cda976d7…`) + runtime hooks patch (`95bdcaae…`) applied. Source edits to `rpcs3-upstream-clean/rpcs3/Emu/Cell/{SPUTraceJsonl.h,SPUTraceJsonl.cpp,SPUThread.cpp}` were authored (R6.7 A.1 hooks ported to upstream-clean's R5.8/R5.9e.3 base) but the MSBuild rebuild hits cascading dependency issues: missing FFmpeg libs in `build/lib/Release-x64/` (resolvable by copying from `3rdparty/ffmpeg/lib/windows/x86_64/`), missing LLVM libs (resolvable by copying from `build/lib/Release-x64/llvm_build/lib/`), missing abseil libs (resolvable by copying from `build/lib/Release-x64/protobuf_build/lib/`), and a glslang↔spvtools mismatch (glslang.lib was rebuilt with new `SpvTools.cpp` references that pull in unresolved spvtools optimizer symbols — needs SPIRV-Tools subproject built from source which is out of scope for a single capture session). Resume path: (a) restore a known-good R6.7-aware rpcs3.exe (e.g., from CI artifact / dedicated build session), or (b) resolve the SPIRV-Tools build path in upstream-clean's CMake setup (likely needs `git submodule update --init --recursive` for the glslang submodule's External/spirv-tools subdir + a CMake configure pass). Once an R6.7-aware binary exists, the resume is mechanical: set `RPCS3_SPU_TRACE_JSONL`, run the .self, move side-files to canonical, remove `#[ignore]`. |
| A.6 | Replay test `single_spu_dma_get_v1_replay.rs` mirroring the 6 existing tests. Acceptance: byte-identical interpreter+recompiler, OUT_MBOX matches canonical computation. **2026-05-02 — DONE as `#[ignore]`-gated scaffolding**. New `rust/rpcs3-spu-recompiler/tests/single_spu_dma_get_v1_replay.rs` (~270 lines) lands the full test mirroring the 6 existing oracles + the R6.7-specific assertions: exactly 1 `spu_mfc_cmd` (cmd=0x40, tag=3, size=128, eah=0, lsa=0x10000), exactly 1 `mfc_dma_complete`, exactly 1 `spu_wrch ch28` carrying status=0xDEADA12F, exactly 1 `spu_stop` 0x101, post-DMA LS at `[lsa..lsa+size]` matches the counting pattern, `apply_mfc_dma_pre_replay` produces 1 dispatch + 1-element queue, both Interpreter and Recompiler reach `Finished{0x101}` byte-identical via `diff_snapshots`, both `final_snapshot.channels.out_mbox == Some(0xDEADA12F)`. Test is `#[ignore]`d with a clear ungate instruction in the attribute message; once A.5 capture lands, removing the `#[ignore]` flips this to the 7th replay-validated oracle and the project's first DMA oracle. New `apply_mfc_dma_pre_replay` + `DmaPreReplayPlan` re-exports added to `rpcs3-spu-differential::lib`. |

### Phase B — bridge honest fallback

| Step | Deliverable |
|------|-------------|
| B.1 | Bridge update: `classify_stall_channel` recognises 16-25 as MFC family with informative names + `potentially_retryable=false`. Activation log mentions DMA fallback explicitly. |
| B.2 | Verify regression: 6 existing fixtures (no DMA) + 1 new DMA fixture all run with bridge OFF (status canonical) AND ON (DMA fixture's bridge ON falls back to C++ on first MFC op without corruption; the 6 non-DMA fixtures unchanged). |

### Phase C — Rust SPU MFC channel handling (replay-only)

| Step | Deliverable |
|------|-------------|
| C.1 | Add ch16-25 to `rust/rpcs3-spu-thread/src/lib.rs` `ch::` module. **DONE 2026-05-02** — `MFC_LSA, MFC_EAH, MFC_EAL, MFC_SIZE, MFC_TAG_ID, MFC_CMD, MFC_WR_TAG_MASK, MFC_WR_TAG_UPDATE, MFC_RD_TAG_STAT, MFC_RD_TAG_MASK` constants land. |
| C.2 | Wire wrch dispatcher to update `ch_mfc_cmd` fields. **DONE 2026-05-02** — `SpuChannels` extended with `mfc_lsa`, `mfc_eah`, `mfc_eal`, `mfc_size`, `mfc_tag_id`, `mfc_wr_tag_mask`, `mfc_wr_tag_update`, `mfc_tag_stat_queue: VecDeque<u32>`. `write` accepts ch16-23 (param channels never stall, ch21 is a no-op in replay mode). `read` accepts ch24 (pops from `mfc_tag_stat_queue` or stalls) and ch25 (stateless mirror of `mfc_wr_tag_mask`). `count` updated. 4 new unit tests in `rpcs3-spu-thread`. |
| C.3 | wrch ch21 dispatcher: in REPLAY mode, replay state machine handles it (copies from `.dmachunk`). In NORMAL/runtime mode, returns `UnsupportedChannel` (bridge falls back). **DONE 2026-05-02** — replay-mode integration uses **pre-application**: new `mfc_replay::apply_mfc_dma_pre_replay(events, trace_path, canonical_dma_dir, program) -> Result<DmaPreReplayPlan, MfcReplayError>` walks the captured event stream BEFORE the SPU runs, drives `MfcReplayState`, applies GET DMA bytes into a 256 KiB LS scratch (replacing the program's segments), and collects the rdch ch24 captured values into a queue. The SPU's own `wrch ch21` during replay is then a no-op (LS already has the post-DMA bytes). Runtime DMA is still out of scope. |
| C.4 | rdch ch24 dispatcher: in REPLAY mode, returns the oracle tag stat. In NORMAL/runtime mode, returns `UnsupportedChannel`. **DONE 2026-05-02** — `SpuChannels::read(MFC_RD_TAG_STAT)` pops from `mfc_tag_stat_queue` (`WouldStall` when empty). Queue is fed via the new `SpuProgram::with_mfc_tag_stat_queue(queue) -> Self` builder + `initial_mfc_tag_stat_queue: Vec<u32>` field. Both `InterpreterExecutor::execute` and `RecompilerExecutor::execute` extend the program-load path to copy the queue into `spu.channels.mfc_tag_stat_queue` before the SPU runs. |
| C.5 | Transformer: drop MFC events as pure context (no longer hard-reject) once executor wiring lands. **DONE 2026-05-02** — `transform_single_spu_subset` now treats `SpuMfcCmd` and `MfcDmaComplete` as pure context (same as `SpuRdch` / `SpuWrch` for non-mailbox channels). Pre-application is the layer that actually consumes them. Parser-level rejections (`UnsupportedMfcCmd`, `UnsupportedMfcEah`, `BadDmaSize`, `BadDmaLsa`, `BadDmaSha`, `BadMfcTag`, `BadDmaTransferredBytes`, `MalformedMfcSequence`, `UnsupportedDmaInTrace` for bare `wrch ch21`) all stay in place. End-to-end test `replay_executor_get_dma_copies_chunk_to_ls` runs synthetic SPU bytecode (assembled via `rpcs3_spu_interpreter::encode`) through the actual `InterpreterExecutor` with pre-applied DMA + populated tag-stat queue, asserting post-DMA LS holds the chunk bytes AND r10 (rdch ch24 destination) equals the popped tag-stat value. |

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
- Trace patches unchanged: `cda976d7…` / `95bdcaae…` pinned (sha
  updated post-R6.7 A.1 vs original `d65aec91` / `8f253d7d`).
- v4 still diagnostic-only.
- No `.dmachunk` files exist yet anywhere (relaxed post-A.5: one
  `.dmachunk` now exists at canonical `behavior-freeze/fixtures/spu/dma/471fb943…2be5.dmachunk`).
- No new schema variants in code (relaxed post-A.2: `spu_mfc_cmd`
  and `mfc_dma_complete` are now landed schema additions).

---

## 13. R6.7 closure (2026-05-03)

R6.7 design + implementation phases A, C are **complete**. Phases
B (bridge honest-fallback) and D (bridge runtime DMA opt-in) move
to R7.

### 13.1 Landed in R6.7

| Phase | Deliverable | Status | Memory ref |
|---|---|---|---|
| Design | This document (§ 1-9) | ✅ committed 2026-05-01 | `project_r67_dma_mfc_design.md` |
| A.1 | Writer extension — emits `spu_mfc_cmd` + `mfc_dma_complete` + content-addressed `.dmachunk` side-files | ✅ landed in upstream-clean C++ tree | `project_r67_a1_dma_writer_extension.md` |
| A.2 | Parser extension (`trace_fmt.rs`) — recognizes the new event kinds with full validation; 8 rejection codes; ordering invariant ch21→spu_mfc_cmd | ✅ 12 new tests | `project_r67_a2_dma_parser_extension.md` |
| A.3 | DMA chunk loader (`dma_chunk.rs`) — `resolve_dma_chunk_side_file()` per-trace + canonical fallback; SHA-256 + size validation; 7 error variants | ✅ 11 new tests | `project_r67_a3_dma_chunk_loader.md` |
| A.4 | `MfcReplayState` (`mfc_replay.rs`) — state machine for ch16-25 + tag-stat queue + Immediate/Any/All wait modes | ✅ 13 new tests | `project_r67_a4_mfc_replay_state.md` |
| Phase C | Executor wiring — ch16-25 in `rpcs3-spu-thread::ch`; `SpuChannels` extended; `apply_mfc_dma_pre_replay()` helper; transformer accepts MFC events as pure context | ✅ end-to-end synthetic test green | `project_r67_phase_c_mfc_executor_wiring.md` |
| A.5 | First DMA-bound replay-validated oracle `single_spu_dma_get_v1` | ✅ landed 2026-05-03 | `project_r67_a5_landed_7th_oracle.md` |

### 13.2 The seventh oracle: `single_spu_dma_get_v1`

**Load-bearing acceptance:** OUT_MBOX = `0xDEADA12F` is only
reachable when (a) the GET actually copied 128 bytes from EA into
LS at lsa=0x10000, AND (b) the SPU computed the deterministic
post-DMA sum + XOR. A silent fake-DMA path (zero-fill LS) would
produce `0xDEADBEEF` (different) — the canonical comparison
distinguishes "no DMA" from "wrong DMA" from "right DMA".

**Trace shape (15 events):**

```
seq  0: spu_image          sha=97a38063…  size=0x40000
seq  1: spu_wrch ch16=0x10000     (MFC_LSA)
seq  2: spu_wrch ch17=0           (MFC_EAH)
seq  3: spu_wrch ch18=0x10068400  (MFC_EAL)
seq  4: spu_wrch ch19=128         (MFC_Size)
seq  5: spu_wrch ch20=3           (MFC_TagID)
seq  6: spu_wrch ch21=0x40        (MFC_Cmd = GET)
seq  7: spu_mfc_cmd  cmd=0x40 tag=3 size=128 lsa=0x10000
                     eah=0 eal=0x10068400
                     ea_chunk_sha256=471fb943…2be5
seq  8: mfc_dma_complete  tag=3  transferred=128
seq  9: spu_wrch ch22=0x8         (WrTagMask)
seq 10: spu_wrch ch23=0x2         (WrTagUpdate = ALL)
seq 11: spu_rdch ch24=0x8         (RdTagStat returns 1<<3)
seq 12: spu_wrch ch28=0xDEADA12F  (OUT_MBOX = canonical)
seq 13: spu_stop  stop_code=0x101
seq 14: final_state  r18=0x1FC0 r19=0xDEADBEEF r20=0xDEADA12F
```

**Canonical artifacts:**

- `behavior-freeze/fixtures/spu/traces/single_spu_dma_get_v1.jsonl`
  (15 events, 2,347 bytes)
- `behavior-freeze/fixtures/spu/images/97a38063…ef56.spuimg`
  (262,144 bytes — full LS at thread create)
- `behavior-freeze/fixtures/spu/dma/471fb943…2be5.dmachunk`
  (128 bytes — sum=8128, counting pattern 0x00..0x7F)

### 13.3 Capture forensics (load-bearing for re-capture)

Two non-obvious gotchas surfaced during the A.5 build-unblock
work. Both must be re-established before any future R6.7 / R7
re-capture from this MSBuild tree:

1. **`R:\` SUBST drive must be active during build.** The
   `rpcs3-upstream-clean/build/tmp/rpcs3-Release-x64/rpcs3.tlog/link.command.1.tlog`
   has **545 burned-in `R:\` paths** (`/LIBPATH:R:\BUILD\LIB\RELEASE-X64\GLSLANG`,
   `R:\BUILD\TMP\RPCS3-RELEASE-X64\*.OBJ`, `/OUT:R:\BIN\RPCS3.EXE`).
   Without an active SUBST, link.exe silently skips the missing
   `R:\` paths and falls through to `$(VULKAN_SDK)\Lib\glslang.lib`
   (75 MB, /MD CRT vs our /MT) → 52 LNK2001 unresolved
   `spvtools::Optimizer` externals from `glslang.lib(SpvTools.obj)`.
   Fix: `subst R: "<repo-root>"` before invoking msbuild.

2. **Interpreter mode required for the CAPTURE run only.** RPCS3
   LLVM JIT bypasses the C++ `set_ch_value()` / `get_ch_value()`
   for MFC channels (and all per-channel paths in general — the
   existing 6 pre-A.5 oracles only get `spu_image + ch28 wrch +
   stop + final_state` from JIT mode because their replay
   reconstruction doesn't need per-channel events). The R6.7 A.1
   trace hooks live INSIDE `set_ch_value()`, so JIT inlining
   bypasses them. To capture full ch16-23 wrch + ch24 rdch +
   spu_mfc_cmd + mfc_dma_complete events, set
   `bin/config/config.yml`:
   - `Core: SPU Decoder: Interpreter (static)`
   - `Core: PPU Decoder: Interpreter (static)`

   Restore to `Recompiler (LLVM)` after capture. The fixture's
   `.notes.md` documents this in detail.

### 13.4 What R7 covers (Phase B + Phase D — runtime DMA)

R6.7 GET-only replay is complete. The remaining MFC work moves
to R7:

**R7 scope (proposed):**

1. **Phase B — bridge honest-fallback (was deferred from R6.7
   § 7.1):** when the runtime bridge encounters `MFC_Cmd` on a
   delegated SPU, return `BridgeOutcome::FellBackToCpp` rather
   than attempting DMA. The C++ side then handles the cmd
   natively. Validates that bridge ON / OFF still produce the
   same canonical TTY (`[dma_get_v1] OK cause=0x1
   status=0xdeada12f`) for the A.5 fixture.

2. **Phase D — bridge runtime DMA opt-in:** the bridge gains
   real DMA execution via FFI back into RPCS3's
   `process_mfc_cmd()`. This is the path needed for any SPU
   program that branches on DMA results before stop (the A.5
   fixture does — `cs = sum(LS[lsa..lsa+128]) ^ 0xDEADBEEF` is
   computed AFTER the GET, then written to OUT_MBOX). Phase D
   acceptance: bridge ON executes the A.5 .self end-to-end and
   produces byte-identical state vs bridge OFF. The replay
   oracle stays unchanged.

3. **R7 non-scope (still deferred to R8+):**
   - MFC PUT (LS → EA). Symmetric to GET but requires capturing
     EA-before-PUT bytes; not in R7.
   - DMA list cmds (GETL, PUTL, GETLB, etc.). Need per-list-element
     event sequence.
   - Atomic ops (GETLLAR, PUTLLC, PUTLLUC, PUTQLLUC). LL/SC
     reservation tracking is its own work item.
   - MFC barriers / fence bits. Not in scope until at least 2
     overlapping DMAs are observed in a CC0 fixture.
   - Multi-SPU DMA races on shared EA regions. R6.7+R7 single-SPU
     only.

**What stays DIAGNOSTIC-ONLY forever:**

- `tests/data/spurs_test_v3_real.jsonl` (R5.9d-era multi-SPU
  SPURS) — DMA-bound, contains commercial code, never promoted.
- `tests/data/spurs_test_v4_real.jsonl` (R5.10a..p ISA-coverage
  iteration's working trace) — DMA-bound at the protocol level,
  contains commercial code, never promoted. v4 informed the
  ISA-coverage push but is now retired; R6.7 closes the cycle by
  delivering the fresh CC0 `single_spu_dma_get_v1` oracle as the
  canonical first DMA-bound trace.

### 13.5 Hard rules carried forward to R7

The R6.7 § 11 rules ("Explicit refusal of fake DMA") are
**unchanged** and carry into R7. The seventh oracle is the
load-bearing proof that real captured DMA bytes — round-tripped
through SHA-256 + content-addressed `.dmachunk` side-file +
strict size/lsa validation — replay byte-identical across
Interpreter and Recompiler. R7 must preserve this contract;
synthesising MFC success without an oracle (replay) or RPCS3
vm:: (runtime) is a hard reject. No manual JSONL editing. No
fake `.dmachunk` content.

### 13.6 Final acceptance state at R6 closure

- ✅ 7 replay-validated oracles green (cross-backend byte-identical)
- ✅ workspace `--lib --no-fail-fast` green
- ✅ `check_trace_fixtures.py` green (7 fixtures listed)
- ✅ `check_patch_separation.py` green (3 SHA-pinned patches intact)
- ✅ Bridge `7d6b6bba…` unchanged
- ✅ Scaffolding `cda976d7…` + runtime hooks `95bdcaae…` unchanged
- ✅ v4 still diagnostic-only
- ✅ `single_spu_dma_get_v1` is the project's first DMA-bound
  replay-validated oracle; OUT_MBOX = `0xDEADA12F` canonical

---

## 14. R8.1 closure note (2026-05-19) — PUT direction

R8.1 extends the design with the symmetric inverse of GET. The
state machine, writer, parser, executor wiring, FFI, and bridge
all gain a PUT branch that mirrors GET semantics with the data
direction reversed (LS → EA) and one new load-bearing invariant:
**the captured `.dmachunk` for a PUT MUST byte-match the SPU's
LS at the dispatch lsa**. The state machine surfaces a
divergence as `MfcReplayError::PutLsBytesMismatch` — never
silently coerced.

### 14.1 Architectural deviation from § 9.7

§ 9.7 (PUT discussion) predicted PUT would require in-line state
machine wiring with the executor because the SPU writes LS bytes
BEFORE dispatch and the assertion must fire AT dispatch time.
R8.1 ships PUT support without that wiring change by introducing
two state-machine entry points:

- `process_mfc_cmd` (existing) — AssertNow semantics. Caller
  guarantees `ls` is the SPU's LS at dispatch time. Suitable for
  future in-line executor integration.
- `process_mfc_cmd_pre_replay` (NEW) — PUT route defers the LS
  assertion to a post-replay step in the test layer. The chunk
  is still loaded (validates side-file SHA + size), the pending
  packet cross-check runs, the in-flight tag is registered. The
  deferred assertion is performed by the replay test against
  both backends' final LS — for the canonical fixture this is
  equivalent to dispatch-time assertion because the SPU doesn't
  touch LS post-PUT. A future R-phase driving the state machine
  in-line with the executor would restore the dispatch-time
  contract automatically.

### 14.2 R8.1 acceptance state

- ✅ 8 replay-validated oracles green (cross-backend byte-identical)
- ✅ workspace `--lib --no-fail-fast` green (all crates pass)
- ✅ `check_trace_fixtures.py` green (8 fixtures listed)
- ✅ `check_patch_separation.py` green (3 SHA-pinned patches:
  rust bridge bumped to `0afda1c6…`, runtime hooks bumped to
  `1f598d37…`, scaffolding unchanged `cda976d7…`)
- ✅ `check_triple_symmetry.py --fixture get` green (R7.3 carry)
- ✅ `check_triple_symmetry.py --fixture put` green (R8.1 new)
- ✅ rpcs3.exe `3ef63a825f9820373bb1df175bc975d5063f531b98206860fab36a50a8cd95d2`
- ✅ v4 still diagnostic-only (PUT path landed without promoting
  any commercial DMA capture)
- ✅ `single_spu_dma_put_v1` is the project's first PUT-bound
  replay-validated oracle; spu sentinel = `0xC0FFEECA`,
  ea_status = `0xCAFEA57E` (both canonical, both byte-identical
  across the three execution paths)

### 14.3 R8.1 hard rules carried forward to R8.2+

The § 11 rules ("Explicit refusal of fake DMA") plus § 13.5
("Hard rules carried forward to R7") continue verbatim into
R8.2+. The PUT branch specifically adds these:

- No silent fake-PUT path. If the SPU's LS bytes diverge from
  the captured chunk, the state machine reports
  `PutLsBytesMismatch` with `{tag, lsa, size, first_diff_offset,
  captured, observed}` — never coerced to make replay pass.
- No manual `.dmachunk` editing. The PUT chunk is captured at
  runtime from `this->ls + mfc_lsa` and content-addressed by
  SHA-256; hand-editing the file breaks the SHA pin in the
  `spu_mfc_cmd` event.
- PUT scope explicitly excludes list (PUTL/PUTLB), atomic
  (PUTLLC/PUTLLUC/PUTQLLUC), and barrier/fence variants. R8.1
  fixture targets cmd 0x20 ONLY; the parser canary moves to
  cmd 0x44 GETL to keep the rejection surface tight.

---

## 15. R8.2 closure note (2026-05-20) — multi-DMA GET coverage

R8.2 closed on the same day as the first delivery attempt: it is
a **pure fixture-only delivery** with zero engine-side code
changes. The 9th oracle `single_spu_dma_get_multi_v1` exercises
two queued GETs (tags 3 + 5, distinct EAs / sizes / LSAs) plus
ALL wait mode plus a multi-bit `WrTagMask`. All mechanics were
already correctly implemented in the 8-oracle baseline; R8.2 is
a coverage gain that locks them as a regression sentinel.

### 15.1 Why no code changes

The R6.7 A.4 design anticipated multi-tag in-flight: `process_mfc_cmd`
inserts into an `HashMap<u32, MfcInFlight>` keyed by tag, and
`process_mfc_dma_complete` removes by tag. Wait modes
(Immediate / Any / All) compute `observed_now =
completed_tags & wr_tag_mask` and gate accordingly. The R6.7 A.4
unit test `mfc_replay_handles_wr_tag_mask_update_basic` already
exercised exactly the R8.2 mechanic — 2 dispatches (tags 3 + 5,
mask 0x28, ALL mode) — but on synthetic events. R8.2 promotes
that synthetic scenario to a real-binary oracle backed by a
captured trace + `.dmachunk` pool entries.

Bridge ON multi-dispatch works for the same reason: the R7.2
runtime DMA GET callback is invoked **per `wrch ch21`**, and
`try_delegate_execution` installs it once per session. Two
back-to-back wrches → two callback invocations → two
`vm::_ptr<u8>` copies → two tag-stat queue entries. The R7.2
documentation already noted "multiple GETs in the same session
work transparently"; R8.2 is the first empirical confirmation.

### 15.2 R8.2 acceptance state

- ✅ 9 replay-validated oracles green (cross-backend byte-identical)
- ✅ workspace `--lib --no-fail-fast` green (zero failures across
  all crates)
- ✅ `check_trace_fixtures.py` green (9 fixtures listed)
- ✅ `check_patch_separation.py` green (3 SHA-pinned patches
  UNCHANGED from R8.1 — no regenerations needed)
- ✅ `check_triple_symmetry.py --fixture {get,put,get_multi}`
  all three green
- ✅ rpcs3.exe unchanged (`3ef63a82…`, same R8.1 binary)
- ✅ v4 still diagnostic-only (R8.2 lands without promoting
  any commercial DMA capture)
- ✅ `single_spu_dma_get_multi_v1` is the project's first
  multi-DMA replay-validated oracle; status = `0xE12DEA4E`
  (= ((0x1FC0 << 16) | 0x1080) ^ 0xFEEDFACE) byte-identical
  across the three execution paths

### 15.3 R8.2 hard rules carried forward to R8.3+

The § 11 + § 13.5 + § 14.3 rules carry verbatim. The multi-DMA
branch specifically adds these:

- No silent fake-DMA path for either GET in the multi sequence.
  Each chunk must round-trip via the content-addressed pool
  (per-trace + canonical resolver, R6.7 A.3).
- The two GET dispatches MUST be captured as two distinct
  `spu_mfc_cmd` events in canonical order (wrch ch16-21 →
  spu_mfc_cmd → mfc_dma_complete). The parser's ordering
  invariant catches interleaved dispatches.
- ALL mode in the state machine MUST gate `rdch ch24` until
  every bit in the mask has fired its complete. Returning the
  mask prematurely (off-by-one in wait satisfaction) would
  surface as a Rust-side `MissingMfcDmaComplete` error during
  pre-replay; the engine never weakens the contract to "make
  replay pass".
- Multi-DMA scope explicitly excludes list / atomic / barrier
  variants. R8.2 covers cmd=0x40 GET only, exactly 2
  dispatches, distinct tags. Three-or-more dispatches are
  in-scope mechanically (the data structures generalize) but
  no fixture exercises that case yet.

---

## 16. R8.3a closure note (2026-05-20) — ANY wait mode + ch24 drain-aggregate

R8.3a is the first DMA fixture to surface a **real runtime/replay
divergence** and co-fix it. The 10th oracle
`single_spu_dma_get_any_v1` exercises `WrTagUpdate = ANY` (= 1)
on top of the R8.2 multi-DMA shape. The fixture's SPU embeds
the actual ch24 returned value into the canonical OUT_MBOX
status via `(tag_stat << 24) ^ 0xBEEFBEAD` — this is what
exposed the bug.

### 16.1 The divergence

C++ executor (`rpcs3/Emu/Cell/SPUThread.cpp`'s `process_mfc_cmd`)
exposes `RdTagStat` (ch24) as `completed_tags & wr_tag_mask` —
a snapshot of all completed tag bits intersected with the
current SPU mask register. Pre-R8.3a, the Rust SPU runtime
treated `mfc_tag_stat_queue` as a FIFO: each ch21 GET/PUT
dispatch callback pushed `1 << tag`; each ch24 read popped one
front entry.

For single-DMA fixtures (R6.7 GET v1, R8.1 PUT v1), pop-one
worked because the queue always had exactly one entry. For
R8.2 multi-DMA ALL, pop-one returned `1 << 3 = 0x8` instead of
`0x28`, but the SPU C source discarded the value (`(void)
tag_stat;`) so the divergence didn't surface in the canonical
status. R8.3a's tag_stat embed broke the latency.

### 16.2 The fix

Single function, single file: `SpuChannels::read` in
`rust/rpcs3-spu-thread/src/lib.rs`. New shape:

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

**Drain-OR-AND** semantics:
- **Drain**: empty the queue in one read.
- **OR**: aggregate all entries into a single `completed`
  bitmap (matches the C++ executor's `completed_tags` register).
- **AND** with `mfc_wr_tag_mask`: filter per the SPU's wait
  mask (matches the C++ executor's mask-intersection).

This unifies the two producer paths:
- **Pre-replay** pushes a single pre-aggregated value
  (= the captured ch24 return) per `spu_rdch ch24` event in
  the trace. Drain returns that value; AND is a no-op (the
  captured value is already mask-filtered by RPCS3).
- **Runtime** pushes individual `1 << tag` bits per ch21
  dispatch. Drain aggregates them; AND filters.

### 16.3 Limitation (documented for R8.4+)

The drain semantic empties the queue. A future fixture that
performs MULTIPLE ch24 reads in the same SPU session (e.g.
polling at different wait windows) would see the first read
consume all pending bits and subsequent reads stall. That's
NOT Cell BE behavior — the hardware exposes `completed_tags`
as a register that retains state across reads. The R8.3a fix
is observationally correct for one-shot reads (all 10 current
oracles do exactly one ch24 read per session) but a future
fixture would force a refactor to a persistent
`completed_tags: u32` field on `SpuChannels`.

This is the empirical-scoping policy in action: the smallest
fix that closes the current gap, documented limit so the next
divergence is anticipated.

### 16.4 R8.3a acceptance state

- ✅ 10 replay-validated oracles green (cross-backend byte-identical)
- ✅ workspace `--lib --no-fail-fast` green (all crates)
- ✅ `check_trace_fixtures.py` green (10 fixtures listed)
- ✅ `check_patch_separation.py` green (3 SHA-pinned patches
  UNCHANGED — R8.3a fix is Rust core only, no C++ patch
  regeneration)
- ✅ `check_triple_symmetry.py --fixture {get,put,get_multi,get_any}`
  all four green
- ✅ rpcs3.exe rebuilt to `3d25d782…` (relinked new
  `rpcs3_spu_ffi.lib`)
- ✅ v4 still diagnostic-only
- ✅ `single_spu_dma_get_any_v1` is the project's first
  ANY-wait-mode oracle AND the first oracle whose authoring
  exposed a real engine bug; status = `0x892FAE2D` byte-identical
  across the three execution paths.

### 16.5 R8.3a hard rules carried forward to R8.4+

The § 11 + § 13.5 + § 14.3 + § 15.3 rules carry verbatim.
The ANY branch + ch24 drain-aggregate fix specifically adds:

- No silent ch24 truncation. `completed & mfc_wr_tag_mask`
  is the load-bearing semantic; weakening to "return whatever
  is in the queue head" recreates the R8.3a divergence.
- No fake RdTagStat in fixtures. The captured ch24 IS the
  canonical for the backend that produced the trace. If a
  future backend (real hardware, async-DMA emulator) returns
  a different value, that's a backend change → re-capture,
  re-document, bump the oracle.
- ANY mode oracles MUST embed the tag_stat into the canonical
  status arithmetic. Discarding the value via `(void)` would
  hide divergences (the R8.2 latency).
- One ch24 read per SPU session in R8.3a-era oracles. The
  drain-clear semantic doesn't support multi-read polling;
  R8.4+ must refactor to persistent `completed_tags` if such
  a fixture is required.

---

## 17. R8.3b closure note (2026-05-20) — persistent completed_tags

R8.3b lifted the "one ch24 read per session" limitation by
adding the `completed_tags: u32` field to `SpuChannels` and
making ch24 reads NEVER clear it. The 11th oracle
`single_spu_dma_tag_poll_v1` performs two ch24 reads with
distinct masks in the same SPU session — exactly the pattern
that the R8.3a drain-clear semantic could not handle.

### 17.1 The divergence

The R8.3a fix drained the queue on each read but the queue
emptied permanently — there was no persistent state to feed
subsequent reads. Real Cell BE / RPCS3 C++ exposes
`completed_tags` as a register that:

- Accumulates bits as MFC DMA completes fire.
- Is read via ch24 (mask-filtered) but NOT cleared on read.
- (Some Cell BE generations support per-bit clear via
  `WrTagUpdate=IMMEDIATE` write — deferred to R8.4+.)

A SPU program that polls multiple tag subsets across the same
wait window depends on this persistence. With the R8.3a
implementation, the second ch24 read in
`single_spu_dma_tag_poll_v1` returned `WouldStall` because the
queue had drained to empty during the first read. Bridge ON
fell back to C++ at the stall outcome (correct fallback
behavior, but NOT what triple-symmetry expects from a fully
delegated session).

### 17.2 The fix

Single field + single function:

```rust
pub struct SpuChannels {
    pub mfc_tag_stat_queue: VecDeque<u32>,  // existing producer queue
    pub completed_tags: u32,                // R8.3b — persistent register
    // ... other fields ...
}

ch::MFC_RD_TAG_STAT => {
    // Drain queue: absorb any newly-arrived bits into completed_tags.
    while let Some(v) = self.mfc_tag_stat_queue.pop_front() {
        self.completed_tags |= v;
    }
    if self.completed_tags == 0 {
        return Err(ChannelStatus::WouldStall);
    }
    // Mask-filter return, NEVER clear completed_tags.
    Ok(self.completed_tags & self.mfc_wr_tag_mask)
}
```

The producer queue is retained for backwards compat:

- **Pre-replay** (R6.7 C.3 `apply_mfc_dma_pre_replay`) still
  pushes one entry per captured `spu_rdch ch24` event. On
  the first read in replay, all such entries get absorbed
  into `completed_tags` — subsequent reads see the same
  state.
- **Runtime callbacks** (R7.2 GET + R8.1 PUT) still push
  `1 << tag` per ch21 dispatch. The next ch24 read absorbs
  whatever has accumulated since the prior absorption.

### 17.3 What R8.3b does NOT do

R8.3b adds persistence ACROSS READS but not clearing
semantics. Specifically:

- `MFC_TAG_UPDATE_IMMEDIATE` (mode 0) intentionally clears
  `completed_tags` per-bit on read in real Cell BE. R8.3b
  oracles use ANY mode exclusively; Immediate is in-scope
  mechanically but no oracle exercises it.
- Some Cell BE implementations clear `completed_tags & mask`
  on `WrTagUpdate` writes (semantic varies by silicon).
  R8.3b ignores any such side effect.

These limitations are deferred to R8.4+ when an oracle forces
them, per the empirical-scoping policy.

### 17.4 R8.3b acceptance state

- ✅ 11 replay-validated oracles green (cross-backend byte-identical)
- ✅ workspace `--lib --no-fail-fast` green (all crates)
- ✅ `check_trace_fixtures.py` green (11 fixtures listed)
- ✅ `check_patch_separation.py` green (3 SHA-pinned patches
  UNCHANGED from R8.1 — R8.3a/R8.3b fixes are Rust-core only)
- ✅ `check_triple_symmetry.py --fixture {get,put,get_multi,
  get_any,get_tag_poll}` all five green
- ✅ rpcs3.exe rebuilt to `34ec50d7…` (relinked fresh
  `rpcs3_spu_ffi.lib`)
- ✅ v4 still diagnostic-only
- ✅ `single_spu_dma_tag_poll_v1` is the project's first
  repeated-RdTagStat polling oracle; status `0xDD1EAA5C`
  byte-identical across the three execution paths.

### 17.5 R8.3b hard rules carried forward to R8.4+

- No silent ch24 clearing. `completed_tags` is persistent;
  any future clearing must happen via explicit `WrTagUpdate`
  semantic landing in its own R-phase.
- Polling fixtures must embed BOTH (or all N) ch24 returned
  values in the canonical status arithmetic. Discarding any
  read via `(void)` hides per-read divergences (cf. R8.2's
  latent bug surfaced by R8.3a's tag_stat embed).
- The cargo-cache rebuild gotcha is now documented: for
  Rust-core-only fixes that need to relink rpcs3.exe,
  `touch` a source file in the dependency graph before
  `cargo build` to force a fresh `.lib`.
