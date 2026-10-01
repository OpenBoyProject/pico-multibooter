Download the archive matching your OS and CPU. Each includes `mb-cable`,
`mb-uploader`, `mb-dumper`, `pico-multibooter.uf2`, and both GBA ROMs, plus a
usage guide, build metadata, and checksums. Windows tools have `.exe` extensions.
Linux archives require glibc 2.35 or newer.

Copy the UF2 to the Pico's `RPI-RP2` drive in BOOTSEL mode. No compiler or
picotool installation is needed for that method. The firmware is for the
RP2040 Pico, not Pico 2.

The UF2 and GBA ROMs are also attached separately. `SHA256SUMS` covers all
release assets. GitHub's automatic source archives do not contain the binaries.

`mb-dumper` can dump a ROM with `--size` or only save memory with `--save`.
Save dumping detects standard SRAM/FRAM and Flash signatures; EEPROM needs
`--save-type eeprom512` or `--save-type eeprom8k`. Upload the included v3
`rom-dumper_mb.gba` before using it. Save output is a raw `.sav` file.

See the archive's README for upload, ROM dumping, and save dumping examples.
