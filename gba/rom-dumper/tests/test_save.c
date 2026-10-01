#include <assert.h>
#include <stdio.h>
#include <string.h>

#include "save.h"

static unsigned bank, writes, reads, eeprom_reads, bits_expected;
static uint8_t output[SAVE_MAX_SIZE];

static uint8_t pattern(unsigned offset) {
    return (uint8_t)((offset * 17) ^ (offset >> 8) ^ (offset >> 16));
}

static uint8_t read_byte(uint32_t address) {
    assert(address >= SAVE_START && address < SAVE_START + 65536);
    ++reads;
    return pattern(bank * 65536 + address - SAVE_START);
}

static void write_byte(uint32_t address, uint8_t value) {
    /* The only permitted writes select a Flash bank. No ID/program/erase. */
    const uint32_t addresses[] = {SAVE_START + 0x5555, SAVE_START + 0x2aaa,
                                  SAVE_START + 0x5555, SAVE_START};
    const uint8_t values[] = {0xaa, 0x55, 0xb0};
    unsigned step = writes % 4;
    assert(address == addresses[step]);
    if (step < 3) assert(value == values[step]);
    else {
        const unsigned banks[] = {0, 1, 0};
        assert(writes / 4 < 3 && value == banks[writes / 4]);
        bank = value;
    }
    ++writes;
}

static void read_eeprom(unsigned block, unsigned address_bits, uint8_t *bytes) {
    assert(address_bits == bits_expected);
    assert(block == eeprom_reads++);
    for (unsigned i = 0; i < 8; ++i) bytes[i] = pattern(block * 8 + i);
}

int main(void) {
    const struct { const char *tag; SaveType type; } tags[] = {
        {"SRAM_V110", SAVE_SRAM}, {"SRAM_F_V103", SAVE_SRAM},
        {"FLASH_V123", SAVE_FLASH64}, {"FLASH512_V130", SAVE_FLASH64},
        {"FLASH1M_V103", SAVE_FLASH128}, {"EEPROM_V124", SAVE_EEPROM_UNKNOWN}
    };
    for (unsigned i = 0; i < sizeof(tags) / sizeof(tags[0]); ++i) {
        for (unsigned offset = 0; offset < 4; ++offset) {
            uint8_t rom[64] = {0};
            memcpy(rom + offset, tags[i].tag, strlen(tags[i].tag));
            assert(save_detect(rom, sizeof(rom)) == tags[i].type);
            assert(save_detect(rom, offset + strlen(tags[i].tag) - 1) == SAVE_UNKNOWN);
        }
    }
    assert(save_detect((const uint8_t *)"FLASH1M_Vxxx", 12) == SAVE_UNKNOWN);
    assert(save_detect(NULL, 0) == SAVE_UNKNOWN);
    const SaveBus bus = {read_byte, write_byte, read_eeprom};
    for (SaveType type = SAVE_SRAM; type <= SAVE_EEPROM8K; ++type) {
        bank = writes = reads = eeprom_reads = 0;
        bits_expected = type == SAVE_EEPROM512 ? 6 : 14;
        uint32_t size = save_capture(type, &bus, output);
        assert(size == save_size(type));
        for (unsigned i = 0; i < size; ++i) assert(output[i] == pattern(i));
        assert(writes == (type == SAVE_FLASH128 ? 12u : 0));
        assert(bank == 0);
        if (type == SAVE_EEPROM512 || type == SAVE_EEPROM8K) {
            assert(eeprom_reads == size / 8 && reads == 0);
        } else assert(reads == size && eeprom_reads == 0);
    }
    writes = reads = eeprom_reads = 0;
    assert(save_capture(SAVE_UNKNOWN, &bus, output) == 0);
    assert(writes == 0 && reads == 0 && eeprom_reads == 0);

    /* Check every EEPROM address, including the unused high address bits. */
    for (unsigned width = 6; width <= 14; width += 8) {
        unsigned blocks = width == 6 ? 64 : 1024;
        for (unsigned block = 0; block < blocks; ++block) {
            uint16_t request[17];
            unsigned count = save_eeprom_request(block, width, request);
            assert(count == width + 3 && request[0] == 1 && request[1] == 1);
            assert(request[count - 1] == 0);
            unsigned decoded = 0;
            for (unsigned bit = 2; bit < count - 1; ++bit)
                decoded = (decoded << 1) | request[bit];
            assert(decoded == block);
        }
        uint16_t request[17];
        assert(save_eeprom_request(blocks, width, request) == 0);
        assert(save_eeprom_request(0, 7, request) == 0);
    }
    uint16_t reply[68];
    const uint8_t expected[] = {0x80, 0x01, 0xff, 0x00, 0xaa, 0x55, 0x12, 0x34};
    for (unsigned i = 0; i < 68; ++i) reply[i] = 0xfffe;
    for (unsigned byte = 0; byte < 8; ++byte)
        for (unsigned bit = 0; bit < 8; ++bit)
            reply[4 + byte * 8 + bit] |= (expected[byte] >> (7 - bit)) & 1;
    save_eeprom_decode(reply, output);
    assert(memcmp(output, expected, 8) == 0);
    puts("save: signatures, SRAM/Flash banks, EEPROM reads, and byte order passed");
    return 0;
}
