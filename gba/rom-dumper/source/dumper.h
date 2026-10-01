#ifndef ROM_DUMPER_H
#define ROM_DUMPER_H

#include <stdint.h>
#include "save.h"

#if defined(__arm__)
#define DUMPER_CODE __attribute__((section(".iwram"), long_call))
#else
#define DUMPER_CODE
#endif

/* Application words carried by PMB3 EXCHANGE and bulk transfers. */
#define DUMPER_HELLO UINT32_C(0x52444d50)
#define DUMPER_ID UINT32_C(0x52444d03)
#define DUMPER_CHECKSUM UINT32_C(0x43524300)
#define DUMPER_BEGIN UINT32_C(0x4245474e)
#define DUMPER_READ UINT32_C(0x52420000)
#define DUMPER_DONE UINT32_C(0x444f4e45)
#define DUMPER_CANCEL UINT32_C(0x43414e43)
#define DUMPER_ACK UINT32_C(0x4f4b0003)
#define DUMPER_SAVE UINT32_C(0x53410000)
#define DUMPER_SAVE_STATUS UINT32_C(0x53544154)
#define DUMPER_SAVE_UNKNOWN UINT32_C(0xbad00004)
#define DUMPER_SAVE_EEPROM_SIZE UINT32_C(0xbad00005)
#define DUMPER_BAD_COMMAND UINT32_C(0xbad00001)
#define DUMPER_BAD_ADDRESS UINT32_C(0xbad00002)
#define DUMPER_BAD_STATE UINT32_C(0xbad00003)
#define DUMPER_ROM_START UINT32_C(0x08000000)
#define DUMPER_ROM_END UINT32_C(0x0a000000)
#define DUMPER_MAX_BLOCK_WORDS 253u

typedef enum { READY, PREPARING, SENDING, COMPLETE, CANCELLED, FAILED } DumperPhase;
typedef enum { COMMAND, BEGIN_ADDRESS, BEGIN_SIZE, STREAM } DumperState;

typedef struct {
    uint32_t crc;
    uint32_t start;
    uint32_t next;
    uint32_t total;
    uint32_t sent;
    uint32_t remaining;
    uint32_t block_bytes;
    volatile uint32_t save_request;
    volatile uint32_t save_result;
    volatile uint32_t generation;
    DumperState state;
    DumperPhase phase;
} Dumper;

typedef uint32_t (*DumperReadWord)(uint32_t address);

DUMPER_CODE void dumper_init(Dumper *dumper);
/* The caller queues the result for the NEXT full-duplex transfer. */
DUMPER_CODE uint32_t dumper_handle(Dumper *dumper, uint32_t command, DumperReadWord read_word);
/* Called with IRQs masked after the foreground save snapshot finishes. */
void dumper_save_complete(Dumper *dumper, uint32_t generation, uint32_t result);

#endif
