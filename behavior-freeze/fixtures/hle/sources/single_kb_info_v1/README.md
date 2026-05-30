# single_kb_info_v1 (HLE backlog — cellKb GetInfo, deterministic headless)

`ioKbInit(127) -> ioKbGetInfo`. Headless emu-core has no host keyboard handler →
connected=0, max=127 (CELL_KB_MAX_KEYBOARDS). NIDs (sys_io) captured at runtime.
Arm: cellKbInit -> cell_kb_init; cellKbGetInfo -> cell_kb_get_info(&kb,
&NullKbBackend), BE-serializes CellKbInfo {max@0, now@4, info@8, status[127]@12}.
Exit 0xC0DE iff max==127 && connected==0. Consumed by hle_kb_info.rs.
CC0 1.0 — see LICENSE.md.
