# Build tasks

Build the devkitPro C ROMs from Cargo, starting at the workspace root:

See [environment setup](../README.md#environment-setup) for prerequisites.

```sh
cargo build-gba
cargo build-gba hello-world
cargo build-gba rom-dumper
```

Without a target, both ROMs are built. `cargo build-gba` expands to
`cargo run -p xtask -- gba`. The helper runs `make all` in each selected
project and stops if a build fails. Make handles incremental rebuilds.

This needs Make, Python 3, devkitARM, libgba, and gbafix. Python runs the ROM
padding and layout checker. Set `DEVKITPRO` and `DEVKITARM` to your installation
paths. The ROMs always use the Makefiles' `-O2` setting, regardless of the Rust
build profile. Building or testing the helper itself does not invoke devkitPro.

Outputs:

- `gba/hello-world/build/hello-world_mb.gba`
- `gba/rom-dumper/build/rom-dumper_mb.gba`

The `rom-dumper` image supports both cartridge ROM and save-memory dumps with
[`mb-dumper`](../dumper/README.md). There is no separate save-dumper build target.

Each build fixes and validates the multiboot header and pads to 16 bytes.
The command builds files only. To upload one, put the GBA into multiboot mode:

```sh
cargo upload --release -- gba/hello-world/build/hello-world_mb.gba
```
