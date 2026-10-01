# mb-dumper

PC cartridge ROM and save dumper using [`mb-host`](../host/README.md) for discovery and USB
transfers. It owns the GBA application's commands, address/range validation,
block CRC verification, progress, cancellation, and output file. It does not
perform multiboot or depend on `mb-uploader`.

From the workspace root, build and upload the
[GBA dumper v3](../gba/rom-dumper/README.md). Insert the cartridge and hold
START+SELECT during the GBA boot logo before uploading.

See [environment setup](../README.md#environment-setup) for prerequisites.

> **Before using `--save` on a patched cartridge:** Read the
> [patched-cartridge notes](#patched-cartridges). Incorrect automatic detection
> can modify SRAM save data, even though the requested operation is a dump.

```sh
cargo build-gba rom-dumper
cargo upload --release -- gba/rom-dumper/build/rom-dumper_mb.gba
# Wait for the ready screen, then read an 8 MiB cartridge:
cargo dump --release -- --size 0x800000 cartridge.gba
# Optional explicit port:
cargo dump --release -- --port /dev/ttyACM0 --size 0x800000 cartridge.gba
# Read only the cartridge's save memory:
cargo dump --release -- --save cartridge.sav
```

With a release archive, no build is needed. From the extracted directory:

```sh
./mb-uploader rom-dumper_mb.gba
# Wait for the ready screen. Run either or both, one at a time:
./mb-dumper --size 0x800000 cartridge.gba
./mb-dumper --save cartridge.sav
# Optional explicit port for a save dump:
./mb-dumper --port /dev/ttyACM0 --save cartridge.sav
```

On Windows PowerShell, use `.\mb-uploader.exe` and `.\mb-dumper.exe`.
`cargo dump --release -- ...` runs `mb-dumper` from source. Matching PMB3 Pico firmware
and the GBA dumper v3 ROM are required. Rebuild and upload the new GBA image
when upgrading from v2. Transfers use a 256 kHz clock and a minimum 36 us gap
between words. The same uploaded GBA program handles both ROM and save dumps.

Each block uses one `BulkExchange` to send READ and its zero clocks, receiving
the command echo, up to 253 data words, and CRC. Blocks shrink to fit the cable
capacity; progress updates are limited to five per second. At 256 kHz with
the 36 us gap, the wire limit is about 24 KiB/s before USB and processing
overhead. The CLI reports measured
throughput in KiB/s.

For ROM dumps, `--size` is required: a nonzero multiple of four, at most 32 MiB,
in decimal or hexadecimal. The output must be a new file. Only blocks with a matching echoed
command and application CRC are written; errors leave an incomplete file.
Uncertain transfers are never replayed. DONE is sent after every block is
verified and the output is synchronized. On application errors, CANCEL is sent
if the connection remains usable. Ctrl-C exits with 130 and closes the cable.

## Save memory

`--save` reads only save memory and replaces `--size`. The output is a raw
`.sav` file. Standard ROM signatures select SRAM/FRAM (32 KiB), Flash (64 KiB),
or Flash (128 KiB). The GBA scans the cartridge locally; the PC does not need
to download the ROM first. Detection can take several seconds.

EEPROM signatures do not specify capacity. Choose the known type explicitly;
these are alternatives for different cartridges:

```sh
cargo dump --release -- --save --save-type eeprom512 cartridge.sav
cargo dump --release -- --save --save-type eeprom8k cartridge.sav
# Release binary, for an 8 KiB EEPROM cartridge:
./mb-dumper --save --save-type eeprom8k cartridge.sav
```

| `--save-type` | Output size | Selection |
| --- | --- | --- |
| `auto` (default) | Detected | First recognized ROM save-library signature |
| `sram` | 32 KiB | SRAM or FRAM |
| `flash64` | 64 KiB | Flash with one bank |
| `flash128` | 128 KiB | Flash with two banks |
| `eeprom512` | 512 bytes | EEPROM with 6-bit read addresses |
| `eeprom8k` | 8 KiB | EEPROM with 14-bit read addresses |

`--save-type` requires `--save`; neither can be combined with `--size`.
Overrides must match the cartridge hardware. Detection uses the first standard library
signature; modified ROMs, repros, and flashcarts may need an override or a
different dumper. An unrecognized signature returns an error.

The GBA snapshots the save into RAM before streaming it with the same block
CRC checks as ROM data. Flash128 reads both 64 KiB banks and returns to bank 0.
EEPROM reads use GBA DMA3. No Flash programming or erase commands are sent,
but bank-selection writes can overwrite SRAM if the save type is wrong.
RTC registers and emulator-specific metadata are not included.

During detection and snapshotting, the terminal shows
`Reading cartridge save memory...` without a percentage. Streaming then shows
verified bytes, percentage, and KiB/s. The GBA shows bytes sent and confirms
`Save verified and saved on PC` after the PC finishes. Small saves may finish
before an intermediate progress update is visible.

### Patched cartridges

Automatic detection trusts a ROM library signature; it does not identify the
physical save chip. SRAM patches can leave `FLASH1M_V...` in the ROM even when
Flash has been replaced by SRAM. Other patches, reproduction cartridges, and
flashcarts can also change how and where saves are stored.

Selecting `flash128` on SRAM sends bank-selection writes to `0x0E005555`,
`0x0E002AAA`, and `0x0E000000`. SRAM stores those bytes as save data instead of
interpreting them as commands. This was observed with an SRAM-patched LeafGreen
cartridge: the dump contained duplicated 64 KiB halves and damaged game
checksums. The transfer CRC still passed because it covered the bytes after
they had been changed.

Do not use automatic save detection, or guess a Flash override, on these
cartridges. Determine their save hardware and patch first, and use a dumper
that supports that implementation. The current `sram` option reads only
32 KiB; it is not a general solution for patched 64 KiB saves or custom bank
layouts. Some patches keep a save backup inside the ROM, which may permit
recovery from a full ROM dump, but that is cartridge-specific.

## Checking a save in an emulator

Keep the original `.sav` dump and test with a copy. Use the ROM dumped from
the same cartridge to remove game version and region differences as a variable,
then import the raw save using the emulator's save-memory import or file setup.
This is cartridge save memory, not an emulator save state.

For patched cartridges, the dumped ROM may rely on custom hardware and the
save may use a nonstandard layout. Emulator testing can require a compatible
unmodified ROM and save conversion; matching the dumped ROM alone is not
enough. See [Patched cartridges](#patched-cartridges).

The dumper reads all save bytes, including unused space and redundant slots.
It does not check whether the cartridge contains a valid saved game or verify
the game's internal checksums. A successful transfer CRC only confirms that
the snapshot arrived intact. If the game rejects it, check the detected type,
file size, ROM version, and emulator import setup; a ROM mismatch is only one
possible cause.

For manual commands, on-screen progress, hardware timing considerations, and
cartridge size limitations, see the [GBA guide](../gba/rom-dumper/README.md).

```sh
cargo test --locked -p mb-dumper
```
