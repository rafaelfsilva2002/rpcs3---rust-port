# SPU PPU↔SPU Trace Capture Schema (R5.7)

**Status:** Schema documented. As of the current iteration:
- **R5.7 (schema):** documented here.
- **R5.8 A.1+A.2 (Rust pipeline):** parser + transformer + R5.6 round-trip equivalence test — **shipped**.
- **R5.8 A.3 (C++ scaffolding):** trace-writer infrastructure (`rpcs3/Emu/Cell/SPUTraceJsonl.{h,cpp}`) and build-system entries — **shipped**.
- **R5.8 A.3 final (runtime hooks + real capture):** runtime hooks NOT applied in any hot-path C++ source; no real trace captured yet. **Deferred** to a build-capable maintainer. See [`SPU_TRACE_CAPTURE_PATCH.md`](./SPU_TRACE_CAPTURE_PATCH.md) for the integration patch and [`SPU_TRACE_CAPTURE_RUNTIME_HOOKS.md`](./SPU_TRACE_CAPTURE_RUNTIME_HOOKS.md) for the validation checklist.

This document defines the wire format. The schema closes the design gap so an implementer with C++ access can drop in the runtime hooks without further design questions.

---

## Motivation

R5.5 shipped a deterministic `&[TraceEvent]` replay engine. R5.6 demonstrated it on a synthetic homebrew-like command-dispatch fixture. The remaining correctness gap, repeated explicitly in `docs/PROJECT_STATUS.md`'s "What not to claim yet" section, is:

> No real (captured) PS3 SPU ELF + trace pair is committed yet.

R5.7 produces the exact capture schema needed to close that gap. With this doc in hand:

1. An RPCS3 C++ patch can be written that emits a deterministic per-event log when a homebrew runs under the C++ emulator.
2. The captured log can be transformed into `&[TraceEvent]` and replayed against the Rust SPU stack (interpreter and recompiler) to assert byte-exact equivalence.
3. Any divergence — wrong popped value, off-PC park, wrong wake kind, GPR mismatch — surfaces as a `TraceReplayError` keyed at the failing event index.

The schema is intentionally narrow: single-SPU, no timing, no concurrency. Multi-SPU traces and PPU-thread interleaving are explicit non-goals here and are deferred to a future R5.8+ wave.

---

## Scope

**In scope (this doc):**

- Container format (JSONL) with rationale.
- Header fields shared by every event.
- Per-side event types and payloads.
- Field type / range / endianness / unit definitions.
- Determinism requirements the capture must satisfy.
- Conceptual instrumentation hooks in RPCS3 C++ (file/function-level, not patch-line precise).
- Mapping table from captured events to R5.5 `TraceEvent` variants.
- Validation strategy before and after the first real trace lands.

**Out of scope (deferred to R5.8+):**

- The C++ instrumentation patch.
- A Rust JSONL parser.
- The capture-stream → `TraceEvent` transformer.
- Multi-SPU concurrent traces.
- PPU-side thread-interleaving capture.
- Timing / performance data — capture records ordering only.
- Memory / DMA capture — only mailbox + signal traffic is recorded in this version.
- ELF binary capture — assumed to be checked in alongside the trace as a separate file (the trace references the program by hash + entry_pc).

---

## Container format: JSONL

Each line of the capture file is a single JSON object representing one event. Empty lines and lines starting with `#` are ignored (comments). UTF-8 throughout.

Why JSONL over the alternatives:

| Format | Pros | Cons | Verdict |
|---|---|---|---|
| **JSONL** | Text-reviewable, streamable, line-oriented diffing, trivial `fprintf` from C++, mature parsers everywhere | Slightly verbose for numeric-heavy content | **Chosen.** |
| Binary protobuf / FlatBuffers | Compact, schema-checked | Not text-reviewable, requires schema toolchain, bad for git diff | Rejected — diff/review is a hard requirement. |
| TOML | Already in workspace toolchain | Awkward for arrays of heterogeneous events | Rejected — JSONL fits the streaming-of-events shape better. |
| Single JSON document | Standard | Whole-file load required, no streaming, harder to append | Rejected — large traces (hundreds of events) suffer. |
| CSV | Tiny, parseable | Heterogeneous payloads require padding columns; nested fields awkward | Rejected — schema flexibility needed. |

A sample capture file with three events:

```jsonl
{"seq":0,"side":"spu","kind":"spu_rdch","pc":256,"channel":29,"value":null,"would_stall":true}
{"seq":1,"side":"spu","kind":"spu_park","pc":256,"reason":"channel_read","channel":29}
{"seq":2,"side":"ppu","kind":"ppu_push_inmbox","target_spu":0,"value":1}
```

---

## Common event header

Every event MUST contain these top-level keys:

| Key | Type | Required | Semantics |
|---|---|:---:|---|
| `seq` | u64 | yes | Monotonic global sequence number. Strictly increasing across both SPU and PPU sides. Reset to 0 at trace start. |
| `side` | string | yes | `"spu"` or `"ppu"`. |
| `kind` | string | yes | One of the per-side event kinds enumerated below. |

`seq` is the load-bearing field for determinism. The C++ patch must emit events in real program order; the JSONL writer is responsible for assigning the next `seq` value before flush. Two events with the same `seq` is a malformed trace.

---

## SPU-side events

These are emitted from inside the SPU thread's execution path (interpreter or recompiler — the schema is backend-agnostic on the C++ side too).

**R5.9+ multi-SPU note (parser landed 2026-04-28; transformer landed 2026-04-28; writer-emit landed 2026-04-28; re-capture pending):** every SPU-side event below carries a `target_spu: <u32>` field identifying which SPU emitted it. The R5.9c writer (`SPUTraceJsonl.cpp` 7 SPU-side recorders) emits the field as the first JSON field after `kind`; runtime hooks pass `this->lv2_id` (or `spu->lv2_id` from the `TraceFinalGuard` destructor; or a `trace_target_spu`/`trace_target_spu_w` snapshot in the `get_ch_value`/`set_ch_value` SPU_WrOutMbox lambda scopes). The R5.9a parser accepts the field via `#[serde(default)]` — when the field is absent (legacy R5.7/R5.8 single-SPU traces, or the synthetic `R5_6_REFERENCE_JSONL` round-trip fixture, or the pre-R5.9c real trace from `spurs_test.self`), the parser treats it as `target_spu: 0`. On real R5.9c captures, the field becomes load-bearing: each SPU's `final_state` is enforced as terminal-for-that-SPU only, and emitting another event with the same `target_spu` after its `final_state` raises `TraceParseError::EventAfterFinalState`. PPU-side events have always required `target_spu` — no shim there.

The R5.9b transformer adds a per-SPU API `captured_events_to_traces_per_spu(events) -> Result<BTreeMap<u32, Vec<TraceEvent>>, TraceTransformError>` that groups events by `target_spu` and produces one `Vec<TraceEvent>` per SPU. The legacy single-SPU `captured_events_to_trace` is now a wrapper that returns the unique group when there is exactly one SPU and otherwise refuses with `TraceTransformError::MultipleSpusUnsupportedBySingleSpuApi { spu_count }` — preventing silent flattening of multi-SPU traces by callers that haven't migrated. Replay (`replay_trace`) remains single-SPU and is deferred to R5.9e.

R5.9e.1 adds an additional SPU-side event kind, `spu_image`, that carries metadata about the SPU's local-store contents at thread-creation time (sha256, load_addr, size, entry_pc); the actual bytes live in a side-file referenced by content hash. The `spu_image` event is documented in its own section below ("R5.9e.1 — SPU image metadata + side-file layout"); the parser/writer/replay support for it is deferred to R5.9e.2/.3/.5 respectively.

### `spu_rdch`

SPU executed a `rdch` instruction.

```jsonc
{
  "seq": <u64>,
  "side": "spu",
  "kind": "spu_rdch",
  "pc": <u32>,            // PC of the rdch instruction (NOT pc+4)
  "channel": <u32>,       // 7-bit channel id
  "value": <u32 | null>,  // null when would_stall=true; the read value when would_stall=false
  "would_stall": <bool>   // true if the rdch attempted on this channel currently stalls
}
```

If `would_stall: true`, a `spu_park` event MUST follow with the same `pc` before any further SPU event for this thread.

### `spu_wrch`

SPU executed a `wrch` instruction.

```jsonc
{
  "seq": <u64>,
  "side": "spu",
  "kind": "spu_wrch",
  "pc": <u32>,            // PC of the wrch instruction
  "channel": <u32>,
  "value": <u32>,         // The value the SPU attempted to write (regardless of stall outcome)
  "would_stall": <bool>
}
```

Same park-follows-stall rule applies.

### `spu_rchcnt`

SPU executed a `rchcnt`. Never stalls.

```jsonc
{
  "seq": <u64>,
  "side": "spu",
  "kind": "spu_rchcnt",
  "pc": <u32>,
  "channel": <u32>,
  "count": <u32>          // The count value that was returned to the SPU
}
```

### `spu_park`

SPU thread parks on a channel-op stall. MUST come after a `spu_rdch` or `spu_wrch` event with `would_stall: true` and the same `pc`.

```jsonc
{
  "seq": <u64>,
  "side": "spu",
  "kind": "spu_park",
  "pc": <u32>,                              // Park PC = parking instruction's PC (NOT pc+4)
  "reason": "channel_read" | "channel_write",
  "channel": <u32>,
  "channels_at_park": <ChannelsObject> | null  // OPTIONAL — see below
}
```

`reason` mirrors `rpcs3_spu_thread::SpuParkReason` exactly: `channel_read` ↔ `ChannelRead { channel }`, `channel_write` ↔ `ChannelWrite { channel }`.

**`channels_at_park`** is an optional channel-state snapshot taken at the moment the SPU parked. When present (non-null), the R5.8 transformer emits an `ExpectChannelState` immediately after the `ExpectSpuPark` for this park — letting captures assert intermediate channel state at park boundaries (mailbox depths, signal slot values). When absent (the field is omitted from JSON), the transformer emits only the `ExpectSpuPark` for this park.

The `ChannelsObject` shape is identical to `final_state.channels`:

```jsonc
{
  "in_mbox": <u32 | null>,
  "out_mbox": <u32 | null>,
  "out_intr_mbox": <u32 | null>,
  "snr1": <u32>,
  "snr2": <u32>
}
```

Practical guidance for the C++ patch: emit `channels_at_park` only when the captured workload's correctness depends on intermediate channel state being asserted. The first park in a typical handshake is usually fully described by its `reason` + `channel` (in_mbox empty / out_mbox full is implicit), and adding `channels_at_park: null` (or omitting the field) keeps the trace compact.

### `spu_wake`

SPU thread resumes execution after a park. MUST come after the park condition was satisfied externally (by a PPU event). The next SPU event MUST be a `spu_rdch` or `spu_wrch` at the same `pc` as the preceding `spu_park`.

```jsonc
{
  "seq": <u64>,
  "side": "spu",
  "kind": "spu_wake",
  "pc": <u32>             // Wake PC = parked PC; SPU re-executes the channel op at this address
}
```

### `spu_stop`

SPU executed a `stop` (or `stopd`).

```jsonc
{
  "seq": <u64>,
  "side": "spu",
  "kind": "spu_stop",
  "pc": <u32>,
  "stop_code": <u32>      // 14-bit stop code from the instruction's immediate
}
```

After `spu_stop` only the `final_state` event MAY follow on the SPU side. No further `spu_*` events.

### `final_state`

Emitted exactly once, at SPU thread exit (after `spu_stop`). Captures the snapshot the R5.5 trace replay engine asserts against.

```jsonc
{
  "seq": <u64>,
  "side": "spu",
  "kind": "final_state",
  "gpr_lane_zero": [
    { "reg": <u32>, "value": <u32> }   // One entry per GPR the capture chose to assert
  ],
  "channels": {
    "in_mbox": <u32 | null>,
    "out_mbox": <u32 | null>,
    "out_intr_mbox": <u32 | null>,
    "snr1": <u32>,
    "snr2": <u32>
  }
}
```

`gpr_lane_zero` lists the registers the capture **chose to assert** at end-of-run, paired with their lane-0 value. Registers absent from this list are NOT asserted by the transformer — there is no "implicit zero" assumption, and the transformer never infers values from omitted entries. This is intentional: the C++ capture side decides which registers carry workload-relevant outputs (e.g., the homebrew's contract registers) and emits only those. Including every non-zero GPR is permitted but produces noisy traces with assertions on scratch / temporary registers.

If a future trace needs full 4-lane GPR capture (e.g., for fixtures that touch lanes 1–3 in observable ways), introduce a `gpr_full` field that mirrors the SPU u128 register layout. R5.7 standardizes lane-0 only because all currently-committed fixtures restrict assertions to lane 0.

---

## PPU-side events

These are emitted from inside the PPU code path that interacts with an SPU's mailboxes / signals. `target_spu` has always been required on PPU-side events. As of R5.9a, the parser also enforces per-SPU `final_state` terminality, which means a `ppu_*` event for SPU id N appearing after that SPU's own `final_state` is a parser error (`EventAfterFinalState`). Pre-R5.9a single-SPU traces (where the SPU side has no `target_spu` field and PPU side carries `target_spu: 0`) continue to parse cleanly because all events collapse to id 0 via the SPU-side default-0 shim.

### `ppu_push_inmbox`

PPU writes a value into the SPU's `in_mbox`.

```jsonc
{
  "seq": <u64>,
  "side": "ppu",
  "kind": "ppu_push_inmbox",
  "target_spu": <u32>,    // SPU id (0 for single-SPU traces)
  "value": <u32>
}
```

### `ppu_pop_outmbox`

PPU drains the SPU's `out_mbox`.

```jsonc
{
  "seq": <u64>,
  "side": "ppu",
  "kind": "ppu_pop_outmbox",
  "target_spu": <u32>,
  "value": <u32>          // The value the PPU read; MUST equal what SPU's last wrch wrote
}
```

If the PPU read on an empty mailbox, the C++ instrumentation must emit `value: null`. The transformer maps `null` → `expect: None` and `Some(v)` → `expect: Some(v)`.

```jsonc
{
  "seq": <u64>, "side": "ppu", "kind": "ppu_pop_outmbox",
  "target_spu": 0, "value": null    // empty mailbox
}
```

### `ppu_signal`

PPU OR-merges a value into one of the SPU's signal-notification slots.

```jsonc
{
  "seq": <u64>,
  "side": "ppu",
  "kind": "ppu_signal",
  "target_spu": <u32>,
  "slot": 0 | 1,          // 0 = SNR1, 1 = SNR2 — matches rpcs3_spu_thread::SpuChannels::snr indexing
  "value": <u32>
}
```

---

## R5.9e.1 — SPU image metadata + side-file layout (replay prerequisite)

**Status:** schema doc-only (R5.9e.1, landed 2026-04-28). Parser support (R5.9e.2), writer emission (R5.9e.3), `SpuProgram` builder (R5.9e.4), and replay engines (R5.9e.5/.6) are deferred to subsequent slices. See [`SPU_TRACE_R5_9E_REPLAY_PLAN.md`](./SPU_TRACE_R5_9E_REPLAY_PLAN.md) for the full plan.

Replay (Rust `replay_trace<E: SpuExecutor>`) requires the SPU's bytecode at thread-creation time. The R5.9c writer captures behavioral events but NOT the LS contents; without the image bytes, no replay is possible. R5.9e introduces a `spu_image` event whose body is METADATA ONLY — the actual bytes live in a side-file referenced by content hash. This keeps the JSONL line-oriented and small (no base64 inflation), allows automatic deduplication when multiple SPUs share an image, and lets parse + transform stages (R5.9d milestone) skip image I/O entirely.

### `spu_image`

Emitted ONCE per SPU thread, BEFORE any other SPU-side event for that `target_spu`. The SPU's LS bytes have been dumped to a sibling `.spuimg` side-file at the moment this event is emitted.

```jsonc
{
  "seq": <u64>,
  "side": "spu",
  "kind": "spu_image",
  "target_spu": <u32>,         // lv2_id of the captured SPU
  "image_sha256": "<64-hex>",  // SHA-256 of the side-file bytes (lowercase hex)
  "load_addr": <u32>,          // LS offset where the image is loaded; usually 0
  "size": <u32>,               // byte count of the captured image (= side-file size)
  "entry_pc": <u32>            // SPU's initial PC; becomes SpuProgram.entry_pc
}
```

#### Field semantics

- **`seq`** — global monotonic, same contract as every other event. The event MUST appear in `seq` order before any other SPU-side event for `target_spu`.
- **`side`** — always `"spu"`. The image is logically tied to the SPU side because it describes the SPU's LS state.
- **`kind`** — always `"spu_image"`.
- **`target_spu`** — REQUIRED. The `lv2_id` of the captured SPU (R5.9c convention). Two SPUs with the same image still emit two distinct `spu_image` events (one per `target_spu`); the side-file is shared on disk via SHA-256 dedup.
- **`image_sha256`** — REQUIRED. SHA-256 of the captured bytes, in lowercase hex (64 chars). This is the key the replay engine uses to locate the side-file. Tamper-detection: replay computes the side-file's SHA-256 at load and rejects with `ImageHashMismatch` if it differs.
- **`load_addr`** — REQUIRED. LS offset (`u32`) where the image bytes are mapped. Usually `0` (full-LS dump). Raw SPU mode may use non-zero offsets; capturing the value makes replay deterministic without further heuristics.
- **`size`** — REQUIRED. Byte count of the image. Must equal the on-disk size of the `.spuimg` file. Must be a multiple of 4 (SPU instructions are 4-byte aligned). Maximum allowed value is 262144 (256 KB = the full SPU local store).
- **`entry_pc`** — REQUIRED. The SPU's initial PC as set by the loader at thread-creation time. Replay uses this as `SpuProgram.entry_pc`; it is NOT the same as the first SPU event's PC (which may be inside a function reached after some setup).

### Side-file layout

The side-file holds the raw bytes the SPU LS contained at thread-creation time. Two layouts are documented; both are valid; the choice is per-trace and documented in the trace's `.notes.md`.

**Per-trace layout** (preferred for local-only / diagnostic traces):

```
<trace>.jsonl
<trace>.images/<sha256>.spuimg
```

Example: `tests/data/foo.jsonl` + `tests/data/foo.images/abc123…cd.spuimg`.

**Centralized layout** (preferred for committed fixtures, enables cross-trace dedup):

```
behavior-freeze/fixtures/spu/traces/<homebrew>.jsonl
behavior-freeze/fixtures/spu/images/<sha256>.spuimg
```

Used by `behavior-freeze/fixtures/spu/traces/<homebrew>.jsonl` when its `.notes.md` declares `external-image: <sha256> @ behavior-freeze/fixtures/spu/images/`. The replay engine resolves the location via the `.notes.md` directive when a sibling `<trace>.images/` directory is absent.

### Side-file content

The `.spuimg` file is **raw binary bytes** — exactly the bytes the SPU's LS contained, in LS order. No header, no encoding, no compression at the schema layer. (Per-fixture `.notes.md` MAY document a separate `.spuimg.gz` alongside if a maintainer chooses to compress for repository-size reasons; the replay engine treats `.spuimg.gz` as "decompress on read" and re-validates the SHA against the decompressed bytes.)

**Why no header / framing inside the `.spuimg`:** the JSONL `spu_image` event already carries `size`, `load_addr`, and `image_sha256` — duplicating any of those inside the side-file would create a divergence risk. The side-file is purely the captured byte stream.

### Rules

The schema enforces these invariants. R5.9e.2 (parser) and R5.9e.5 (replay) implement the corresponding errors.

1. **Hash integrity.** `image_sha256` MUST equal `sha256(<.spuimg file bytes>)`. Mismatch is a load-time replay error (`ImageHashMismatch`).
2. **Side-file required for replay; optional for parse + transform.** A trace can be parsed and transformed without the side-file present (R5.9d-style use cases) — the parser only validates the `spu_image` event metadata. The replay engine (R5.9e.5+) requires the side-file resolves; absent file = `ImageFileMissing` error.
3. **Multiple SPUs MAY reference the same `image_sha256`.** Common when several SPURS workers load the same `.spucore.elf`. Side-file is shared on disk; each SPU still emits its own `spu_image` event with its own `target_spu` (and possibly different `entry_pc` / `load_addr`).
4. **No bytes in the JSONL.** The wire format MUST NOT carry image bytes inline — neither as base64, hex, nor any other encoding. The `spu_image` event is metadata-only; bytes are out-of-band by design.
5. **`.spuimg` content is raw bytes.** Same as captured.
6. **License rules.** `.spuimg` files follow the same redistributability rules as `.jsonl` fixtures (per [`behavior-freeze/fixtures/spu/traces/README.md`](../behavior-freeze/fixtures/spu/traces/README.md)): only authorial / public-domain / explicitly-redistributable images may be committed. Commercial PS3 game extraction is forbidden.
7. **Ordering.** `spu_image` for `target_spu = X` MUST appear in `seq` order BEFORE any other SPU-side event with `target_spu = X`. Out-of-order is a parser error (`ImageEventOutOfOrder`). PPU-side events for `X` MAY appear before `X`'s `spu_image` (PPU events are observed at the PPU side and can precede the SPU's startup).
8. **No silent fallback.** If an image cannot be resolved at replay time, the engine MUST raise an explicit error (`ImageFileMissing` / `ImageHashMismatch` / `MissingImageForSpu`). Replay MUST NOT proceed with a synthesized / zero / null image — that would corrupt the oracle property of the captured trace.

### Unsupported cases (R5.9e scope-out)

These are documented at the schema layer so any future writer / parser / replay implementation knows the boundaries. Each case has an explicit error variant or rejection reason; none silently fall through.

1. **DMA-dependent traces.** Traces containing `spu_wrch` to `MFC_Cmd` (channel 21) imply LS↔main-memory DMA whose endpoints are not captured. Replay rejects with `UnsupportedDmaInTrace { target_spu, event_index }`. spurs_test (DMA-heavy SPURS runtime) falls in this bucket; spurs_test_v3 stays diagnostic local-only.
2. **Self-modifying code.** Traces containing SMC indicators (`spu_wrch` to `MFC_RdAtomicStat`, or future `dsync`/`sync` side-channel events) violate the single-image-per-SPU assumption. Replay rejects with `UnsupportedSelfModifyingCode { target_spu, event_index }`. SMC support is deferred to a future R5.9f or beyond.
3. **Multiple `spu_image` events for the same `target_spu`.** Forbidden in R5.9e — only one image per SPU per trace. A trace whose SPU re-loads its LS mid-execution (e.g., DMA-driven code overlay) is rejected at parse time with `DuplicateSpuImage { target_spu, first_index, second_index }`. Multi-snapshot traces would require a different schema (per-instruction LS deltas) and are out of scope here.
4. **Replay without an image.** A SPU whose timeline contains executed events but NO preceding `spu_image` event has no bytecode for the replay engine to step through. Replay rejects with `MissingImageForSpu { target_spu }`. Pre-R5.9e traces (R5.9c-captured spurs_test_v3) fall in this bucket on replay; this is the documented blocker that R5.9e.3 lifts.
5. **Commercial / copyrighted images.** Schema-level FORBIDDEN. Same hard rule as for `.jsonl` fixtures. Per-fixture `.notes.md` MUST document license + chain-of-custody before any `.spuimg` lands in `behavior-freeze/fixtures/spu/images/`.
6. **Non-aligned / oversized images.** `size % 4 != 0` or `size > 262144` is rejected at parse time with `BadImageSize`. SPU LS is exactly 256 KB; instruction alignment is 4 bytes.
7. **Inline byte encoding.** Any future PR proposing to embed image bytes in the JSONL (base64, hex, etc.) MUST be rejected on schema grounds — see Rule 4 above. The motivating reasons are captured in [`SPU_TRACE_R5_9E_REPLAY_PLAN.md`](./SPU_TRACE_R5_9E_REPLAY_PLAN.md) § A.1.

### Cross-trace consequences

- **`R5_6_REFERENCE_JSONL`** has no `spu_image` event and is exercised against a hand-derived `SpuProgram`, not a captured one. The R5.9e.2 parser MUST continue accepting traces without `spu_image` events; only the BUILDER (R5.9e.4) cares about images. Round-trip equivalence with `mailbox_command_protocol_trace()` is preserved unchanged.
- **`spurs_test_v3` real trace** (R5.9c-captured, R5.9d-validated) has no `spu_image` events because it pre-dates R5.9e.3 writer extension. After R5.9e.5 lands, the replay diagnostic on this trace expects `MissingImageForSpu` (or `UnsupportedDmaInTrace` if the DMA check fires first) — the trace stays diagnostic local-only.
- **First replay-validated fixture** (R5.9e.7 deliverable) is the FIRST trace that ships with `.spuimg` side-files. Naming reserved: `behavior-freeze/fixtures/spu/traces/single_spu_mailbox_v1.jsonl` + sibling `behavior-freeze/fixtures/spu/images/<sha>.spuimg`.

---

## Field-level definitions

| Field | Type | Range / unit | Notes |
|---|---|---|---|
| `seq` | u64 | `0..u64::MAX`, strictly monotonic | Trace start = 0 |
| `pc` | u32 | `0..0x40000`, multiple of 4 | SPU LS is 256 KB; PCs are 4-byte aligned |
| `channel` | u32 | `0..128`, 7-bit | Common: `IN_MBOX=29`, `OUT_MBOX=28`, `SNR1=3`, `SNR2=4`, `WROUTINTRMBOX=30` |
| `value` (rdch/wrch/mailbox) | u32 \| null | full u32 range; `null` only on stall-read or empty-pop | `null` is a JSON null, not the integer 0 |
| `count` (rchcnt) | u32 | `0..u32::MAX`; typical `0` or `1` | Mailbox/queue depth |
| `stop_code` | u32 | `0..0x4000`, 14-bit | From SPU `stop` immediate field |
| `slot` (signal) | u32 | `0` or `1` only | Index into `SpuChannels::snr[2]` |
| `reason` (park) | string | `"channel_read"` or `"channel_write"` | Maps to `SpuParkReason` |
| `target_spu` | u32 | `0..u32::MAX`; required on PPU-side events; optional on SPU-side events as of R5.9a parser (default 0 if absent — backward-compat shim for R5.7/R5.8 single-SPU traces). R5.9c writer will start emitting it on every SPU-side event. | Multi-SPU traces enforce per-SPU `final_state` terminality after R5.9a; single-SPU traces are a degenerate case (all events have `target_spu == 0`). |
| `gpr_lane_zero[].reg` | u32 | `0..128` | SPU has 128 GPRs |
| `gpr_lane_zero[].value` | u32 | full u32 range | Lane 0 (preferred slot, high u32 of the u128) |
| `image_sha256` (spu_image) | string | exactly 64 hex chars, lowercase | Replay engine uses this as the side-file lookup key. Must equal `sha256(<.spuimg bytes>)`. |
| `load_addr` (spu_image) | u32 | `0..0x40000`, multiple of 4 | LS offset where the image bytes are mapped. Usually `0` (full-LS dump). |
| `size` (spu_image) | u32 | `4..262144`, multiple of 4 | Byte count of the captured image. Must equal the on-disk size of the `.spuimg` file. Maximum is 256 KB (full LS). |
| `entry_pc` (spu_image) | u32 | `0..0x40000`, multiple of 4 | Initial PC for the SPU; becomes `SpuProgram.entry_pc` at replay. NOT necessarily the PC of the first SPU event. |

JSON numbers are decoded as `u32`/`u64` in the natural way. The schema does not depend on host endianness — the JSON writer serializes integers in their natural decimal form.

---

## Determinism requirements

A capture is valid for R5.5 replay only if it satisfies all of the following:

1. **Single SPU.** Only events for one SPU thread per trace file. Multi-SPU runs need one trace file per SPU plus an external sync log; that's R5.9+ scope.
2. **Strictly monotonic `seq`.** No two events share a `seq`. Gaps are allowed but discouraged (they are valid, just less efficient).
3. **PC accuracy.** The captured `pc` field for an SPU event MUST be the actual instruction PC at execution time, NOT pc+4. R5.4a's invariant ("park PC = channel-op PC") propagates straight through.
4. **Channel-op-event ordering.** For a stalling SPU op:
   - `spu_rdch`/`spu_wrch` with `would_stall: true` comes first.
   - `spu_park` with the same `pc` comes second.
   - PPU event(s) that resolve the stall come next (could be one or many — e.g., a series of unrelated PPU pops before the awaited push).
   - `spu_wake` with the same `pc` comes after the resolving PPU event.
   - `spu_rdch`/`spu_wrch` with `would_stall: false` and `value: <consumed>` comes last (the SPU's retry).
5. **No timing.** The schema records ordering only. Do NOT include wall-clock timestamps, duration, or scheduler ticks. R5.5 replay is determinism-driven; timing fields would cause irrelevant divergences.
6. **No DMA / memory writes.** Only mailbox + signal traffic is captured. SPU LS reads/writes from / to main memory are NOT in this version. Non-channel side effects (LS contents, GPRs not in the final state) are observable only via `final_state`.
7. **PPU-event recording precedes SPU consumption.** `ppu_push_inmbox` MUST be flushed to JSONL before the SPU's next `spu_rdch` for that channel runs. Same for `ppu_signal` and SNR consumption. This is the C++-side ordering invariant; if the patch can't guarantee it (race with the SPU thread), the capture is malformed.
8. **Final state emitted on every successful trace.** Even if the SPU exits with an error or budget exhaustion (extension events, see "Open questions"), the final_state event is emitted with the best-known channel + GPR state.

---

## Conceptual instrumentation hooks

The schema does not lock the implementer into specific RPCS3 source files — the codebase evolves and per-version paths drift. These are the conceptual hook points; the implementer should grep current sources for the matching function names.

### SPU-side hooks

| Hook point | Conceptual function | Captured event |
|---|---|---|
| Channel read entry | `SPUThread::get_ch_value(channel)` (or `RDCH` interpreter handler) | `spu_rdch` (always, regardless of stall) |
| Channel write entry | `SPUThread::set_ch_value(channel, value)` (or `WRCH` handler) | `spu_wrch` |
| Channel count read | `SPUThread::get_ch_count(channel)` (or `RCHCNT` handler) | `spu_rchcnt` |
| Channel-op stall path | The branch that yields the SPU thread when a channel access would block | `spu_park` |
| SPU thread wake-up path | The point where a parked SPU resumes execution | `spu_wake` |
| `stop` / `stopd` handler | `SPUInterpreter::STOP` / `SPUInterpreter::STOPD` | `spu_stop` |
| Thread-exit cleanup | After `cpu_task` returns / before SPU thread teardown | `final_state` |

The hooks should be lightweight — log to a per-thread buffer and flush on stop / sleep / context switch. The schema does not require sub-instruction granularity; one event per logical channel-op invocation suffices.

### PPU-side hooks

| Hook point | Conceptual function | Captured event |
|---|---|---|
| PPU writes SPU mailbox | mailbox-write helper that targets SPU's in_mbox (varies by HLE module) | `ppu_push_inmbox` |
| PPU reads SPU mailbox | corresponding pop helper | `ppu_pop_outmbox` |
| PPU sends SPU signal | SNR1 / SNR2 write helper | `ppu_signal` |

PPU hooks must run on the PPU thread that performs the action — DO NOT log from the SPU side after the fact, because the resulting `seq` ordering would be wrong.

### Final-state hook

The final_state event is the contract surface that R5.5 `ExpectGprWord` and `ExpectChannelState` events assert against. It MUST be emitted after `spu_stop` and before the trace file is closed.

GPR lane-0 enumeration: emit one `gpr_lane_zero` entry per register the capture decides to assert. Typical capture policy: emit only the homebrew's contract registers (the ones whose final values matter to the workload). Emitting every non-zero GPR is permitted but creates noisy traces with assertions on scratch / temporary registers.

---

## Mapping to R5.5 `TraceEvent`

The capture stream is richer than R5.5's `TraceEvent` enum: the SPU's own `spu_rdch`/`spu_wrch`/`spu_rchcnt` invocations are not represented as `TraceEvent` variants because the trace replay engine doesn't model SPU-instruction-level events — it models SPU **state transitions** plus PPU side actions.

The transformer (R5.8 deliverable) walks the capture stream while maintaining a small state machine and emits `TraceEvent`s at boundaries. Here is the mapping rule per captured event class:

| Captured event(s) | R5.5 `TraceEvent` produced |
|---|---|
| `spu_park { pc, reason, channel }` (no `channels_at_park`) | `ExpectSpuPark { reason, pc: Some(pc) }` |
| `spu_park { pc, reason, channel, channels_at_park: <ChannelsObject> }` | `ExpectSpuPark { reason, pc: Some(pc) }` followed by `ExpectChannelState { ... }` mirroring `channels_at_park` |
| `ppu_push_inmbox { value }` while preceding event is `spu_park` on `RDINMBOX` | `PpuPushInMbox { value, expect_wake: Ready }` |
| `ppu_push_inmbox { value }` while SPU is running (not parked) | `PpuPushInMbox { value, expect_wake: NotParked }` |
| `ppu_push_inmbox { value }` while SPU is parked on a different reason | `PpuPushInMbox { value, expect_wake: StillBlocked }` |
| `ppu_pop_outmbox { value }` while preceding event is `spu_park` on `WROUTMBOX` | `PpuPopOutMbox { expect: Some(value), expect_wake: Some(Ready) }` |
| `ppu_pop_outmbox { value }` while SPU is running | `PpuPopOutMbox { expect: Some(value), expect_wake: Some(NotParked) }` |
| `ppu_pop_outmbox { value }` while SPU is parked on a different reason | `PpuPopOutMbox { expect: Some(value), expect_wake: Some(StillBlocked) }` |
| `ppu_pop_outmbox { value: null }` (empty mailbox) | `PpuPopOutMbox { expect: None, expect_wake: ... }` |
| `ppu_signal { slot, value }` while preceding event is `spu_park` on the matching `RDSIGNOTIFY` channel | `PpuSignal { slot, value, expect_wake: Ready }` |
| `ppu_signal { slot, value }` otherwise | `PpuSignal { slot, value, expect_wake: NotParked }` or `StillBlocked` per state |
| `spu_stop { stop_code }` | `ExpectSpuFinished { stop_code }` |
| `final_state.gpr_lane_zero[i]` | One `ExpectGprWord { reg: i.reg, lane: 0, value: i.value }` per entry |
| `final_state.channels` | One `ExpectChannelState { ... }` mirroring all five fields |

Events the transformer **discards** (consumed only to maintain state-machine context, not emitted as `TraceEvent`):

- `spu_rdch` / `spu_wrch` (any) — used to track expected post-park behavior, not asserted directly.
- `spu_rchcnt` — same.
- `spu_wake` — purely a state-machine pivot; the next SPU event re-establishes flow.

**State machine** the transformer maintains:

```
Initial: SPU_RUNNING
On spu_park: transition to SPU_PARKED { reason, channel }
On spu_wake: transition to SPU_RUNNING
On spu_stop: transition to SPU_FINISHED
On final_state: emit ExpectGprWord + ExpectChannelState, terminate
```

PPU events look up the current SPU state to determine the correct `expect_wake` projection for the emitted `TraceEvent`.

---

## Validation strategy

### Phase 0 — schema-only (R5.7, this doc)

No real trace exists yet. Validation is paper-only: the schema is reviewable end-to-end and complete enough that an implementer can build the C++ patch + Rust transformer without further design questions. The "Open questions" section at the bottom of this doc enumerates known gaps for future iteration.

### Phase 1 — synthetic round-trip (no R5.7 work, listed for context)

Once a JSONL parser + transformer lands (R5.8 sub-step), validate the schema by hand-encoding the existing R5.6 synthetic trace as JSONL, decoding it back to `Vec<TraceEvent>`, and asserting the round-trip reproduces the original literal. This proves the schema → transformer pipeline is loss-free for known-good inputs before any real C++ patch runs.

### Phase 2 — first real trace (R5.8+)

When the C++ patch produces a real captured trace from a homebrew running under RPCS3:

1. Decode the JSONL into `Vec<TraceEvent>` via the transformer.
2. Run `replay_trace` against `InterpreterExecutor` — must succeed, end at `Finished`.
3. Run `replay_trace` against `RecompilerExecutor` — must produce identical report (same final state, same step count modulo JIT-vs-interp variance allowed by R5.4c contract).
4. Mutation tests: poke individual `TraceEvent`s in the decoded vec, re-run, assert specific `TraceReplayError` kinds keyed at the right `event_index`.
5. Commit the JSONL file to `behavior-freeze/fixtures/spu/traces/` plus the homebrew ELF (or a precise ELF reference if licensing prevents direct commit).

### Phase 3 — multi-trace cross-validation (R5.9+)

Once the first real trace works, encode a second (different homebrew, different protocol) as a regression sentinel. Test that the recompiler's behavior on novel real traces stays byte-exact under future JIT changes.

---

## Out of scope for R5.7

- The C++ instrumentation patch itself.
- A Rust JSONL parser. Workspace already pulls `serde_json` indirectly; when R5.8 lands, deriving `serde::Deserialize` for the schema types is a one-line addition.
- The capture-stream → `TraceEvent` transformer.
- Multi-SPU concurrent traces.
- PPU-thread interleaving capture.
- Timing / performance fields.
- DMA / memory traffic.
- Direct ELF embedding inside the trace file.
- Schema version negotiation. Add a `"schema_version": <u32>` top-level header line in R5.8 if the schema needs to evolve.

---

## Open questions for the R5.8 implementer

1. **Trace-start preamble.** Should the first line be a header object like `{"schema_version": 1, "spu_id": 0, "elf_hash": "<sha256>", "elf_entry_pc": 0x100}` rather than the first `seq=0` event? Decision: **yes**, but defer to R5.8 — for R5.7 the schema treats the trace as event-only. The header line is additive and won't break the current schema.
2. **Truncated traces.** If the SPU is killed externally before reaching `spu_stop`, what does final_state look like? Decision: emit `spu_error { message }` instead of `spu_stop`, then `final_state` with whatever GPR/channel state is recoverable. R5.5's `ExpectSpuFinished` doesn't cover this; a new `ExpectSpuError` variant on the Rust side may be needed when this case actually arises.
3. **Budget exhaustion.** RPCS3 doesn't have a step budget the way `replay_trace` does; this is a Rust-side concept. The transformer should map "trace ended without spu_stop and without spu_error" → `BudgetExhausted` from the replay's perspective.
4. **Floating-point determinism.** Is the SPU's float computation byte-exact across hosts (FTZ + denormal handling)? The Rust interpreter explicitly applies FTZ. If C++ disagrees on any platform, captured GPR state from a Linux RPCS3 run might fail replay on a Windows Rust run. Document a "reproducibility caveat" in the trace metadata until confirmed.
5. **Channel-event order vs Cell ABI semantics.** Does `cellSpursPushSpuMailbox` (or the equivalent HLE function) issue the actual channel write before returning to the caller? If the helper is asynchronous, the capture point must be at the actual mailbox-write site, not at the HLE entry. Implementer responsibility — verify with a small probe homebrew.
6. **Multiple PPU threads.** Two PPU threads racing to push the same SPU's in_mbox would interleave non-deterministically. Single-PPU-thread is implicit in this schema; multi-PPU is R5.10+ scope.
7. **Signal-merge semantics.** `ppu_signal` records the value the PPU sent, NOT the post-OR-merge slot value. The transformer must track `snr` slot state across signals to compute the right `ExpectChannelState` if assertions land between two signals. Implementer responsibility.
8. **Rd/wr count instructions during stall.** A `spu_rchcnt` issued while another op is stalled — does it run, or does the SPU thread already yield? RPCS3 should serialize them (single SPU thread), but document the assumption explicitly.

---

## Reference: R5.6 synthetic trace as JSONL

This is what the existing `mailbox_command_protocol_trace()` (in [`rpcs3-spu-differential`](../rust/rpcs3-spu-differential/src/lib.rs)) would look like if hand-encoded into the R5.7 schema. Useful for round-trip validation in R5.8 Phase 1.

```jsonl
# R5.6 synthetic mailbox-command-protocol fixture as a JSONL trace.
# Program: rdch r3,IN(29); il r4,0xFF; ceq r5,r3,r4; brnz r5,+4(HALT);
#          ai r6,r3,0x29; wrch r6,OUT(28); br -6(LOOP); stop 0xD5
# Entry pc 0x100; max_steps 200.

{"seq":0, "side":"spu","kind":"spu_rdch", "pc":256,"channel":29,"value":null,"would_stall":true}
{"seq":1, "side":"spu","kind":"spu_park", "pc":256,"reason":"channel_read","channel":29}

# PPU sends command 1 — wakes the rdch.
{"seq":2, "side":"ppu","kind":"ppu_push_inmbox","target_spu":0,"value":1}
{"seq":3, "side":"spu","kind":"spu_wake", "pc":256}
{"seq":4, "side":"spu","kind":"spu_rdch", "pc":256,"channel":29,"value":1,"would_stall":false}
{"seq":5, "side":"spu","kind":"spu_wrch", "pc":276,"channel":28,"value":42,"would_stall":false}

# Loop back — SPU parks on rdch again, in_mbox empty.
{"seq":6, "side":"spu","kind":"spu_rdch", "pc":256,"channel":29,"value":null,"would_stall":true}
{"seq":7, "side":"spu","kind":"spu_park", "pc":256,"reason":"channel_read","channel":29,"channels_at_park":{"in_mbox":null,"out_mbox":42,"out_intr_mbox":null,"snr1":0,"snr2":0}}

# PPU sends command 2 — wakes the rdch; SPU's wrch will park (out_mbox full).
{"seq":8, "side":"ppu","kind":"ppu_push_inmbox","target_spu":0,"value":2}
{"seq":9, "side":"spu","kind":"spu_wake", "pc":256}
{"seq":10,"side":"spu","kind":"spu_rdch", "pc":256,"channel":29,"value":2,"would_stall":false}
{"seq":11,"side":"spu","kind":"spu_wrch", "pc":276,"channel":28,"value":43,"would_stall":true}
{"seq":12,"side":"spu","kind":"spu_park", "pc":276,"reason":"channel_write","channel":28,"channels_at_park":{"in_mbox":null,"out_mbox":42,"out_intr_mbox":null,"snr1":0,"snr2":0}}

# PPU drains 0x2A (cmd 1's result) — wakes the wrch.
{"seq":13,"side":"ppu","kind":"ppu_pop_outmbox","target_spu":0,"value":42}
{"seq":14,"side":"spu","kind":"spu_wake", "pc":276}
{"seq":15,"side":"spu","kind":"spu_wrch", "pc":276,"channel":28,"value":43,"would_stall":false}

# Loop back — parks on rdch a third time.
{"seq":16,"side":"spu","kind":"spu_rdch", "pc":256,"channel":29,"value":null,"would_stall":true}
{"seq":17,"side":"spu","kind":"spu_park", "pc":256,"reason":"channel_read","channel":29,"channels_at_park":{"in_mbox":null,"out_mbox":43,"out_intr_mbox":null,"snr1":0,"snr2":0}}

# PPU sends halt sentinel 0xFF — SPU wakes, branches to HALT, stops.
{"seq":18,"side":"ppu","kind":"ppu_push_inmbox","target_spu":0,"value":255}
{"seq":19,"side":"spu","kind":"spu_wake", "pc":256}
{"seq":20,"side":"spu","kind":"spu_rdch", "pc":256,"channel":29,"value":255,"would_stall":false}
{"seq":21,"side":"spu","kind":"spu_stop", "pc":284,"stop_code":213}

# Final cleanup pop and final state.
{"seq":22,"side":"ppu","kind":"ppu_pop_outmbox","target_spu":0,"value":43}
{"seq":23,"side":"spu","kind":"final_state","gpr_lane_zero":[{"reg":3,"value":255},{"reg":6,"value":43}],"channels":{"in_mbox":null,"out_mbox":null,"out_intr_mbox":null,"snr1":0,"snr2":0}}
```

(The example above is annotated for readability with `#` comments and visual whitespace; the actual JSONL parser ignores blank lines and `#`-prefixed lines and requires each event to be a complete JSON object on a single line. The R5.8 test fixture
[`R5_6_REFERENCE_JSONL`](../rust/rpcs3-spu-differential/src/trace_fmt.rs) carries the same trace in strictly single-line form.)

24 captured events ↔ 16 R5.5 `TraceEvent`s after transformation. The 24:16 ratio is typical: every park is preceded by its stall observation; every wake is bracketed by a stall + retry pair. The transformer collapses this richness into the assertion-and-action shape `replay_trace` expects.

The capture only asserts `r3` (last command consumed = 0xFF) and `r6` (last computed result = 0x2B) — the workload's contract registers. Other GPRs (`r4` holding the halt sentinel, `r5` holding the ceq result) are deterministic outputs of the program but were chosen NOT to be asserted, illustrating the "capture chooses what to assert" semantic of `gpr_lane_zero`.

---

## Cross-references

- R5.5 trace replay engine: [`rust/rpcs3-spu-differential/src/lib.rs`](../rust/rpcs3-spu-differential/src/lib.rs) — search for `pub enum TraceEvent`, `pub fn replay_trace`, `pub struct TraceReplayReport`.
- R5.6 synthetic fixture: same file — search for `pub fn mailbox_command_protocol_program`, `pub fn mailbox_command_protocol_trace`.
- R5.4a parking model: [`rust/rpcs3-spu-thread/src/lib.rs`](../rust/rpcs3-spu-thread/src/lib.rs) — `SpuParkReason`, `SpuParkState`.
- R5.4b wake API: same file — `SpuWakeResult`, `try_resolve_park`, `ppu_*_and_try_wake` helpers.
- Project status & next-phase recommendations: [`docs/PROJECT_STATUS.md`](./PROJECT_STATUS.md).
- Homebrew capture / RPCS3 dump plan: [`docs/history/HOMEBREW_PLAN.md`](./history/HOMEBREW_PLAN.md) (moved from `behavior-freeze/docs/` in 2026-05-22 consolidation).
