// single_event_queue_v1 — PPU-only sys_event_queue + port round-trip
// CC0 1.0 (public domain). See LICENSE.md.
//
// Minimal end-to-end exercise of the LV2 sys_event_queue_* +
// sys_event_port_* syscalls. Targets R10.6 (EventRegistry impl)
// via the syscall arms wired by this same slice.
//
// Behaviour (single-thread, non-blocking):
//   create queue → create port → connect port→queue →
//   port_send(0xAA, 0xBB, 0xCC) → receive (event already queued,
//   returns immediately) → verify payload → disconnect →
//   destroy port → destroy queue → return 0xC0DE.
//
// `sysEventQueueReceive` is normally blocking, but because we
// `sysEventPortSend` BEFORE receiving, the queue already holds the
// event and receive returns it without parking — safe on a single
// PPU.
//
// No SPU, no DMA, no printf. Status via process exit code (r3).
//
// LV2 syscalls touched:
//   #128 sys_event_queue_create
//   #129 sys_event_queue_destroy
//   #130 sys_event_queue_receive
//   #134 sys_event_port_create
//   #135 sys_event_port_destroy
//   #136 sys_event_port_connect_local
//   #137 sys_event_port_disconnect
//   #138 sys_event_port_send

#include <ppu-types.h>
#include <sys/event_queue.h>
#include <sys/process.h>

SYS_PROCESS_PARAM(1001, 0x10000);

int main(s32 argc, const char *argv[])
{
    (void)argc;
    (void)argv;

    s32 ret;

    sys_event_queue_t queue;
    sys_event_queue_attr_t qattr;
    qattr.attr_protocol = SYS_EVENT_QUEUE_FIFO;
    qattr.type = SYS_EVENT_QUEUE_PPU;
    for (int i = 0; i < 8; i++) qattr.name[i] = 0;

    ret = sysEventQueueCreate(&queue, &qattr, SYS_EVENT_QUEUE_KEY_LOCAL, 8);
    if (ret) {
        return 1;
    }

    sys_event_port_t port;
    ret = sysEventPortCreate(&port, SYS_EVENT_PORT_LOCAL, SYS_EVENT_PORT_NO_NAME);
    if (ret) {
        return 2;
    }

    ret = sysEventPortConnectLocal(port, queue);
    if (ret) {
        return 3;
    }

    // Send before receive so the queue is non-empty (no parking).
    ret = sysEventPortSend(port, 0xAA, 0xBB, 0xCC);
    if (ret) {
        return 4;
    }

    sys_event_t event;
    event.source = 0;
    event.data_1 = 0;
    event.data_2 = 0;
    event.data_3 = 0;
    ret = sysEventQueueReceive(queue, &event, 0);
    if (ret) {
        return 5;
    }

    // Verify the payload survived the round-trip. source is the
    // sending port id (kernel convention); we only assert on the
    // three data fields we control.
    if (event.data_1 != 0xAA) {
        return 6;
    }
    if (event.data_2 != 0xBB) {
        return 7;
    }
    if (event.data_3 != 0xCC) {
        return 8;
    }

    ret = sysEventPortDisconnect(port);
    if (ret) {
        return 9;
    }

    ret = sysEventPortDestroy(port);
    if (ret) {
        return 10;
    }

    ret = sysEventQueueDestroy(queue, 0);
    if (ret) {
        return 11;
    }

    return 0xC0DE;
}
