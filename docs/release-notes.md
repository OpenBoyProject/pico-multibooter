Download the archive matching your OS and CPU. Each includes `mb-cable`,
`mb-uploader`, `mb-dumper`, `pico-multibooter.uf2`, and both GBA ROMs, plus a
usage guide, build metadata, and checksums. Windows tools have `.exe` extensions.
Linux archives require glibc 2.35 or newer.

Copy the UF2 to the Pico's `RPI-RP2` drive in BOOTSEL mode. No compiler or
picotool installation is needed for that method. The firmware is for the
RP2040 Pico, not Pico 2.

The UF2 and GBA ROMs are also attached separately. `SHA256SUMS` covers all
release assets. GitHub's automatic source archives do not contain the binaries.

See the archive's README for upload and cartridge-dumping instructions.
