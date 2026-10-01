# Pico cable firmware

The firmware exposes `Pico GBA Multibooter`, a USB CDC serial device implementing
[PMB3](../protocol/README.md). It clocks host-supplied words over SPI and returns
received words. It contains no BIOS handshake, encryption, ROM checksums, or
upload state. Those belong to [`mb-uploader`](../uploader/README.md).

## Build and flash

Run commands from the workspace root. See [environment setup](../README.md#environment-setup) for prerequisites.

```sh
cargo build-firmware --locked --release
# Connect the Pico in BOOTSEL mode; requires picotool on PATH.
cargo flash --locked --release
```

The ELF is `target/thumbv6m-none-eabi/release/firmware`. Use matching PMB3
firmware and PC binaries. Development VID/PID is
`16c0:27dd`; identify the device by its product name and INFO reply `PICO-MB3`.

## Wiring

| Pico GPIO | Pico header pin | GBA signal |
| --- | --- | --- |
| GP2 | 4 | SC / clock |
| GP3 | 5 | SI / data into GBA |
| GP4 | 6 | SO / data out of GBA |
| GND | 3 (or another GND) | GND |

Use a common ground and 3.3 V signals. USB powers the Pico; GBA VCC and SD are
unused. SPI uses mode 3, MSB-first, nominal 256 kHz, with no chip select. The
clock is rounded down to a rate supported by the RP2040 divider.

## Operations and timing

Info returns the protocol identity and bulk word limit without clocking SPI.
It does not probe GBA connection or readiness. Exchange sends and receives one word.
BulkWrite sends words and discards replies. BulkRead repeatedly sends a fill
word and returns incoming words. BulkExchange sends and receives word arrays.
There are at most 256 words per bulk command, using fixed buffers and no heap.

The onboard LED is off while idle and on while handling a command and sending
its reply. It turns off on completion, error, or disconnect. Repeated commands
can make it flicker; there is no idle heartbeat.

Each SPI word finishes synchronously, then waits at least 36 us asynchronously.
Single and bulk exchanges use the same pacing. The GBA must prepare and rearm
its serial interface; a computed reply arrives on a later transfer.

## Sessions

The PC must assert DTR. A DTR drop, USB reset, deconfiguration, disconnect, or
suspend cancels the pending operation and discards partial framing and replies.
A generation counter remembers short DTR drops. The link checks the connection
before clocking each word. Cancellation does not interrupt a word halfway.

Thirty seconds without received bytes discards an incomplete frame. Replies
have a 2-second write deadline. USB replies end with a short packet, including
a zero-length packet when needed. Transfers are processed sequentially. A
failed transfer returns an error, never partial success. Do not automatically
retry after a lost response: the GBA may already have consumed the words.

Closing the cable does not reset the GBA or undo an application operation.

## Code and tests

- `main.rs`: USB descriptors, SPI and LED setup, and concurrent USB tasks.
- `usb.rs`: USB packet I/O, connection monitoring, and the SPI adapter.
- `server.rs`: stateless command dispatch and replies.
- `link.rs`: raw exchange interface and minimum word gap.

`lib.rs` exposes `server` and `link` so their tests can run on the PC with a
mock link. The Pico executable links this code with its hardware adapters.

```sh
cargo test --locked -p firmware --lib
cargo build-firmware --locked --release
```

Tests cover exact word forwarding, bulk lengths, rejected commands, link
errors, and cancellation. They use a mock link and do not measure SPI timing.
BIOS handshake tests live in `mb-uploader`.
