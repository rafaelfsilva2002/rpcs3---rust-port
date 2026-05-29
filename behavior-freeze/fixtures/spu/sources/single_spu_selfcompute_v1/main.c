// single_spu_selfcompute_v1 — PPU side
// CC0 1.0 (public domain). See LICENSE.md.
//
// Drives a single SELF-CONTAINED SPU thread that needs NO input:
//   1. init SPU, import image, create group, init thread, start group.
//   2. The SPU computes sum(1..=1000) = 500500 = 0x7A314, writes it to
//      OUT_MBOX, and halts (stop 0x101).
//   3. PPU joins; the lv2 kernel reads OUT_MBOX as the group-exit status.
//   4. PPU returns 0xC0DE iff status == 0x7A314 (else the raw status, so a
//      mismatch is visible in the exit code for diagnostics).
//
// There is NO sysSpuThreadWriteMb (no IN_MBOX) and NO DMA, so this boots
// cleanly through EmuCore::run_self's synchronous single-SPU path — unlike the
// mailbox/signal/DMA oracles, which stall or need live DMA. Purpose: validate
// the SPU JIT backend (RecompilerExecutor) end-to-end vs the interpreter.

#include <ppu-types.h>
#include <ppu-asm.h>
#include <sys/spu.h>
#include <sys/process.h>
#include <stdio.h>
#include <string.h>

extern const u8  spu_selfcompute_bin[];
extern const u32 spu_selfcompute_bin_size;

SYS_PROCESS_PARAM(1001, 0x10000);

#define EXPECTED_STATUS 0x0007A314u /* sum(1..=1000) = 500500 */

int main(s32 argc, const char *argv[])
{
    (void)argc;
    (void)argv;

    s32 ret;

    ret = sysSpuInitialize(1, 0);
    if (ret) { printf("[selfcompute_v1] sysSpuInitialize: 0x%08x\n", ret); return 1; }

    sysSpuImage spu_image;
    ret = sysSpuImageImport(&spu_image, spu_selfcompute_bin, 0);
    if (ret) { printf("[selfcompute_v1] sysSpuImageImport: 0x%08x\n", ret); return 1; }

    sysSpuThreadGroupAttribute group_attr;
    sysSpuThreadGroupAttributeInitialize(group_attr);
    sysSpuThreadGroupAttributeName(group_attr, "selfcompute_v1");
    sysSpuThreadGroupAttributeType(group_attr, SPU_THREAD_GROUP_TYPE_NORMAL);

    sys_spu_group_t group_id;
    ret = sysSpuThreadGroupCreate(&group_id, 1, 100, &group_attr);
    if (ret) { printf("[selfcompute_v1] sysSpuThreadGroupCreate: 0x%08x\n", ret); return 1; }

    sysSpuThreadAttribute thread_attr;
    sysSpuThreadAttributeInitialize(thread_attr);
    sysSpuThreadAttributeName(thread_attr, "spu_0");
    sysSpuThreadAttributeOption(thread_attr, SPU_THREAD_ATTR_NONE);

    sysSpuThreadArgument thread_args;
    sysSpuThreadArgumentInitialize(thread_args);

    sys_spu_thread_t spu_thread_id;
    ret = sysSpuThreadInitialize(&spu_thread_id, group_id, 0, &spu_image,
                                 &thread_attr, &thread_args);
    if (ret) { printf("[selfcompute_v1] sysSpuThreadInitialize: 0x%08x\n", ret); return 1; }

    ret = sysSpuThreadGroupStart(group_id);
    if (ret) { printf("[selfcompute_v1] sysSpuThreadGroupStart: 0x%08x\n", ret); return 1; }

    // No IN_MBOX push: the SPU needs no input. Just wait for it to finish.
    u32 cause, status;
    ret = sysSpuThreadGroupJoin(group_id, &cause, &status);
    if (ret) { printf("[selfcompute_v1] sysSpuThreadGroupJoin: 0x%08x\n", ret); return 1; }

    // NOTE: no success-path printf. Under EmuCore::run_self the newlib
    // printf path reaches an unimplemented import (sysPrxForUser 0xe0da8efd)
    // that breaks the clean exit, so we keep the success path printf-free and
    // signal the result purely via the process exit code (0xC0DE / status).
    (void)cause;

    sysSpuThreadGroupDestroy(group_id);
    sysSpuImageClose(&spu_image);

    return (status == EXPECTED_STATUS) ? 0xC0DE : (int)status;
}
