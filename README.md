# Pico multibooter

A Raspberry Pi Pico cable that connects a PC over USB to the Game Boy Advance
link port. It can upload multiboot ROMs into GBA RAM and exchange data with
running programs, including the cartridge ROM and save dumper included here.

Instructions for building the cable and the accompanying blog post can be found
at [meirl.dev/blog/multiboot-cable](https://meirl.dev/blog/multiboot-cable).

This workspace contains the Pico firmware, a reusable PC USB library, PC tools,
and GBA programs.

| Directory | Cargo package | Purpose |
| --- | --- | --- |
| `protocol/` | `protocol` | Shared `no_std` PMB3 commands, framing, and CRC |
| `firmware/` | `firmware` | RP2040 USB/SPI firmware |
| `host/` | `mb-host` | Library: discovery, connections, raw transfers, and cancellation |
| `cable/` | `mb-cable` | CLI: cable inspection and raw transfers |
| `xtask/` | `xtask` | Build tasks for devkitPro GBA ROMs |
| `uploader/` | `mb-uploader` | Multiboot CLI and library: ROM loading, BIOS handshake, encryption, checksum |
| `dumper/` | `mb-dumper` | CLI: cartridge ROM/save dumping, block verification, and file output |

All three PC applications use `mb-host`. The USB library has no ROM loading or
multiboot algorithm. The PC applications share `protocol` with the firmware.
The uploader batches encrypted words through raw `BulkExchange` and validates
BIOS replies on the PC. The dumper uses raw bulk transfers with the running GBA
application. The Pico has no multiboot state or algorithm; it forwards the
host's words unchanged. Use matching PMB3 firmware and PC binaries. The GBA
dumper has its own application protocol, currently version 3.

## Environment setup

### Using a release archive

Download the archive for your OS and CPU from
[Releases](https://github.com/OpenBoyProject/pico-multibooter/releases) and
extract it. Each archive includes:

- `mb-cable`, `mb-uploader`, and `mb-dumper` (`.exe` on Windows).
- `pico-multibooter.uf2` for the RP2040 Pico.
- `hello-world_mb.gba` and `rom-dumper_mb.gba`.
- A usage guide, build information, and SHA-256 checksums.

Archives are built for Linux x64, Windows x64, macOS Intel, and macOS Apple
Silicon. Linux binaries require glibc 2.35 or newer (Ubuntu 22.04 or newer).
No Rust or devkitPro installation is needed to use the archive. Flash the UF2
by copying it to the Pico's BOOTSEL drive; picotool is optional for this method.
Use the firmware and PC tools from the same release.

On Linux, your account needs permission to open the USB serial port. On Debian
and Ubuntu this usually means membership in `dialout`; log out and back in
after changing groups. Other distributions may use a different group. Close
other programs using the port before running a cable command.

### Building from source

Install the tools needed for the parts you want to build:

| Part | Requirements |
| --- | --- |
| PC tools | Current stable [Rust and Cargo](https://rustup.rs/), plus a native C linker: GCC/Clang on Linux, Xcode Command Line Tools on macOS, or Visual Studio C++ Build Tools with a Windows SDK on Windows |
| Pico firmware | Rust's `thumbv6m-none-eabi` target; [picotool](https://github.com/raspberrypi/picotool#building--installing) on PATH for `cargo flash` |
| GBA ROMs | [devkitPro](https://devkitpro.org/wiki/Getting_Started) with the `gba-dev` package group (devkitARM, libgba, and GBA tools including gbafix), GNU Make, and Python 3 |

Install the Pico target:

```sh
rustup target add thumbv6m-none-eabi
```

Use devkitPro's installer/package manager to install `gba-dev`. On Windows,
run GBA build commands in the devkitPro MSYS2 shell, with Cargo available on
PATH. On Linux/macOS, set these variables to your installation paths, for
example:

```sh
export DEVKITPRO=/opt/devkitpro
export DEVKITARM="$DEVKITPRO/devkitARM"
export PATH="$DEVKITPRO/tools/bin:$PATH"
```

`make`, `python3`, and `cargo` must be available on PATH. If your Python command
is named `python`, set `PYTHON=python`. Python pads and validates the compiled
GBA images; the release packaging script also uses it. Prebuilt picotool
packages are linked from its installation guide. The Rust firmware itself does not need the Pico
C SDK or a separate ARM GCC toolchain.

## Build and run

Run source-build commands from the workspace root:

```sh
cargo build --locked --release  # PC tools and libraries
cargo build-firmware --locked --release
cargo build-gba                 # Both GBA ROMs
cargo build-gba hello-world     # Only hello world
cargo build-gba rom-dumper      # Only the cartridge dumper
```

PC binaries are under `target/release/`. The firmware ELF is
`target/thumbv6m-none-eabi/release/firmware`. GBA ROMs are written to
`gba/<name>/build/<name>_mb.gba`. Their Makefiles use `-O2` and validate the
multiboot images. Direct `make -C gba/<name>` commands also work.

`cargo cable`, `cargo upload`, and `cargo dump` run the corresponding PC
binaries. Add `--release` before `--` to use an optimized build. The workspace
has no global target override: PC tools and Pico firmware need different
Rust targets, so use the commands above instead of `cargo build --workspace`.

See [USB library](host/README.md), [cable CLI](cable/README.md),
[uploader](uploader/README.md), [dumper](dumper/README.md),
[firmware](firmware/README.md), and [build tasks](xtask/README.md) for details.

## Examples

Use either the release commands from the extracted archive directory or the
Cargo commands from the workspace root. The release examples use Linux/macOS
syntax; in Windows PowerShell, use `.\mb-cable.exe`, `.\mb-uploader.exe`,
and `.\mb-dumper.exe`.

### 1. Flash the Pico firmware

Hold BOOTSEL while connecting the Pico to USB, then release it. With a release
archive, copy `pico-multibooter.uf2` to the `RPI-RP2` drive. The Pico restarts
and appears as a USB serial device. This firmware targets the RP2040 Pico,
not the RP2350 Pico 2.

Alternatively, if picotool is installed:

```sh
# Release archive:
picotool load -v -x pico-multibooter.uf2

# From source: builds and flashes the firmware.
cargo flash --locked --release
```

Check that the cable appears and responds:

```sh
# Release archive:
./mb-cable list
./mb-cable info

# From source:
cargo cable -- list
cargo cable -- info
```

### 2. Upload hello world

The release archive includes `hello-world_mb.gba`. To build it from source:

```sh
cargo build-gba hello-world
```

Connect the cable to the GBA and turn it on without a cartridge. Once it is
waiting at the boot screen, upload the ROM:

```sh
# Release archive:
./mb-uploader hello-world_mb.gba

# From source:
cargo upload --release -- gba/hello-world/build/hello-world_mb.gba
```

The uploader reports a verified checksum, and the GBA displays `Hello world!`.
The program runs from RAM until the GBA is switched off or restarted.

### 3. Dump a cartridge ROM or save

The release archive includes `rom-dumper_mb.gba`. To build it from source:

```sh
cargo build-gba rom-dumper
```

With the GBA switched off, insert the cartridge and connect the cable. Turn it
on while holding START+SELECT through the boot logo to enter multiboot mode
instead of starting the cartridge. Keep the cartridge inserted and upload:

```sh
# Release archive:
./mb-uploader rom-dumper_mb.gba

# From source:
cargo upload --release -- gba/rom-dumper/build/rom-dumper_mb.gba
```

Wait for `GBA ROM Dumper ready...` on the GBA, then run the PC dumper. For an
8 MiB cartridge:

```sh
# Release archive:
./mb-dumper --size 0x800000 cartridge.gba

# From source:
cargo dump --release -- --size 0x800000 cartridge.gba
```

Set `--size` to the cartridge's actual ROM size: `0x400000` for 4 MiB,
`0x800000` for 8 MiB, `0x1000000` for 16 MiB, or `0x2000000` for 32 MiB.
The size is not detected automatically. Choose a new output filename; existing
files are never overwritten. The PC verifies each block before writing it,
and the GBA shows `PC verified and saved ROM` when the dump finishes.

To dump only save memory with the same GBA program:

> **Patched cartridges:** Do not use automatic save detection on SRAM-patched
> or other cartridges with an unknown save implementation. A leftover Flash
> signature can select bank-switch commands that overwrite SRAM save bytes.
> Read the [patched-cartridge notes](dumper/README.md#patched-cartridges) first.

```sh
# Release archive:
./mb-dumper --save cartridge.sav

# From source:
cargo dump --release -- --save cartridge.sav
```

ROM and save dumps are separate commands; either can run first after uploading
the GBA dumper. `--save` detects standard SRAM/FRAM and Flash saves from ROM
signatures. EEPROM requires a known size, for example:

```sh
# Release archive, for a cartridge with 8 KiB EEPROM:
./mb-dumper --save --save-type eeprom8k cartridge.sav

# From source, for the same type:
cargo dump --release -- --save --save-type eeprom8k cartridge.sav
```

Use `eeprom512` for 512-byte EEPROM. Do not pass the ROM's `--size` when
dumping saves. Detection and the RAM snapshot happen before percentage progress
starts. The resulting `.sav` contains raw save memory; the transfer CRC does
not validate the game's save format. See the
[dumper guide](dumper/README.md#save-memory) for supported types and emulator
testing. Save dumping requires the v3 GBA dumper image and matching PC tool.

Upload and dump commands auto-select a single cable. To choose a port:

```sh
# Release archive (use COM3 on Windows or the listed device on macOS):
./mb-uploader --port /dev/ttyACM0 hello-world_mb.gba

# From source:
cargo upload --release -- --port /dev/ttyACM0 gba/hello-world/build/hello-world_mb.gba
```

Use `mb-cable list` or `cargo cable -- list` to find the port.

## Releases

Pushing a tag such as `v1.0` runs the [release workflow](.github/workflows/release.yml).
It tests and builds the PC tools, Pico UF2, and both GBA ROMs, then publishes
platform archives, standalone UF2/GBA files, and `SHA256SUMS`. Tags containing
`-` are published as prereleases. No release is published if a build fails.
See [release maintenance](docs/releases.md) for the process and local checks.

## Checks

```sh
cargo fmt --all -- --check
cargo test --locked -p mb-host -p mb-cable -p mb-uploader -p mb-dumper -p protocol -p xtask
cargo test --locked -p firmware --lib
cargo clippy --locked -p mb-host -p mb-cable -p mb-uploader -p mb-dumper -p protocol -p xtask --all-targets --all-features -- --deny=warnings
cargo clippy --locked -p firmware --lib --tests -- --deny=warnings
cargo clippy --locked -p firmware --target thumbv6m-none-eabi --all-features -- --deny=warnings
make -C gba/rom-dumper check
```

Tests cover fragmented serial I/O and failure handling, host multiboot golden
values and batched acknowledgement checks, and cartridge block verification.
Native tests do not measure USB/GBA timing; compare throughput and CRC results
on hardware when changing the transfer path.
