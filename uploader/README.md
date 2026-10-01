# mb-uploader

PC multiboot uploader built on [`mb-host`](../host/README.md). It owns the ROM
loader, size validation and padding, BIOS handshake, encryption, acknowledgement
checks, checksum, and progress reporting. It never opens or configures a serial
port directly; `mb-host` handles the USB connection and transfers.
Run commands from the workspace root:

```sh
cargo run --release -p mb-uploader -- game.gba
cargo run --release -p mb-uploader -- --port /dev/ttyACM0 game.gba
# Short alias:
cargo upload --release -- game.gba
```

Use a multiboot-compatible ROM of 400-262144 bytes. The uploader validates the
file before connecting and pads its end to 16 bytes. It does not convert normal
cartridge images. Boot the GBA without a cartridge or hold START+SELECT during
the logo. Success is reported only after the BIOS returns the expected checksum.

To read a cartridge's ROM or save memory, upload the included
[`gba/rom-dumper`](../gba/rom-dumper/README.md) image, then run
[`mb-dumper`](../dumper/README.md) with `--size` or `--save`.

Matching PMB3 firmware is required. The uploader reads the capacity from INFO
and sends raw BulkExchange requests at 256 kHz with a minimum 36 us word gap.
All BIOS processing is done on the PC. The Pico has no upload commands or
multiboot state.

Header words and encrypted payload words are batched up to the cable's capacity.
Discovery sends `0x6200` until the GBA replies `0x7202`. The uploader then sends
`0x6102` to select client 1 before transferring the 192-byte header, followed by
`0x6200` and `0x6202` to finish the header exchange. Command values are described
in [GBATEK](https://rust-console.github.io/gbatek-gbaonly/#multiboot-transfer-protocol).
Payload acknowledgements are checked on the PC after each batch. A bad
acknowledgement means the rest of that batch has already been clocked; the
uploader closes the session without replaying it. Restart the GBA before a new
attempt. Readiness, palette negotiation, and checksum polling have deadlines;
USB transactions have their own deadline in `mb-host`. A currently outstanding
USB request can extend a polling deadline by up to its transaction timeout.
The 62.5 ms handshake wait runs on the PC and supports cancellation.

Ctrl-C exits with status 130, other failures with 1, and invalid arguments with 2.
For reuse from Rust, this package also exports `Rom`, `upload_rom`, and progress
and error types:

```rust,no_run
use mb_host::{Cable, list_ports, select_port};
use mb_uploader::{Rom, upload_rom};

fn example() -> Result<(), Box<dyn std::error::Error>> {
    let rom = Rom::load("game.gba")?;
    let mut cable = Cable::open(&select_port(&list_ports(false)?)?)?;
    upload_rom(&mut cable, &rom, |event| eprintln!("{event:?}"))?;
    cable.close()?;
    Ok(())
}
```

Tests check golden encryption/checksum values, maximum ROM size and offset
wraparound, progress, deadlines, and failed acknowledgements. Fragmented USB
simulation verifies that the public uploader uses only raw bulk commands and
closes on cancellation or lost replies. These tests do not measure hardware
timing.

```sh
cargo test --locked -p mb-uploader
```
