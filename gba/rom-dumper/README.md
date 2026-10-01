# GBA cartridge ROM and save dumper

A devkitPro/libgba multiboot program that runs from GBA RAM and reads the
inserted cartridge's ROM or save memory. Its version 3 protocol supports save
detection, a declared dump range, sequential bulk reads, and a status screen.
The Pico forwards words between
the PC and GBA; this program handles the cartridge commands.

## Build and upload

Run commands from the repository root. See [environment setup](../../README.md#environment-setup) for prerequisites.

```sh
cargo build-gba rom-dumper
cargo upload --release -- gba/rom-dumper/build/rom-dumper_mb.gba
```

With a release archive, use `./mb-uploader rom-dumper_mb.gba` instead. Both
ROM and save dumps use this same GBA image; no second upload is needed between
dumps. Use the v3 image with the matching PC tool.

Insert the cartridge before powering on the GBA. Hold START+SELECT during the
boot logo to enter multiboot reception. After upload, wait for **GBA ROM Dumper
ready...** on screen. This means the program is running, not that the cartridge
has been detected. It then displays the requested byte count, address, sent
bytes, percentage, and transfer status. It shows **PC verified and saved ROM**
or **Save verified and saved on PC** only after the host has verified every
block, synchronized the output file, and sent DONE. Sent bytes alone do not
imply a verified file.

The build needs devkitARM, libgba, `gbafix`, Make, and Python 3. Other
installations can set `DEVKITPRO` and `DEVKITARM`. `gba_mb.specs` supplies the RAM
startup/linker script; `gbafix` supplies the cartridge header. The build pads to
16 bytes and validates the logo, header checksum, entry branches, and size.
ELF and map files accompany the `.gba`. `cargo build-gba` invokes the Makefile;
it does not upload the ROM. Use `--port` with `cargo upload` if more than one
cable is connected. Finish an active dump before uploading or flashing.

## Dump ROM

```sh
# Example for a known 8 MiB cartridge. --size is the exact byte count.
cargo dump --release -- \
    --port /dev/ttyACM0 --size 0x800000 cartridge.gba
# Equivalent release binary:
./mb-dumper --port /dev/ttyACM0 --size 0x800000 cartridge.gba
```

Choose your cartridge's actual size: `0x400000` = 4 MiB, `0x800000` = 8 MiB,
`0x1000000` = 16 MiB, `0x2000000` = 32 MiB. `0xC0` reads just the header.
The count must be nonzero, divisible by four, and at most 32 MiB. The ROM header
has no size field. Reading past the physical ROM may return mirrors or open-bus
data. A matching CRC verifies transmission, not cartridge presence or size.

`mb-dumper` keeps one PC connection open, declares the whole range, then
reads up to 253 words per batch. It verifies each block before writing its
little-endian data bytes. It refuses to overwrite files and never retries an
uncertain exchange. Errors/Ctrl+C can leave an incomplete file. Application
errors send CANCEL if the connection is still usable. A dropped connection
cannot notify the GBA; its screen may remain at the last sent byte count.
Reopen and begin a fresh dump to reset application state.

## Save memory

After uploading this v3 image, run:

```sh
cargo dump --release -- --save cartridge.sav
# Equivalent release binary:
./mb-dumper --save cartridge.sav
# For a cartridge known to use 8 KiB EEPROM:
cargo dump --release -- --save --save-type eeprom8k cartridge.sav
# Equivalent release binary:
./mb-dumper --save --save-type eeprom8k cartridge.sav
```

`--save` dumps only save memory and cannot be combined with `--size`.
The PC initially shows `Reading cartridge save memory...` without a percentage.
After preparation, it reports verified bytes, percentage, and transfer speed.
See the [PC guide](../../dumper/README.md#save-memory) for all CLI types and
[emulator testing](../../dumper/README.md#checking-a-save-in-an-emulator).

The SAVE types are 0 = auto, 1 = SRAM/FRAM (32 KiB), 2 = Flash64 (64 KiB),
3 = Flash128 (128 KiB), 4 = EEPROM512 (512 bytes), and 5 = EEPROM8k (8 KiB).
Auto scans the ROM for the first standard save-library signature. It does not
infer EEPROM capacity: choose its type explicitly. Unknown signatures return
`0xBAD00004`; ambiguous EEPROM capacity returns `0xBAD00005`. Overrides must
match the physical cartridge. Modified ROMs and flashcarts may use different
save hardware from the ROM signature.

SAVE schedules foreground work while the serial IRQ remains available.
The host polls SAVE_STATUS using individual words spaced 20 ms apart so GBA
DMA can delay an IRQ without losing the following exchange. After a nonzero
size arrives, the host sends `[0, BEGIN, 0x0E000000, size, 0]`, then uses the
usual READ/CRC/DONE sequence. Do not send HELLO between SAVE and BEGIN.
There is a 60-second preparation timeout on the PC.

The snapshot occupies 128 KiB of EWRAM, outside the serial handler's IWRAM.
SRAM and Flash are read with byte accesses. Flash128 selects each 64 KiB bank
and restores bank 0. EEPROM uses DMA3 with 6- or 14-bit read addresses,
8/8 waitstates, and 68-bit responses. DMA is used only to read EEPROM into the
snapshot; SPI streaming still uses the CPU. The raw save bytes exclude RTC
registers and emulator metadata. No save programming, erase, or flashcart ROM
bank switching is implemented.

The GBA screen shows preparation and save progress, then confirms completion
only after the PC checks the CRCs, synchronizes its file, and sends DONE.

## Transfer speed

All cable exchanges use a nominal 256 kHz SPI clock and a minimum 36 us gap
after each word. Use release builds for both the Pico firmware and PC dumper.

`mb-dumper` reads the bulk capacity from INFO without clocking the GBA. Each
block uses one BULK_EXCHANGE containing READ and all its zero clocks. The reply
contains a stale word, the echoed command, data words, and CRC. The complete
block takes one USB request and reply. No uncertain transfer is replayed.
Terminal progress updates are limited to five per second.

Ignoring USB, scheduler, and block overhead, each 32-bit transfer needs about
125 us of clock time plus the 36 us gap. That gives an approximate ceiling of
24 KiB/s and an 8 MiB wire-time floor of 5.6 minutes. Actual times will be
longer; mb-dumper reports measured KiB/s. Bulk commands provide range validation
and progress without repeating addresses, but still need one SPI word per four
ROM or save bytes. They do not eliminate that wire limit.

The serial interrupt, command handler, and cartridge read routine run from
IWRAM, together with a 1 KiB CRC lookup table. The byte-table CRC uses four
lookups per 32-bit word, applied to both the address and data value. The main
loop renders text about five times a second with serial interrupts enabled; it briefly
masks interrupts only to snapshot four status fields. No rendering, allocation,
or BIOS calls happen in the serial handler. Native tests do not measure
interrupt latency or cable timing. Higher clock rates are not enabled.

## Manual exchanges

Each reply arrives in the **next** transfer. Discard the first returned word.
A command may be split across USB exchanges; the GBA retains its parser state.

```sh
# Identify/reset: second RX must be 0x52444d03.
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

## Application protocol v3

These are numeric u32 application words. SPI sends them MSB-first; PMB3 encodes
them as little-endian u32s in its USB payload. Identity is `0x52444D03`, ACK is
`0x4F4B0003`. Save reads use bank-selection and EEPROM read-address commands,
but never program or erase save data.

| Command | Parameters | Reply on subsequent clock(s) |
| --- | --- | --- |
| `0x52444D50` HELLO | None | ID; resets parser, range, progress, CRC, and save snapshot |
| `0x4245474E` BEGIN | ROM or save snapshot address, byte count | ACK, echoed address, echoed byte count |
| `0x52420000 + count` READ | Then `count + 2` zero clocks | Echoed READ/count, `count` data words, CRC |
| `0x53410000 + type` SAVE | Type 0..5 (see Save memory) | ACK; schedules a save snapshot |
| `0x53544154` SAVE_STATUS | None | 0 while preparing, size when ready, or error |
| `0x444F4E45` DONE | None | ACK only if the declared range was sent; marks complete |
| `0x43414E43` CANCEL | None | ACK; aborts parser/stream and marks cancelled |
| Zero (idle) | None | ID; CRC unchanged |
| Aligned `0x08000000..0x09FFFFFC` | None | Individual ROM word; updates CRC |
| `0x43524300` CHECKSUM (idle) | None | Current CRC; CRC unchanged |

BEGIN validates a nonempty, word-aligned range inside the first 32 MiB ROM
window or the prepared save snapshot at `0x0E000000`. Save ranges must fit the
reported snapshot size. BEGIN preserves that snapshot; HELLO and CANCEL clear
it. READ requires 1..253 words that fit inside the declared remaining range.
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

## Checks

```sh
make -C gba/rom-dumper check
cargo test --locked -p mb-dumper
cargo test --locked -p mb-host -p mb-cable -p mb-uploader -p mb-dumper -p protocol
cargo test --locked -p firmware --lib
python3 gba/rom-dumper/tools/check_rom.py gba/rom-dumper/build/rom-dumper_mb.gba
```

Tests cover individual and sequential reads, CRCs, range/count checks, session
recovery, save snapshot cancellation, save signatures, Flash bank restoration,
EEPROM bit ordering, completion, host byte order, and corrupted responses. Layout checks
verify the built multiboot image. Use a hardware dump to check timing and
cartridge data after changing the transfer path.

References: [devkitPro GBA examples](https://github.com/devkitPro/gba-examples),
[libgba SIO definitions](https://github.com/devkitPro/libgba/blob/master/include/gba_sio.h),
[GBATEK serial documentation](https://mgba-emu.github.io/gbatek/#sio-normal-mode),
[EEPROM](https://problemkaputt.de/gbatek-gba-cart-backup-eeprom.htm), and
[Flash](https://problemkaputt.de/gbatek-gba-cart-backup-flash-rom.htm).
