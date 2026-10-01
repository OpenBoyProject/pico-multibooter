#ifndef ROM_DUMPER_SAVE_H
#define ROM_DUMPER_SAVE_H

#include <stddef.h>
#include <stdint.h>

#define SAVE_START UINT32_C(0x0e000000)
#define SAVE_MAX_SIZE UINT32_C(0x20000)

typedef enum {
    SAVE_AUTO, SAVE_SRAM, SAVE_FLASH64, SAVE_FLASH128, SAVE_EEPROM512,
    SAVE_EEPROM8K, SAVE_EEPROM_UNKNOWN, SAVE_UNKNOWN
} SaveType;

typedef struct {
    uint8_t (*read_byte)(uint32_t address);
    void (*write_byte)(uint32_t address, uint8_t value);
    void (*read_eeprom)(unsigned block, unsigned address_bits, uint8_t *output);
} SaveBus;

SaveType save_detect(volatile const uint8_t *rom, size_t size);
uint32_t save_size(SaveType type);
uint32_t save_capture(SaveType type, const SaveBus *bus, uint8_t *output);
unsigned save_eeprom_request(unsigned block, unsigned address_bits, uint16_t *bits);
void save_eeprom_decode(const uint16_t *bits, uint8_t *output);

#endif
