# Pico multibooter

This archive contains the PC tools, RP2040 Pico firmware, and two GBA multiboot
ROMs. No Rust or devkitPro installation is needed. Use the files from the same
archive together.

Run commands in this directory. Linux/macOS examples use `./mb-cable`;
Windows PowerShell uses `.\mb-cable.exe` (likewise for uploader and dumper).
Linux binaries require glibc 2.35 or newer. Your account needs access to the
USB serial port; on Debian/Ubuntu, this usually requires the `dialout` group.

## Flash the cable

Hold BOOTSEL while plugging the Pico into USB. Release BOOTSEL, then copy
`pico-multibooter.uf2` onto the `RPI-RP2` drive. It restarts as a serial device.
This image is for the RP2040 Pico, not Pico 2. Check the connection:

```sh
./mb-cable list
./mb-cable info
```

## Hello world

Connect the cable to the GBA and turn it on without a cartridge. Upload while
it is waiting at the boot screen:

```sh
./mb-uploader hello-world_mb.gba
```

The GBA should display `Hello world!`.

## Dump a cartridge ROM or save

Switch the GBA off, insert the cartridge, and turn it on holding START+SELECT
through the boot logo. Keep the cartridge inserted and upload:

```sh
./mb-uploader rom-dumper_mb.gba
```

Wait for `GBA ROM Dumper ready...`, then dump an 8 MiB ROM:

```sh
./mb-dumper --size 0x800000 cartridge.gba
```

Use the cartridge's actual size: `0x400000` for 4 MiB, `0x800000` for 8 MiB,
`0x1000000` for 16 MiB, or `0x2000000` for 32 MiB. Size is not auto-detected.
Choose a new filename; existing files are never overwritten. Each block is
CRC-checked before writing. A failed dump can leave a partial output file.

To dump only the save with the same GBA program, run this as a separate command.
You do not need to dump the ROM first:

> **Patched cartridges:** Do not use automatic save detection on SRAM-patched
> cartridges or other cartridges with an unknown save implementation. ROM
> signatures can describe the original hardware instead of the installed chip.
> A false Flash128 detection sends bank-switch writes that overwrite SRAM save
> bytes. Use a dumper that supports the cartridge's actual hardware and patch.
> The `sram` override reads only 32 KiB and does not handle arbitrary patched
> 64 KiB saves or custom layouts. A successful transfer CRC does not catch this.

```sh
./mb-dumper --save cartridge.sav
```

Standard SRAM/FRAM and 64/128 KiB Flash saves are detected from ROM signatures.
For EEPROM, choose the cartridge's known capacity:

```sh
# 512-byte EEPROM:
./mb-dumper --save --save-type eeprom512 cartridge.sav
# Or 8 KiB EEPROM:
./mb-dumper --save --save-type eeprom8k cartridge.sav
```

Other overrides are `sram` (32 KiB), `flash64` (64 KiB), and `flash128`
(128 KiB); use these only when the hardware type is known. Save dumps do not
use `--size`. Detection and snapshotting show a preparation message, followed
by percentage progress during transfer. Use the included v3 GBA dumper image
with this PC tool.

The output is raw save memory, without RTC data or emulator save-state data.
For emulator testing, keep the original dump, import a copy as cartridge save
memory, and use the ROM dumped from the same cartridge. CRC checks confirm
transfer integrity, not the validity of the game's saved data. If the game
rejects a save, also check its type, file size, and emulator import setup.
Patched ROMs may depend on custom hardware or store saves in a different layout;
they can require a compatible unmodified ROM and a converted save for emulation.

The tools auto-select a single cable. Use `--port COM3` on Windows or
`--port /dev/ttyACM0` on Linux if needed; `mb-cable list` shows available cables.
Use `--help` for command options.

## Checksums and build information

`BUILDINFO.json` records the tag, commit, and platform. `SHA256SUMS` contains
hashes of the files in this archive. To check them on Linux, run
`sha256sum -c SHA256SUMS`; on macOS, use `shasum -a 256 -c SHA256SUMS`.
On Windows, `Get-FileHash .\mb-cable.exe -Algorithm SHA256` prints a hash to
compare with its entry. The release page also provides hashes of the archives.

Cable construction: https://meirl.dev/blog/multiboot-cable

Source and full documentation: https://github.com/OpenBoyProject/pico-multibooter
