# PC <-> Pico protocol

PMB3 describes raw cable operations. The host supplies every SPI word; the Pico
clocks those words unchanged. The PC and GBA applications handle BIOS commands,
ROM data, and application checksums.

The crate is `no_std`, has no dependencies, and uses fixed buffers. `Frame`
holds raw command and payload bytes. `Command` identifies the Pico operation;
`Request` and `Response` provide typed arguments and results. `Words` borrows
validated little-endian word data without requiring aligned memory.

## Frames

| Field | Bytes |
| --- | --- |
| ASCII `PMB3` | 4 |
| Command | 1 |
| Payload length, little endian | 2 |
| Payload | 0-1025 |
| CRC-32/ISO-HDLC, little endian | 4 |

CRC covers command, length, and payload. It excludes the magic and CRC fields.
It matches Python's `zlib.crc32` and uses a 1 KiB byte lookup table.
The largest frame is 1036 bytes.

Reply commands set bit 7: an Exchange request uses `0x03`, its reply `0x83`.
Every reply payload starts with a status byte: zero for success, otherwise an
error code. Requests have no status byte. All multibyte fields are little endian.
SPI bit and byte order are handled separately by the firmware.

## Commands

| Command | Value | Request payload | Successful reply after status |
| --- | --- | --- | --- |
| Exchange | 3 | One u32 | One received u32 |
| BulkRead | 5 | Count (u16), fill word (u32) | Count received u32 words |
| BulkWrite | 6 | 1-256 u32 words | Bytes clocked (u32) |
| BulkExchange | 7 | 1-256 u32 words | Same number of received u32 words |
| Info | 8 | Empty | ASCII `PICO-MB3`, maximum bulk word count (u32) |

Unused command values remain reserved. Info never clocks SPI.
Its successful payload is 13 bytes: status
(1), identity (8), and bulk word limit (4). The limit is currently 256 words
(1024 bytes) and must be within 1-256. The identity includes protocol version 3.
The host caches the limit when connecting. Info reports no GBA connection or
readiness state; those checks use host-controlled exchanges.

All exchanges use the 256 kHz SPI clock and at least 36 us after each word.
There is no timing argument or extra application pacing in the cable protocol.

BulkRead transmits the fill word for every received word. Its request is always
six bytes, regardless of read size. BulkWrite discards received words and
acknowledges the number of data bytes clocked. That acknowledgement does not
confirm that the GBA accepted the data. Use
BulkExchange or subsequent reads to obtain application replies.

The Pico does not interpret the words. A BIOS handshake word, encrypted ROM
word, or dumper command is just data. A response computed by the GBA from an
incoming word arrives in a subsequent SPI transfer.

## Errors

| Status | ErrorCode |
| --- | --- |
| 1 | InvalidRequest |
| 2 | TransferFailed |
| 3 | Disconnected |
| 4 | InvalidFrame |

Error replies contain only the status byte. Unused status values remain
reserved. `Response::Error` represents cable failures; `Error` represents local
codec failures such as a bad CRC or wrong reply length. BIOS and application
errors are interpreted by the host application from received SPI words.

## Session rules

- Feed each received byte to `Decoder::push`; USB packets may split or combine
  frames. Consume the borrowed frame before feeding another byte.
- The decoder skips noise before magic, rejects oversized lengths at the header,
  and checks CRC after receiving the declared frame. A truncated frame needs a
  transport timeout and decoder reset.
- Keep one request outstanding. There are no sequence numbers to distinguish
  delayed replies to identical requests.
- Never replay an uncertain transfer. The GBA may already have consumed it.
  A failed bulk transfer returns an error, not partial data or a partial count.
- Dropping DTR or disconnecting cancels the current operation and discards
  partial framing. It does not reset the GBA.

## Compatibility

Build the PC tools and Pico firmware from the same revision. PMB1 and PMB2 use
different frame magic and cannot communicate with PMB3. Earlier development
builds also used PMB3 with different command numbers and payloads, so the INFO
identity alone cannot distinguish every incompatible revision.

The GBA dumper's application protocol is separate, currently version 2.
`mb-uploader` performs BIOS multiboot on the PC using raw BulkExchange calls.
