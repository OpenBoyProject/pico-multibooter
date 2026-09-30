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

## Dump a cartridge

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
