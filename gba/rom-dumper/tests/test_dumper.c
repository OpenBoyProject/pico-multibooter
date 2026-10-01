#include <assert.h>
#include <stdio.h>

#include "dumper.h"

static unsigned reads;

static uint32_t read_word(uint32_t address) {
    ++reads;
    switch (address) {
        case 0x08000000: return 0x12345678;
        case 0x08000004: return 0x89abcdef;
        case 0x09fffffc: return 0xffffffff;
        default: assert(!"unexpected cartridge read"); return 0;
    }
}

static uint32_t sequential_read(uint32_t address) {
    assert(address >= DUMPER_ROM_START && address < DUMPER_ROM_END && !(address & 3));
    return address ^ 0x89abcdef;
}

static uint32_t reference_crc_word(uint32_t crc, uint32_t word) {
    crc ^= word;
    for (unsigned bit = 0; bit < 32; ++bit) {
        crc = (crc >> 1) ^ ((crc & 1) ? UINT32_C(0xedb88320) : 0);
    }
    return crc;
}

static uint32_t sample;

static uint32_t sample_read(uint32_t address) {
    assert(address == DUMPER_ROM_START);
    return sample;
}

static void check_crc_bytes(void) {
    for (unsigned byte = 0; byte < 256; ++byte) {
        Dumper dumper;
        dumper_init(&dumper);
        sample = byte * UINT32_C(0x01010101);
        uint32_t expected = reference_crc_word(UINT32_MAX, DUMPER_ROM_START);
        expected = reference_crc_word(expected, sample);
        assert(dumper_handle(&dumper, DUMPER_ROM_START, sample_read) == sample);
        assert(dumper_handle(&dumper, DUMPER_CHECKSUM, sample_read) == ~expected);
    }
}

static uint32_t read_save(uint32_t address) {
    assert(address >= SAVE_START && address < SAVE_START + SAVE_MAX_SIZE);
    return address ^ UINT32_C(0x87654321);
}

static void check_save(void) {
    Dumper dumper;
    dumper_init(&dumper);
    /* Save ranges are inaccessible until a snapshot completes. */
    dumper_handle(&dumper, DUMPER_BEGIN, read_save);
    assert(dumper_handle(&dumper, SAVE_START, read_save) == DUMPER_BAD_ADDRESS);
    assert(dumper_handle(&dumper, DUMPER_SAVE | SAVE_FLASH128, read_save) == DUMPER_ACK);
    assert(dumper.phase == PREPARING && dumper.save_request == SAVE_FLASH128 + 1);
    assert(dumper_handle(&dumper, DUMPER_SAVE_STATUS, read_save) == 0);
    dumper_save_complete(&dumper, dumper.generation, SAVE_MAX_SIZE);
    assert(dumper_handle(&dumper, DUMPER_SAVE_STATUS, read_save) == SAVE_MAX_SIZE);
    assert(dumper.phase == READY);
    dumper_handle(&dumper, DUMPER_BEGIN, read_save);
    uint32_t start = SAVE_START + 65532;
    assert(dumper_handle(&dumper, start, read_save) == start);
    assert(dumper_handle(&dumper, 8, read_save) == 8);
    assert(dumper_handle(&dumper, DUMPER_READ | 2, read_save) == (DUMPER_READ | 2));
    uint32_t crc = UINT32_MAX;
    for (unsigned i = 0; i < 2; ++i) {
        uint32_t address = start + i * 4;
        uint32_t value = read_save(address);
        assert(dumper_handle(&dumper, 0, read_save) == value);
        crc = reference_crc_word(reference_crc_word(crc, address), value);
    }
    assert(dumper_handle(&dumper, 0, read_save) == ~crc);
    assert(dumper_handle(&dumper, DUMPER_DONE, read_save) == DUMPER_ACK);

    dumper_handle(&dumper, DUMPER_BEGIN, read_save);
    assert(dumper_handle(&dumper, SAVE_START + SAVE_MAX_SIZE - 4, read_save) == SAVE_START + SAVE_MAX_SIZE - 4);
    assert(dumper_handle(&dumper, 8, read_save) == DUMPER_BAD_ADDRESS);

    /* A cancelled or replaced snapshot must not become available later. */
    const uint32_t recovery[] = {DUMPER_HELLO, DUMPER_CANCEL, DUMPER_SAVE | SAVE_SRAM};
    for (unsigned i = 0; i < 3; ++i) {
        dumper_handle(&dumper, DUMPER_SAVE, read_save);
        uint32_t generation = dumper.generation;
        dumper_handle(&dumper, recovery[i], read_save);
        dumper_save_complete(&dumper, generation, SAVE_MAX_SIZE);
        assert(dumper_handle(&dumper, DUMPER_SAVE_STATUS, read_save) == 0);
    }
    dumper_handle(&dumper, DUMPER_SAVE, read_save);
    dumper_save_complete(&dumper, dumper.generation, DUMPER_SAVE_EEPROM_SIZE);
    assert(dumper.phase == FAILED);
    assert(dumper_handle(&dumper, DUMPER_SAVE_STATUS, read_save) == DUMPER_SAVE_EEPROM_SIZE);
    dumper_handle(&dumper, DUMPER_BEGIN, read_save);
    assert(dumper_handle(&dumper, SAVE_START, read_save) == DUMPER_BAD_ADDRESS);
    assert(dumper_handle(&dumper, DUMPER_SAVE | 6, read_save) == DUMPER_BAD_COMMAND);
}

int main(void) {
    check_crc_bytes();
    check_save();
    Dumper dumper;
    dumper_init(&dumper);
    assert(dumper_handle(&dumper, DUMPER_CHECKSUM, read_word) == 0);
    assert(dumper_handle(&dumper, 0, read_word) == DUMPER_ID);
    assert(dumper_handle(&dumper, DUMPER_HELLO, read_word) == DUMPER_ID);

    /* Same pipeline as the hardware: RX is the reply queued BEFORE this TX. */
    const uint32_t tx[] = {
        DUMPER_HELLO, 0x08000000, 0x08000004, 0x09fffffc, DUMPER_CHECKSUM, 0
    };
    const uint32_t rx[] = {
        DUMPER_ID, DUMPER_ID, 0x12345678, 0x89abcdef, 0xffffffff, 0xe4412f24
    };
    uint32_t pending = DUMPER_ID;
    for (unsigned i = 0; i < sizeof(tx) / sizeof(tx[0]); ++i) {
        assert(pending == rx[i]);
        pending = dumper_handle(&dumper, tx[i], read_word);
    }
    assert(pending == DUMPER_ID);
    assert(reads == 3);
    /* Golden CRC from Python zlib over LE (address, value) pairs. */
    assert(dumper_handle(&dumper, DUMPER_CHECKSUM, read_word) == 0xe4412f24);

    const uint32_t unaligned[] = {0x08000001, 0x08000002, 0x08000003, 0x09ffffff};
    for (unsigned i = 0; i < sizeof(unaligned) / sizeof(unaligned[0]); ++i) {
        assert(dumper_handle(&dumper, unaligned[i], read_word) == DUMPER_BAD_ADDRESS);
    }
    const uint32_t invalid[] = {1, 0x07fffffc, 0x0a000000, 0x0e000000, 0xffffffff};
    for (unsigned i = 0; i < sizeof(invalid) / sizeof(invalid[0]); ++i) {
        assert(dumper_handle(&dumper, invalid[i], read_word) == DUMPER_BAD_COMMAND);
    }
    assert(reads == 3); /* Invalid commands must never touch memory. */
    assert(dumper_handle(&dumper, DUMPER_CHECKSUM, read_word) == 0xe4412f24);
    assert(dumper_handle(&dumper, DUMPER_HELLO, read_word) == DUMPER_ID);
    assert(dumper_handle(&dumper, DUMPER_CHECKSUM, read_word) == 0);

    /* Declare a range once, then clock sequential data with zero words. */
    assert(dumper_handle(&dumper, DUMPER_BEGIN, read_word) == DUMPER_ACK);
    assert(dumper_handle(&dumper, 0x08000000, read_word) == 0x08000000);
    assert(dumper_handle(&dumper, 8, read_word) == 8);
    assert(dumper.phase == SENDING && dumper.total == 8 && dumper.sent == 0);
    assert(dumper_handle(&dumper, DUMPER_READ | 2, read_word) == (DUMPER_READ | 2));
    assert(dumper_handle(&dumper, 0, read_word) == 0x12345678);
    assert(dumper_handle(&dumper, 0, read_word) == 0x89abcdef);
    assert(dumper.sent == 0); /* Last prepared data word has not been clocked out. */
    uint32_t bulk_crc = dumper_handle(&dumper, 0, read_word);
    assert(bulk_crc == ~dumper.crc);
    assert(dumper.sent == 8 && dumper.phase == SENDING);
    assert(dumper_handle(&dumper, 0, read_word) == DUMPER_ID);
    assert(dumper_handle(&dumper, DUMPER_DONE, read_word) == DUMPER_ACK);
    assert(dumper.phase == COMPLETE);
    assert(dumper_handle(&dumper, DUMPER_READ | 1, read_word) == DUMPER_BAD_STATE);

    /* Cross-frame state and bounds: two blocks consume exactly the declared range. */
    assert(dumper_handle(&dumper, DUMPER_BEGIN, read_word) == DUMPER_ACK);
    assert(dumper_handle(&dumper, 0x08000000, read_word) == 0x08000000);
    assert(dumper_handle(&dumper, 8, read_word) == 8);
    for (unsigned i = 0; i < 2; ++i) {
        assert(dumper_handle(&dumper, DUMPER_READ | 1, read_word) == (DUMPER_READ | 1));
        assert(dumper_handle(&dumper, 0, read_word) == (i ? 0x89abcdef : 0x12345678));
        dumper_handle(&dumper, 0, read_word); /* CRC of this block. */
        assert(dumper.sent == (i + 1) * 4);
        assert(dumper_handle(&dumper, 0, read_word) == DUMPER_ID);
    }
    assert(dumper_handle(&dumper, DUMPER_READ | 1, read_word) == DUMPER_BAD_STATE);

    unsigned before = reads;
    const uint32_t invalid_counts[] = {0, 254, 65535};
    for (unsigned i = 0; i < 3; ++i) {
        dumper_handle(&dumper, DUMPER_BEGIN, read_word);
        dumper_handle(&dumper, 0x09fffffc, read_word);
        dumper_handle(&dumper, 4, read_word);
        assert(dumper_handle(&dumper, DUMPER_READ | invalid_counts[i], read_word) == DUMPER_BAD_STATE);
    }
    dumper_handle(&dumper, DUMPER_BEGIN, read_word);
    dumper_handle(&dumper, 0x09fffffc, read_word);
    assert(dumper_handle(&dumper, 8, read_word) == DUMPER_BAD_ADDRESS);
    assert(reads == before);

    /* Recovery interrupts every parser state without extra cartridge reads. */
    for (unsigned state = COMMAND; state <= STREAM; ++state) {
        dumper.state = (DumperState)state;
        assert(dumper_handle(&dumper, DUMPER_CANCEL, read_word) == DUMPER_ACK);
        assert(dumper.state == COMMAND && dumper.phase == CANCELLED);
        dumper.state = (DumperState)state;
        assert(dumper_handle(&dumper, DUMPER_HELLO, read_word) == DUMPER_ID);
        assert(dumper.state == COMMAND && dumper.phase == READY);
    }
    dumper_handle(&dumper, DUMPER_BEGIN, read_word);
    dumper_handle(&dumper, 0x08000000, read_word);
    dumper_handle(&dumper, 8, read_word);
    assert(dumper_handle(&dumper, DUMPER_DONE, read_word) == DUMPER_BAD_STATE);
    assert(reads == before);

    /* Largest block at the very end of the ROM window, split across arbitrary
       host frames: handling one word never depends on a USB packet boundary. */
    uint32_t base = DUMPER_ROM_END - 4 * DUMPER_MAX_BLOCK_WORDS;
    dumper_handle(&dumper, DUMPER_BEGIN, sequential_read);
    dumper_handle(&dumper, base, sequential_read);
    dumper_handle(&dumper, 4 * DUMPER_MAX_BLOCK_WORDS, sequential_read);
    assert(dumper_handle(&dumper, DUMPER_READ | DUMPER_MAX_BLOCK_WORDS, sequential_read) ==
           (DUMPER_READ | DUMPER_MAX_BLOCK_WORDS));
    uint32_t expected_crc = UINT32_MAX;
    for (unsigned i = 0; i < DUMPER_MAX_BLOCK_WORDS; ++i) {
        uint32_t address = base + 4 * i;
        uint32_t value = address ^ 0x89abcdef;
        assert(dumper_handle(&dumper, 0, sequential_read) == value);
        expected_crc = reference_crc_word(reference_crc_word(expected_crc, address), value);
    }
    assert(dumper_handle(&dumper, 0, sequential_read) == ~expected_crc);
    assert(dumper.next == DUMPER_ROM_END);
    assert(dumper.sent == dumper.total);
    assert(dumper.phase == SENDING);
    assert(dumper_handle(&dumper, DUMPER_DONE, sequential_read) == DUMPER_ACK);
    assert(dumper.phase == COMPLETE);

    puts("rom-dumper: individual/bulk reads, CRC, completion, recovery, and bounds passed");
    return 0;
}
