# single_mouse_info_v1 (HLE backlog — cellMouse GetInfo, deterministic headless)

`ioMouseInit(127) -> ioMouseGetInfo`. Headless → connected=0, max=127
(CELL_MAX_MICE). NIDs (sys_io) captured at runtime. Arm: cellMouseInit ->
cell_mouse_init; cellMouseGetInfo -> cell_mouse_get_info(&mouse,
&NullMouseBackend), BE-serializes CellMouseInfo {max@0, now@4, info@8,
vendor_id[127]@12, product_id[127]@266, status[127]@520}. Exit 0xC0DE iff
max==127 && connected==0. Consumed by hle_mouse_info.rs. CC0 1.0 — see LICENSE.md.
