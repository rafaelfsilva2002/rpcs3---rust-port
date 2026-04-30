// single_spu_mailbox_v1 — PPU side
// CC0 1.0 (public domain). See LICENSE.md.
//
// Loads a single SPU thread, pushes ONE command via IN_MBOX, joins the
// SPU group, exits. No DMA. No PPU spin-loop on OUT_MBOX (avoids the
// PSL1GHT cooperative-thread problem-state MMIO complication).
//
// Behaviour:
//   1. PPU pushes 0x100 to SPU IN_MBOX via sysSpuThreadWriteMb.
//   2. SPU reads, computes 0x129, writes to OUT_MBOX, halts with 0xD5.
//   3. PPU joins the SPU thread group; OUT_MBOX value sits unread until
//      the trace captures it; PPU doesn't drain.

#include <ppu-types.h>
#include <ppu-asm.h>
#include <sys/spu.h>
#include <sys/process.h>
#include <stdio.h>
#include <string.h>

extern const u8  spu_mailbox_bin[];
extern const u32 spu_mailbox_bin_size;

SYS_PROCESS_PARAM(1001, 0x10000);

int main(s32 argc, const char *argv[])
{
    (void)argc;
    (void)argv;

    s32 ret;

    // Initialize SPU subsystem: 1 cooperative SPU, 0 raw SPUs.
    ret = sysSpuInitialize(1, 0);
    if (ret) {
        printf("[smbox_v1] sysSpuInitialize: 0x%08x\n", ret);
        return 1;
    }

    // Import the embedded SPU image (full ELF blob).
    sysSpuImage spu_image;
    ret = sysSpuImageImport(&spu_image, spu_mailbox_bin, 0);
    if (ret) {
        printf("[smbox_v1] sysSpuImageImport: 0x%08x\n", ret);
        return 1;
    }

    // Create thread group of size 1.
    sysSpuThreadGroupAttribute group_attr;
    sysSpuThreadGroupAttributeInitialize(group_attr);
    sysSpuThreadGroupAttributeName(group_attr, "smbox_v1");
    sysSpuThreadGroupAttributeType(group_attr, SPU_THREAD_GROUP_TYPE_NORMAL);

    sys_spu_group_t group_id;
    ret = sysSpuThreadGroupCreate(&group_id, 1, 100, &group_attr);
    if (ret) {
        printf("[smbox_v1] sysSpuThreadGroupCreate: 0x%08x\n", ret);
        return 1;
    }

    // Initialize the single SPU thread.
    sysSpuThreadAttribute thread_attr;
    sysSpuThreadAttributeInitialize(thread_attr);
    sysSpuThreadAttributeName(thread_attr, "spu_0");
    sysSpuThreadAttributeOption(thread_attr, SPU_THREAD_ATTR_NONE);

    sysSpuThreadArgument thread_args;
    sysSpuThreadArgumentInitialize(thread_args);

    sys_spu_thread_t spu_thread_id;
    ret = sysSpuThreadInitialize(&spu_thread_id, group_id, 0, &spu_image,
                                 &thread_attr, &thread_args);
    if (ret) {
        printf("[smbox_v1] sysSpuThreadInitialize: 0x%08x\n", ret);
        return 1;
    }

    // Start the group.
    ret = sysSpuThreadGroupStart(group_id);
    if (ret) {
        printf("[smbox_v1] sysSpuThreadGroupStart: 0x%08x\n", ret);
        return 1;
    }

    // Push exactly ONE command. SPU will read, compute, write reply,
    // halt with 0xD5. We don't drain OUT_MBOX from the PPU side —
    // simpler than fighting PSL1GHT's cooperative-thread MMIO mapping.
    sysSpuThreadWriteMb(spu_thread_id, 0x100u);

    // Wait for the group to terminate. SPU's stop 0xD5 ends the group.
    u32 cause, status;
    ret = sysSpuThreadGroupJoin(group_id, &cause, &status);
    if (ret) {
        printf("[smbox_v1] sysSpuThreadGroupJoin: 0x%08x\n", ret);
        return 1;
    }

    printf("[smbox_v1] OK cause=0x%x status=0x%x\n",
           (unsigned)cause, (unsigned)status);

    sysSpuThreadGroupDestroy(group_id);
    sysSpuImageClose(&spu_image);

    return 0;
}
