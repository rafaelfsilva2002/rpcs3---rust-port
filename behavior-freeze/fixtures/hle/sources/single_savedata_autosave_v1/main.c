// single_savedata_autosave_v1 — cellSaveData AutoSave2/AutoLoad2 round-trip.
// CC0 1.0 (public domain). See LICENSE.md.
//
// The hardest HLE family: callback-driven. The game calls sysSaveAutoSave2 with
// a status callback + a file callback; the SYSTEM (emu-core bridge) invokes them
// in sequence via EmuCore::call_guest_function, marshalling the sysSave* structs
// in/out of a guest scratch page, and performs the file I/O against the VFS.
//
// This homebrew saves an 8-byte payload to SLOTAUTO00/DATA.BIN, wipes its local
// buffer, loads it back, and compares:
//   AutoSave2 != 0   -> 0xBAD0
//   AutoLoad2 != 0   -> 0xBAD1
//   bytes differ     -> 0xBAD3
//   round-trip match -> 0xC0DE
//
// Pre-wire (permissive no-op) the callbacks never fire -> load reads nothing ->
// 0xBAD3. Behaviour: rsx-free, SPU-free, pure cellSaveData HLE + guest callbacks.

#include <ppu-types.h>
#include <sysutil/save.h>
#include <sys/process.h>
#include <string.h>

SYS_PROCESS_PARAM(1001, 0x10000);

#define PAYLOAD_LEN 8

static unsigned char g_filebuf[64];
static unsigned char g_savebuf[4096]; // work buffer for sysSaveBufferSettings
static const unsigned char PAYLOAD[PAYLOAD_LEN] = {
    0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88,
};

// Status callback: accept, continue to the file callback.
static void status_cb(sysSaveCallbackResult *result, sysSaveStatusIn *in,
                      sysSaveStatusOut *out)
{
    (void)in;
    (void)out;
    result->result = SYS_SAVE_CALLBACK_RESULT_CONTINUE; // 0
}

// File callback (SAVE). The protocol: a callback returning CONTINUE (OK_NEXT)
// has its file op PERFORMED, then is called again; a callback returning DONE
// (OK_LAST) breaks the loop WITHOUT performing an op. So one WRITE needs two
// calls: set up the WRITE + CONTINUE on the first, then DONE on the second.
static int g_write_calls = 0;
static void file_write_cb(sysSaveCallbackResult *result, sysSaveFileIn *in,
                          sysSaveFileOut *out)
{
    (void)in;
    if (g_write_calls == 0) {
        memcpy(g_filebuf, PAYLOAD, PAYLOAD_LEN);
        out->fileOperation = SYS_SAVE_FILE_OPERATION_WRITE; // 1
        out->fileType = SYS_SAVE_FILETYPE_STANDARD_FILE;    // 1
        out->filename = "DATA.BIN";
        out->offset = 0;
        out->size = PAYLOAD_LEN;
        out->bufferSize = sizeof(g_filebuf);
        out->buffer = g_filebuf;
        result->result = SYS_SAVE_CALLBACK_RESULT_CONTINUE; // 0 (perform + loop)
    } else {
        result->result = SYS_SAVE_CALLBACK_RESULT_DONE; // 1 (stop)
    }
    g_write_calls++;
}

// File callback (LOAD): one READ into g_filebuf (CONTINUE), then DONE.
static int g_read_calls = 0;
static void file_read_cb(sysSaveCallbackResult *result, sysSaveFileIn *in,
                         sysSaveFileOut *out)
{
    (void)in;
    if (g_read_calls == 0) {
        out->fileOperation = SYS_SAVE_FILE_OPERATION_READ; // 0
        out->fileType = SYS_SAVE_FILETYPE_STANDARD_FILE;
        out->filename = "DATA.BIN";
        out->offset = 0;
        out->size = PAYLOAD_LEN;
        out->bufferSize = sizeof(g_filebuf);
        out->buffer = g_filebuf;
        result->result = SYS_SAVE_CALLBACK_RESULT_CONTINUE; // 0 (perform + loop)
    } else {
        result->result = SYS_SAVE_CALLBACK_RESULT_DONE; // 1 (stop)
    }
    g_read_calls++;
}

int main(void)
{
    sysSaveBufferSettings buf;
    memset(&buf, 0, sizeof(buf));
    buf.maxDirectories = 1;
    buf.maxFiles = 8;
    buf.bufferSize = sizeof(g_savebuf);
    buf.buffer = g_savebuf;

    s32 r = sysSaveAutoSave2(SYS_SAVE_CURRENT_VERSION, "SLOTAUTO00",
                             SYS_SAVE_ERROR_DIALOG_NONE, &buf, status_cb,
                             file_write_cb, 0, 0);
    if (r != 0) {
        return 0xBAD0;
    }

    memset(g_filebuf, 0, sizeof(g_filebuf)); // wipe before load-back

    r = sysSaveAutoLoad2(SYS_SAVE_CURRENT_VERSION, "SLOTAUTO00",
                         SYS_SAVE_ERROR_DIALOG_NONE, &buf, status_cb,
                         file_read_cb, 0, 0);
    if (r != 0) {
        return 0xBAD1;
    }

    for (int i = 0; i < PAYLOAD_LEN; i++) {
        if (g_filebuf[i] != PAYLOAD[i]) {
            return 0xBAD3;
        }
    }
    return 0xC0DE;
}
