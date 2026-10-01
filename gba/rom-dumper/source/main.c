#include <gba_console.h>
#include <gba_dma.h>
#include <gba_interrupt.h>
#include <gba_sio.h>
#include <gba_systemcalls.h>
#include <stdio.h>

#include "dumper.h"

static Dumper dumper;
static uint32_t save_buffer[SAVE_MAX_SIZE / 4] EWRAM_BSS;

static DUMPER_CODE uint32_t read_cartridge_word(uint32_t address) {
    if (address >= SAVE_START) return save_buffer[(address - SAVE_START) / 4];
    return *(volatile const uint32_t *)(uintptr_t)address;
}

static uint8_t read_save_byte(uint32_t address) {
    return *(volatile const uint8_t *)(uintptr_t)address;
}

static void write_save_byte(uint32_t address, uint8_t value) {
    *(volatile uint8_t *)(uintptr_t)address = value;
}

static void eeprom_dma(const void *source, void *destination, unsigned count) {
    __asm__ volatile ("" ::: "memory");
    DMA3COPY(source, destination, DMA16 | count);
    /* DMA starts two cycles after enabling it; then stalls the CPU. */
    __asm__ volatile ("nop\n\tnop" ::: "memory");
    while (REG_DMA3CNT & DMA_ENABLE) {}
    __asm__ volatile ("" ::: "memory");
}

static void read_eeprom(unsigned block, unsigned address_bits, uint8_t *output) {
    uint16_t request[17], reply[68];
    unsigned count = save_eeprom_request(block, address_bits, request);
    void *eeprom = (void *)0x0dffff00;
    /* Both addresses increment. No other code uses DMA3 during a snapshot. */
    eeprom_dma(request, eeprom, count);
    eeprom_dma(eeprom, reply, 68);
    save_eeprom_decode(reply, output);
}

static void prepare_save(void) {
    uint16_t enabled = REG_IME;
    REG_IME = 0;
    uint32_t request = dumper.save_request;
    uint32_t generation = dumper.generation;
    dumper.save_request = 0;
    REG_IME = enabled;
    if (!request) return;

    SaveType type = (SaveType)(request - 1);
    if (type == SAVE_AUTO) {
        volatile const uint8_t *rom = (volatile const uint8_t *)DUMPER_ROM_START;
        type = rom[0xb2] == 0x96
            ? save_detect(rom, DUMPER_ROM_END - DUMPER_ROM_START) : SAVE_UNKNOWN;
    }
    uint32_t result = type == SAVE_EEPROM_UNKNOWN ? DUMPER_SAVE_EEPROM_SIZE : DUMPER_SAVE_UNKNOWN;
    if (save_size(type)) {
        /* SRAM/Flash: 8 clocks. EEPROM: WS2 at 8/8 clocks. */
        volatile uint16_t *waitcnt = (volatile uint16_t *)0x04000204;
        uint16_t previous = *waitcnt;
        *waitcnt = (previous & ~0x0703) | 0x0303;
        const SaveBus bus = {read_save_byte, write_save_byte, read_eeprom};
        result = save_capture(type, &bus, (uint8_t *)save_buffer);
        *waitcnt = previous;
    }
    enabled = REG_IME;
    REG_IME = 0;
    __asm__ volatile ("" ::: "memory");
    dumper_save_complete(&dumper, generation, result);
    REG_IME = enabled;
}

static DUMPER_CODE void serial_irq(void) {
    uint32_t command = REG_SIODATA32;
    uint32_t reply = dumper_handle(&dumper, command, read_cartridge_word);
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
        case PREPARING: status = "Reading cartridge save"; break;
        case SENDING:
            status = sent == total ? "Waiting for PC verification" :
                start == SAVE_START ? "Sending save" : "Sending ROM";
            break;
        case COMPLETE:
            status = start == SAVE_START ? "Save verified and saved on PC" : "PC verified and saved ROM";
            break;
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
        if (dumper.save_request) {
            show_status();
            prepare_save();
        }
        /* About five screen updates/sec; serial IRQs continue during printing. */
        if (++frames == 12) {
            frames = 0;
            show_status();
        }
    }
}
