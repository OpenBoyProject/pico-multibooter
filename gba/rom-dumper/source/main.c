#include <gba_console.h>
#include <gba_interrupt.h>
#include <gba_sio.h>
#include <gba_systemcalls.h>
#include <stdio.h>

#include "dumper.h"

static Dumper dumper;

static DUMPER_CODE uint32_t read_rom_word(uint32_t address) {
    return *(volatile const uint32_t *)(uintptr_t)address;
}

static DUMPER_CODE void serial_irq(void) {
    uint32_t command = REG_SIODATA32;
    uint32_t reply = dumper_handle(&dumper, command, read_rom_word);
    REG_SIODATA32 = reply;
    REG_SIOCNT = SIO_32BIT | SIO_SO_HIGH | SIO_IRQ | SIO_START;
}

static void show_status(void) {
    /* Snapshot only four fields with IRQs masked. Never print or divide while
       masked: the serial handler must keep up independently of rendering. */
    uint16_t enabled = REG_IME;
    REG_IME = 0;
    __asm__ volatile ("" ::: "memory");
    uint32_t total = dumper.total;
    uint32_t sent = dumper.sent;
    uint32_t start = dumper.start;
    DumperPhase phase = dumper.phase;
    __asm__ volatile ("" ::: "memory");
    REG_IME = enabled;

    const char *status = "Waiting for PC";
    switch (phase) {
        case READY: break;
        case SENDING: status = sent == total ? "Waiting for PC verification" : "Sending ROM"; break;
        case COMPLETE: status = "PC verified and saved ROM"; break;
        case CANCELLED: status = "Cancelled"; break;
        case FAILED: status = "Protocol error - restart dump"; break;
    }
    iprintf("\x1b[4;0H%-29s", status);
    iprintf("\x1b[6;0HRequested: %8lu bytes", (unsigned long)total);
    iprintf("\x1b[7;0HFrom:      0x%08lx", (unsigned long)start);
    iprintf("\x1b[9;0HSent:      %8lu bytes", (unsigned long)sent);
    /* total <= 32 MiB, so sent * 100 fits a uint32_t. */
    iprintf("\x1b[10;0HProgress:  %3lu%%", (unsigned long)(total ? sent * 100 / total : 0));
}

int main(void) {
    REG_IME = 0;
    consoleDemoInit();
    iprintf("\x1b[2JGBA ROM Dumper ready...");
    dumper_init(&dumper);
    show_status();

    irqInit();
    irqSet(IRQ_SERIAL, serial_irq);
    REG_RCNT = R_NORMAL;
    REG_SIOCNT = SIO_32BIT | SIO_SO_HIGH | SIO_IRQ;
    REG_SIODATA32 = DUMPER_ID;
    irqEnable(IRQ_SERIAL | IRQ_VBLANK);
    REG_SIOCNT = SIO_32BIT | SIO_SO_HIGH | SIO_IRQ | SIO_START;

    unsigned frames = 0;
    for (;;) {
        VBlankIntrWait();
        /* About five screen updates/sec; serial IRQs continue during printing. */
        if (++frames == 12) {
            frames = 0;
            show_status();
        }
    }
}
