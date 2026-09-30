# mb-dumper

PC cartridge dumper using [`mb-host`](../host/README.md) for discovery and USB
transfers. It owns the GBA application's commands, address/range validation,
block CRC verification, progress, cancellation, and output file. It does not
perform multiboot or depend on `mb-uploader`.

From the workspace root, build and upload the
[GBA dumper v2](../gba/rom-dumper/README.md). Insert the cartridge and hold
START+SELECT during the GBA boot logo before uploading:

See [environment setup](../README.md#environment-setup) for prerequisites.

```sh
cargo build-gba rom-dumper
cargo run --release -p mb-uploader -- gba/rom-dumper/build/rom-dumper_mb.gba
# Wait for the ready screen, then read an 8 MiB cartridge:
cargo run --release -p mb-dumper -- --size 0x800000 cartridge.gba
# Optional explicit port:
cargo run --release -p mb-dumper -- --port /dev/ttyACM0 --size 0x800000 cartridge.gba
```

`cargo dump --release -- ...` runs the same binary. Matching PMB3 Pico firmware
and the GBA dumper v2 ROM are required. Transfers use a 256 kHz clock and a
minimum 36 us gap between words.

Each block uses one `BulkExchange` to send READ and its zero clocks, receiving
the command echo, up to 253 ROM words, and CRC. Blocks shrink to fit the cable
capacity; progress updates are limited to five per second. At 256 kHz with
the 36 us gap, the wire limit
is about 24 KiB/s before USB and processing overhead. The CLI reports measured
throughput in KiB/s.

`--size` is required: a nonzero multiple of four, at most 32 MiB, in decimal or
hexadecimal. The output must be a new file. Only blocks with a matching echoed
command and application CRC are written; errors leave an incomplete file.
Uncertain transfers are never replayed. DONE is sent after every block is
verified and the output is synchronized. On application errors, CANCEL is sent
if the connection remains usable. Ctrl-C exits with 130 and closes the cable.

For manual commands, on-screen progress, hardware timing considerations, and
cartridge size limitations, see the [GBA guide](../gba/rom-dumper/README.md).

```sh
cargo test --locked -p mb-dumper
```
