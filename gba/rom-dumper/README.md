# GBA cartridge ROM dumper

A devkitPro/libgba multiboot program that runs from GBA RAM and reads the
inserted cartridge. Its version 2 protocol supports a declared dump range,
sequential bulk reads, and a status screen. The Pico forwards words between
the PC and GBA; this program handles the cartridge commands.

## Build and upload

Run commands from the repository root. See [environment setup](../../README.md#environment-setup) for prerequisites.

```sh
cargo build-gba rom-dumper
cargo upload --release -- gba/rom-dumper/build/rom-dumper_mb.gba
```

Insert the cartridge before powering on the GBA. Hold START+SELECT during the
boot logo to enter multiboot reception. After upload, wait for **GBA ROM Dumper
ready...** on screen. This means the program is running, not that the cartridge
has been detected. It then displays the requested byte count, address, sent
bytes, percentage, and transfer status. It shows **PC verified and saved ROM**
only after the host has verified every block, synchronized the output file,
and sent DONE. Sent bytes alone do not imply a verified file.

The build needs devkitARM, libgba, `gbafix`, Make, and Python 3. Other
installations can set `DEVKITPRO` and `DEVKITARM`. `gba_mb.specs` supplies the RAM
startup/linker script; `gbafix` supplies the cartridge header. The build pads to
16 bytes and validates the logo, header checksum, entry branches, and size.
ELF and map files accompany the `.gba`. `cargo build-gba` invokes the Makefile;
it does not upload the ROM. Use `--port` with `cargo upload` if more than one
cable is connected. Finish an active dump before uploading or flashing.

## Save a dump

```sh
# Example for a known 8 MiB cartridge. --size is the exact byte count.
cargo run --release -p mb-dumper -- \
    --port /dev/ttyACM0 --size 0x800000 cartridge.gba
```

Choose your cartridge's actual size: `0x400000` = 4 MiB, `0x800000` = 8 MiB,
`0x1000000` = 16 MiB, `0x2000000` = 32 MiB. `0xC0` reads just the header.
The count must be nonzero, divisible by four, and at most 32 MiB. The ROM header
has no size field. Reading past the physical ROM may return mirrors or open-bus
data. A matching CRC verifies transmission, not cartridge presence or size.

`mb-dumper` keeps one PC connection open, declares the whole range, then
reads up to 253 words per batch. It verifies each block before writing its
little-endian ROM bytes. It refuses to overwrite files and never retries an
uncertain exchange. Errors/Ctrl+C can leave an incomplete file. Application
errors send CANCEL if the connection is still usable. A dropped connection
cannot notify the GBA; its screen may remain at the last sent byte count.
Reopen and begin a fresh dump to reset application state.

## Transfer speed

All cable exchanges use a nominal 256 kHz SPI clock and a minimum 36 us gap
after each word. Use release builds for both the Pico firmware and PC dumper.

`mb-dumper` reads the bulk capacity from INFO without clocking the GBA. Each
block uses one BULK_EXCHANGE containing READ and all its zero clocks. The reply
contains a stale word, the echoed command, ROM words, and CRC. The complete
block takes one USB request and reply. No uncertain transfer is replayed.
Terminal progress updates are limited to five per second.

Ignoring USB, scheduler, and block overhead, each 32-bit transfer needs about
125 us of clock time plus the 36 us gap. That gives an approximate ceiling of
24 KiB/s and an 8 MiB wire-time floor of 5.6 minutes. Actual times will be
longer; mb-dumper reports measured KiB/s. Bulk commands provide range validation
and progress without repeating addresses, but still need one SPI word per four
ROM bytes. They do not eliminate that wire limit.

The serial interrupt, command handler, and cartridge read routine run from
IWRAM, together with a 1 KiB CRC lookup table. The byte-table CRC uses four
lookups per 32-bit word, applied to both the address and ROM value. The main
loop renders text about five times a second with serial interrupts enabled; it briefly
masks interrupts only to snapshot four status fields. No rendering, allocation,
or BIOS calls happen in the serial handler. Native tests do not measure
interrupt latency or cable timing. Higher clock rates are not enabled.

## Manual exchanges

Each reply arrives in the **next** transfer. Discard the first returned word.
A command may be split across USB exchanges; the GBA retains its parser state.

```sh
# Identify/reset: second RX must be 0x52444d02.
cargo cable -- --port /dev/ttyACM0 bulk-exchange 0x52444d50 0

# Individual reads still work: RX = stale, ID, ROM[0], ROM[4], CRC.
cargo cable -- --port /dev/ttyACM0 bulk-exchange 0x52444d50 0x08000000 0x08000004 0x43524300 0

# Declare an eight-byte dump: RX = stale, ID, ACK, address, byte count.
cargo cable -- --port /dev/ttyACM0 bulk-exchange 0x52444d50 0x4245474e 0x08000000 8 0
# Read two words: RX = stale, 0x52420002, ROM[0], ROM[4], CRC.
cargo cable -- --port /dev/ttyACM0 bulk-exchange 0x52420002 0 0 0 0
# After verifying/saving: RX = stale, ACK. GBA shows complete.
cargo cable -- --port /dev/ttyACM0 bulk-exchange 0x444f4e45 0
```

`bulk-read N` makes the Pico transmit N zeros by default. In a bulk stream,
these clock data and then its CRC; when idle, they return the identity. Convert data words to
little-endian bytes when saving; all u32 values are legal ROM data, including
values equal to status/magic words. Interpret replies by position.

## Application protocol v2

These are numeric u32 application words. SPI sends them MSB-first; PMB3 encodes
them as little-endian u32s in its USB payload. Identity is `0x52444D02`, ACK is
`0x4F4B0002`. There are no cartridge writes.

| Command | Parameters | Reply on subsequent clock(s) |
| --- | --- | --- |
| `0x52444D50` HELLO | None | ID; resets parser, range, progress, and CRC |
| `0x4245474E` BEGIN | Absolute ROM address, byte count | ACK, echoed address, echoed byte count |
| `0x52420000 + count` READ | Then `count + 2` zero clocks | Echoed READ/count, `count` ROM words, CRC |
| `0x444F4E45` DONE | None | ACK only if the declared range was sent; marks complete |
| `0x43414E43` CANCEL | None | ACK; aborts parser/stream and marks cancelled |
| Zero (idle) | None | ID; CRC unchanged |
| Aligned `0x08000000..0x09FFFFFC` | None | Individual ROM word; updates CRC |
| `0x43524300` CHECKSUM (idle) | None | Current CRC; CRC unchanged |

BEGIN validates a nonempty, word-aligned range inside the first 32 MiB ROM
window. READ requires 1..253 words that fit inside the declared remaining range.
READ resets the block CRC, then each zero queues one word and increments the
address. The next zero queues the CRC and commits the block's sent count; one
last zero receives that CRC. Thus a batch has `count + 3` outgoing words and
returns `[stale, echoed_READ, data..., CRC]`. BEGIN and DONE each need a final
zero to receive their last response. Other commands are not valid inside a
stream. HELLO and CANCEL interrupt/recover any parser state.

Invalid commands return `0xBAD00001`; invalid addresses/sizes return
`0xBAD00002`; invalid block counts, state, or premature DONE return
`0xBAD00003`. Errors leave the stream, mark failure, and never read invalid
addresses. Restart with HELLO/BEGIN. Individual reads remain useful for debugging
but should not be mixed into a bulk dump.

CRC-32/ISO-HDLC uses polynomial `0xEDB88320`, initial state `0xFFFFFFFF`, final
inversion. Append LE `(address, value)` pairs for each read. This detects a
wrong address as well as corrupt data. A bulk READ starts a new CRC; individual
reads accumulate since HELLO. The pairs `(0x08000000, 0x12345678)`,
`(0x08000004, 0x89ABCDEF)`, `(0x09FFFFFC, 0xFFFFFFFF)` yield `0xE4412F24`.

This program reads cartridge ROM only. It does not access save memory, write
cartridges, or switch flashcart banks.

## Checks

```sh
make -C gba/rom-dumper check
cargo test --locked -p mb-dumper
cargo test --locked -p mb-host -p mb-cable -p mb-uploader -p mb-dumper -p protocol
cargo test --locked -p firmware --lib
python3 gba/rom-dumper/tools/check_rom.py gba/rom-dumper/build/rom-dumper_mb.gba
```

Tests cover individual and sequential reads, CRCs, range/count checks, session
recovery, completion, host byte order, and corrupted responses. Layout checks
verify the built multiboot image. Use a hardware dump to check timing and
cartridge data after changing the transfer path.

References: [devkitPro GBA examples](https://github.com/devkitPro/gba-examples),
[libgba SIO definitions](https://github.com/devkitPro/libgba/blob/master/include/gba_sio.h),
and [GBATEK serial documentation](https://mgba-emu.github.io/gbatek/#sio-normal-mode).
