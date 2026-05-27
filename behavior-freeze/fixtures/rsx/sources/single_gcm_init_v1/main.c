// single_gcm_init_v1 — R13 probe: full rsxInit (gcm device setup) path.
// CC0 1.0. Drives the real PSL1GHT rsxInit so EmuCore surfaces the
// sys_rsx_* lv2 syscalls that the cellGcm HLE (R13) must implement.
#include <ppu-types.h>
#include <rsx/rsx.h>
#include <sys/process.h>
SYS_PROCESS_PARAM(1001, 0x10000);
#define CB_SIZE   (0x10000)
#define HOST_SIZE (1*1024*1024)
static u8 host_buffer[HOST_SIZE] __attribute__((aligned(1024*1024)));
int main(void){
    gcmContextData *ctx;
    rsxInit(&ctx, CB_SIZE, HOST_SIZE, host_buffer);
    return 0xC0DE;
}
