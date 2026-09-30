# GBA hello world

A devkitPro/libgba multiboot ROM that displays `Hello world!` and waits.

Build from the repository root. See [environment setup](../../README.md#environment-setup) for prerequisites.

```sh
cargo build-gba hello-world
```

Boot the GBA without a cartridge, or hold START+SELECT during the boot logo,
then upload:

```sh
cargo upload --release -- gba/hello-world/build/hello-world_mb.gba
```

The ROM runs from RAM and has no serial command handler. `cargo build-gba`
invokes the devkitPro Makefile. The build uses the ROM dumper's layout checker to
validate the header, entry branches, size, and 16-byte alignment.
Run `make -C gba/hello-world check` to check the built image again.
