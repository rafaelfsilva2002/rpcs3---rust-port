# Documentation Architecture Audit — 2026-05-22

**Baseline:** HEAD `0d9884201` (R8.4f-b LANDED) + working tree with 3 unstaged
docs refresh (PROJECT_STATUS § 9 / DESIGN § 19+§ 20 / traces/README item 5).

**Methodology:** 5-agent parallel fan-out (Explore inventory, acceptance-auditor
vetius-verify, comment-analyzer, fixture-notes audit, logs+history audit) +
`vetiusspec:vetius-verify` skill methodology.

**Scope:** All `.md` / `.txt` / log / state files in repo. Excludes
`target/`, `build*/`, `.git/`, `3rdparty/`, `*.obj`, `*.pdb`, `*.tlog`.

**Status:** SYNTHESIS COMPLETE — no fixes applied yet.

---

## Executive verdict

🟡 **YELLOW** — Documentation prose is GREEN (Agent 2 acceptance audit:
all PROJECT_STATUS § 9 / DESIGN § 20 / traces/README claims verified
against repo). But **code comments + fixture source READMEs carry
substantial drift** that Agent 3 + Agent 4 surfaced.

The 3 unstaged docs refresh IS necessary and sufficient for the
authoritative roadmap. But the project carries a second layer of
documentation (code doc-comments + source-dir READMEs) that has NOT
been refreshed in the R8 cycle.

**Plus one operational finding:** `rust/.claude/vetius-context.local.json`
is git-tracked despite `**/.claude/` gitignore (committed pre-ignore in
`5120e1970`) and continues producing dirty diffs forever. Untrack
candidate.

---

## Drift inventory by priority

### P0 — Blocks R8.5b workflow / engine-logic misleading

| Source | Line(s) | Drift | Fix |
|---|---|---|---|
| `rust/rpcs3-spu-differential/src/trace_fmt.rs` | 809 | User-visible error: "PUTL deferred to R8.5+" / "R8.4b/c/d will implement GETL replay/runtime" — all landed | Update to reflect that this branch now only fires for non-list codes; clarify wording |
| `rust/rpcs3-spu-differential/src/mfc_replay.rs` | 1-39 | Module banner says "GET-only DMA" / "refuses any other cmd code with UnsupportedMfcCmd" / "PUT, list, atomic NOT in scope" | Rewrite banner: GET (0x40) + PUT (0x20) + 6 list codes (0x44/0x45/0x46/0x24/0x25/0x26) supported; only atomics + stall-and-notify remain deferred |
| `rust/rpcs3-spu-differential/src/lib.rs` | 56-65 | "ACEITO PARCIAL — wiring into Interpreter/Recompiler requires Phase C" — Phase C closed | Mark Phase C closed; full family wired via `with_mfc_tag_stat_queue` + R8.4d/e callbacks |
| `rust/rpcs3-spu-thread/src/lib.rs` | 273-281, 332-334, 910-915 | Multiple comments say "runtime-mode MFC out of scope", "wrch ch21 is a no-op in runtime mode", "future R7+ will route" | Update: R7/R8.1/R8.4d/R8.4e callbacks installed; wrch ch21 dispatches GET/PUT/GETL/PUTL when callbacks registered |
| `rpcs3/Emu/Cell/SPUTraceJsonl.h` | 184-187 | `record_spu_mfc_cmd` scope says "GET only" / "MUST NOT call for PUT/list/atomic" — PUT lifted in R8.1 | Update: simple GET (0x40) + PUT (0x20). List codes go through `record_spu_mfc_getl_cmd` |

**Why P0:** Anyone implementing R8.5b reading these comments will get
wrong mental model of what's already done. The user-visible error
message at `trace_fmt.rs:809` would be the most embarrassing if surfaced
during R8.5b debugging.

### P1 — Misleads future sessions (high drift risk)

#### P1.a — Operational (single concrete drift)

| Item | Action |
|---|---|
| `rust/.claude/vetius-context.local.json` is git-tracked despite gitignore | `git rm --cached rust/.claude/vetius-context.local.json` (file stays on disk, future writes stop dirtying tree). **Do NOT touch the file content itself.** Verify gitignore covers `**/.claude/` (already does). |

#### P1.b — Code comments (MEDIUM severity, Agent 3)

| Source | Line(s) | Drift |
|---|---|---|
| `rust/rpcs3-spu-differential/src/trace_fmt.rs` | 295-298 | "R8.4c will lift the parser canary" — already lifted; past tense needed |
| `rust/rpcs3-spu-differential/src/trace_fmt.rs` | 656-665 | `UnsupportedMfcListCmd` variant doc says "R8.4b/c/d will progressively implement" — all 6 landed |
| `rust/rpcs3-spu-thread/src/lib.rs` | 325-330 | R6.7 C.2 banner says "captured GET trace" — also handles full family now |
| `rust/rpcs3-spu-thread/src/lib.rs` | 375-380 | "A future R8.4+ feature" — R8.4 is HEAD; should say "future R8.5+" |
| `rust/rpcs3-spu-ffi/src/lib.rs` | 1-21 | Crate intro frames everything as "R6.0 → R6.1+ will use" — R6/R7/R8 happened |
| `rust/rpcs3-spu-ffi/src/tests.rs` | 1121, 1142-1145 | Test uses GETL as rejection canary — GETL has dedicated callback now |
| `behavior-freeze/harness/check_patch_separation.py` | 69-113 | Comment block frames R8.4e as HEAD; HEAD is R8.4f-b. SHA pin is correct (R8.4f-a/b reused), just comment incomplete |

#### P1.c — Fixture source READMEs (Agent 4)

| Source | Drift | Fix |
|---|---|---|
| `behavior-freeze/fixtures/spu/sources/single_spu_dma_get_v1/README.md` | Status header says "A.5 BLOCKED on real-binary capture" + "replay test is `#[ignore]`d" — landed 2026-05-03, is 7th oracle | Update status header to "REPLAY-VALIDATED 2026-05-03" |
| `behavior-freeze/fixtures/spu/sources/single_spu_mailbox_multi_v1/README.md` | Opens with "R6.4b-pre: sources authored; .self not yet built" — replay-validated since R6.4b-replay (2026-04-29) | Update status to reflect replay-validated state |

#### P1.d — Trace notes (Agent 4)

| File | Drift |
|---|---|
| `single_spu_dma_put_v1.notes.md:60` | Unresolved authoring placeholder: `<bumped from R7.2 to R8.1 final — see check_patch_separation.py>` |
| `single_spu_dma_getlb_v1.notes.md:115-121` | "R8.4f-b deferred" section is now obsolete (R8.4f-b landed 17th/18th oracles) |

### P2 — Cleanup nice-to-have (defer to post-R8.5b)

| Item | Note |
|---|---|
| `rust/README.md` titled "Phase 0 scaffolding" but content is wave-6+ | Defers to PROJECT_STATUS.md so non-blocking |
| Older source READMEs reference legacy `ps3-build` Debian container with `PS3DEV=/opt/ps3dev` vs current `rpcs3-ps3dev-toolchain:local` + `PS3DEV=/usr/local/ps3dev` | Each fixture was built with the toolchain of its era — not strictly drift |
| R5.x archival comments in `mfc_replay.rs`, `dma_chunk.rs`, `jit.rs`, `interpreter/lib.rs`, `decoder/lib.rs` | Historical labels, accurate, leave-as-archival |
| `behavior-freeze/harness/lib/frame_hash.py:8` legitimate TODO | Still pending; not drift |

---

## What's GREEN (no action needed)

- `docs/PROJECT_STATUS.md` (working tree) — Agent 2 verified all § 9 claims
- `docs/SPU_DMA_MFC_R6_7_DESIGN.md` (working tree) — § 19 closure marker + § 20 all verified
- `behavior-freeze/fixtures/spu/traces/README.md` (working tree) — item 5 all 8 cmd codes verified
- 16 of 18 `.notes.md` (mailbox, branch_loop, signal, loadstore, mailbox_multi, game_like, dma_get, dma_get_multi, dma_get_any, dma_tag_poll, dma_tag_immediate, dma_getl, dma_putl, dma_getlf, dma_putlb, dma_putlf)
- `behavior-freeze/docs/*.md` (INVENTORY, DECISIONS, DEFERRED, BACKLOG_RESIDUAL, HOMEBREW_PLAN, SPU_RECOMPILER_PLAN, AUTONOMOUS_LOG) — all carry explicit point-in-time disclaimers + cross-ref live PROJECT_STATUS
- `docs/history/PROJECT_STATUS_R5_ARCHIVE.md` — clear archive marker, properly back-referenced
- `historico/pre-r4b-2026-04-25/*` — clear "🧊 FROZEN BASELINE" banners, only `.md` + `.md.bak` (no large binaries)
- `docs/patches/*.patch` — all 3 SHAs match pinned values exactly
- `docs/R6_LIVE_BRIDGE_PLAN.md`, `SPU_RUST_BRIDGE_PATCH.md`, `SPU_TRACE_*.md` — phase-specific design artifacts, not claiming live status
- `bin/config/config.yml` — out of repo (lives in rpcs3-upstream-clean/), but verified there: SPU/PPU Decoder restored to Recompiler (LLVM)
- `aqtinstall.log` — orphan in parent dir, outside repo

---

## Recommended fix sequence

### Stage 1 — P0 only (engine-logic comment fixes)

5 edits in source files. No behavior change, comments only:

1. `rust/rpcs3-spu-differential/src/trace_fmt.rs:809` — rewrite error message
2. `rust/rpcs3-spu-differential/src/mfc_replay.rs:1-39` — rewrite module banner
3. `rust/rpcs3-spu-differential/src/lib.rs:56-65` — close Phase C banner
4. `rust/rpcs3-spu-thread/src/lib.rs:273-281, 332-334, 910-915` — update runtime-mode comments
5. `rpcs3/Emu/Cell/SPUTraceJsonl.h:184-187` — update scope comment to mention PUT

**Side effect:** patch SHA bumps. Per `check_patch_separation.py`:
- `SPUTraceJsonl.h` is in the **scaffolding** patch → scaffolding SHA bumps
- `mfc_replay.rs`, `trace_fmt.rs`, `lib.rs` are Rust-side → no patch bump (patches only pin C++ files)
- `SPUThread.cpp` not touched → no hooks SHA bump
- `SPURustBridge.cpp` not touched → no bridge SHA bump

Need to verify by running `check_patch_separation.py` after the C++ edit and updating the pinned hash.

### Stage 2 — P1 (operational + comments + notes)

7-8 edits:

1. `git rm --cached rust/.claude/vetius-context.local.json` (untrack only — file stays)
2. 6 medium-severity code comment updates (P1.b)
3. 2 source README updates (P1.c — dma_get_v1, mailbox_multi_v1)
4. 2 trace notes updates (P1.d — dma_put_v1 placeholder, dma_getlb_v1 stale defer marker)
5. Update comment block in `check_patch_separation.py:69-113`

### Stage 3 — P2 (defer until post-R8.5b)

- `rust/README.md` "Phase 0 scaffolding" reframe
- Optional toolchain reference normalization in older source READMEs
- Historical R5.x comment labels — leave as archival

### Stage 4 — Commit

Single consolidated commit covering:
- Original 3 docs refresh (PROJECT_STATUS § 9, DESIGN § 19+§ 20, traces/README item 5)
- This audit report file (DOCS_AUDIT_2026_05_22.md)
- All P0 + P1 fixes applied
- `git rm --cached` for vetius-context.local.json

Commit message must:
- Reference R8.4f-b as baseline (HEAD when audit started)
- Bump scaffolding SHA pin if `SPUTraceJsonl.h` was modified
- Note that gates remain green
- Per CLAUDE.md global: include `Co-Authored-By` trailer

---

## Open questions for user before applying

1. **`git rm --cached` for the tracked vetius-context.local.json**: confirm OK? File contents are local cache, untracking has no effect on local config but stops dirtying `git status`.

2. **Stage 1 patch SHA bump**: `SPUTraceJsonl.h` is part of the scaffolding patch. Editing the doc comment will bump the scaffolding SHA. We'll need to update `check_patch_separation.py` pin AND regenerate `docs/patches/spu_trace_jsonl_scaffolding.patch`. **Acceptable to bump patches for comment-only changes**, or prefer to leave that C++ file untouched and only edit Rust-side comments?

3. **Stage 1 vs Stage 1+2 in one go**: apply only P0 (5 edits) and stop for review, OR apply P0 + P1 together (15-16 edits total)?

4. **Source README rewrites**: for `single_spu_dma_get_v1/README.md` which currently describes the pre-landing "BLOCKED" state — should I preserve that as a historical section + add a current "REPLAY-VALIDATED" header on top, or replace entirely? (Preserving history is safer, replace is cleaner.)

5. **Master plan in memory**: should I write a memory topic file `project_docs_audit_2026_05_22.md` pointing to this audit file so future sessions can recover the plan? Recommended yes.

---

## Recovery path if context lost

If the conversation gets compacted or the session restarts before fixes
are applied:

1. Read this file (`behavior-freeze/docs/DOCS_AUDIT_2026_05_22.md`)
2. Read memory `project_docs_audit_2026_05_22.md` (if step 5 above
   was approved)
3. Resume from Stage 1 by applying the 5 P0 edits listed above
4. Run `python behavior-freeze/harness/check_patch_separation.py` to
   verify any SHA bumps after C++ edits
5. Run `python behavior-freeze/harness/check_trace_fixtures.py` (should
   stay green throughout — no fixture changes)

---

## Skip list (explicitly not fixing)

- `aqtinstall.log` (outside repo)
- `bin/config/config.yml` (outside rpcs3-master/, already verified clean
  in rpcs3-upstream-clean/)
- Memory files (already compacted earlier this session)
- R5.x archival code comments (low severity, leave-as-archival per
  global rule)
- `rust/.claude/vetius-context.local.json` content (state cache — do
  not edit, only untrack)
