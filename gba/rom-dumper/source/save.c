#include "save.h"

static int signature(volatile const uint8_t *rom, size_t left, const char *tag) {
    size_t i = 0;
    while (tag[i]) {
        if (i >= left || rom[i] != (uint8_t)tag[i]) return 0;
        ++i;
    }
    /* Nintendo library signatures end in a three-digit version. */
    for (unsigned digit = 0; digit < 3; ++digit, ++i) {
        if (i >= left || rom[i] < '0' || rom[i] > '9') return 0;
    }
    return 1;
}

SaveType save_detect(volatile const uint8_t *rom, size_t size) {
    for (size_t i = 0; i < size; ++i) {
        volatile const uint8_t *p = rom + i;
        size_t left = size - i;
        switch (*p) {
            case 'S':
                if (signature(p, left, "SRAM_V") || signature(p, left, "SRAM_F_V"))
                    return SAVE_SRAM;
                break;
            case 'F':
                if (signature(p, left, "FLASH1M_V")) return SAVE_FLASH128;
                if (signature(p, left, "FLASH512_V") || signature(p, left, "FLASH_V"))
                    return SAVE_FLASH64;
                break;
            case 'E':
                if (signature(p, left, "EEPROM_V")) return SAVE_EEPROM_UNKNOWN;
                break;
        }
    }
    return SAVE_UNKNOWN;
}

uint32_t save_size(SaveType type) {
    switch (type) {
        case SAVE_SRAM: return 32768;
        case SAVE_FLASH64: return 65536;
        case SAVE_FLASH128: return 131072;
        case SAVE_EEPROM512: return 512;
        case SAVE_EEPROM8K: return 8192;
        default: return 0;
    }
}

static void flash_bank(const SaveBus *bus, uint8_t bank) {
    bus->write_byte(SAVE_START + 0x5555, 0xaa);
    bus->write_byte(SAVE_START + 0x2aaa, 0x55);
    bus->write_byte(SAVE_START + 0x5555, 0xb0);
    bus->write_byte(SAVE_START, bank);
}

uint32_t save_capture(SaveType type, const SaveBus *bus, uint8_t *output) {
    uint32_t size = save_size(type);
    if (type == SAVE_EEPROM512 || type == SAVE_EEPROM8K) {
        for (unsigned i = 0; i < size; i += 8)
            bus->read_eeprom(i / 8, type == SAVE_EEPROM512 ? 6 : 14, output + i);
    } else {
        for (uint32_t i = 0; i < size; ++i) {
            if (type == SAVE_FLASH128 && (i & 0xffff) == 0)
                flash_bank(bus, i >> 16);
            output[i] = bus->read_byte(SAVE_START + (i & 0xffff));
        }
        if (type == SAVE_FLASH128) flash_bank(bus, 0);
    }
    return size;
}

unsigned save_eeprom_request(unsigned block, unsigned address_bits, uint16_t *bits) {
    if ((address_bits != 6 && address_bits != 14) ||
        block >= (address_bits == 6 ? 64u : 1024u)) return 0;
    bits[0] = 1;
    bits[1] = 1; /* Read only: a write request would start with 10. */
    for (unsigned i = 0; i < address_bits; ++i)
        bits[2 + i] = (block >> (address_bits - i - 1)) & 1;
    bits[2 + address_bits] = 0;
    return address_bits + 3;
}

void save_eeprom_decode(const uint16_t *bits, uint8_t *output) {
    for (unsigned byte = 0; byte < 8; ++byte) {
        uint8_t value = 0;
        for (unsigned bit = 0; bit < 8; ++bit)
            value = (value << 1) | (bits[4 + byte * 8 + bit] & 1);
        output[byte] = value;
    }
}
