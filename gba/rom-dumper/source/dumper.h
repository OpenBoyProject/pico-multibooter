#ifndef ROM_DUMPER_H
#define ROM_DUMPER_H

#include <stdint.h>

#if defined(__arm__)
#define DUMPER_CODE __attribute__((section(".iwram"), long_call))
#else
#define DUMPER_CODE
#endif

/* Application words carried by PMB3 EXCHANGE and bulk transfers. */
#define DUMPER_HELLO UINT32_C(0x52444d50)
#define DUMPER_ID UINT32_C(0x52444d02)
#define DUMPER_CHECKSUM UINT32_C(0x43524300)
#define DUMPER_BEGIN UINT32_C(0x4245474e)
#define DUMPER_READ UINT32_C(0x52420000)
#define DUMPER_DONE UINT32_C(0x444f4e45)
#define DUMPER_CANCEL UINT32_C(0x43414e43)
#define DUMPER_ACK UINT32_C(0x4f4b0002)
#define DUMPER_BAD_COMMAND UINT32_C(0xbad00001)
#define DUMPER_BAD_ADDRESS UINT32_C(0xbad00002)
#define DUMPER_BAD_STATE UINT32_C(0xbad00003)
#define DUMPER_ROM_START UINT32_C(0x08000000)
#define DUMPER_ROM_END UINT32_C(0x0a000000)
#define DUMPER_MAX_BLOCK_WORDS 253u

typedef enum { READY, SENDING, COMPLETE, CANCELLED, FAILED } DumperPhase;
typedef enum { COMMAND, BEGIN_ADDRESS, BEGIN_SIZE, STREAM } DumperState;

typedef struct {
    uint32_t crc;
    uint32_t start;
    uint32_t next;
    uint32_t total;
    uint32_t sent;
    uint32_t remaining;
    uint32_t block_bytes;
    DumperState state;
    DumperPhase phase;
} Dumper;

typedef uint32_t (*DumperReadWord)(uint32_t address);

DUMPER_CODE void dumper_init(Dumper *dumper);
/* The caller queues the result for the NEXT full-duplex transfer. */
DUMPER_CODE uint32_t dumper_handle(Dumper *dumper, uint32_t command, DumperReadWord read_word);

#endif
