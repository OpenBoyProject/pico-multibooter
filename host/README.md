# mb-host USB cable library

`mb-host` (Rust import `mb_host`) owns discovery and USB CDC serial I/O using the
[PMB3 protocol](../protocol/README.md). The [`mb-cable`](../cable/README.md) binary
provides cable inspection and raw transfers. ROM loading, the BIOS handshake,
encryption, and cartridge handling live in
[`mb-uploader`](../uploader/README.md) and [`mb-dumper`](../dumper/README.md).

## Library

```rust,no_run
use mb_host::{Cable, list_ports, select_port};

fn example() -> Result<(), mb_host::Error> {
    let port = select_port(&list_ports(false)?)?;
    let mut cable = Cable::open(&port)?;
    let word = cable.exchange(0x12345678)?;
    let words = cable.bulk_exchange(&[0x12345678, 0])?;
    let incoming = cable.bulk_read(256, 0)?;
    cable.bulk_write(&[1, 2, 3])?;
    cable.close()
}
```

`list_ports(true)` enumerates every serial port. `list_ports(false)` returns only
ports whose product name exactly matches `Pico GBA Multibooter`. Neither opens
devices or sends INFO. Results are sorted by port name.

`select_port` requires exactly one product-name match; shared VID/PID alone is
insufficient. Listing identifies candidates, not firmware compatibility.
`Cable::open` takes an explicit name; opening checks the `PICO-MB3` INFO response
and caches its bulk word limit. `cable.info()` returns `DeviceInfo` and refreshes
that limit; `cable.bulk_capacity()` reads the cache without USB I/O. Neither
operation probes the GBA.

`exchange` clocks one word. Bulk methods handle 1-256 words per transaction
and check the cached `bulk_capacity`. Reads send only count and
fill value over USB; the Pico transmits that fill word for each incoming word.
Writes discard SPI replies and validate a byte-count acknowledgement, which
confirms clocking rather than application acceptance. `bulk_exchange` returns
all received words, suitable for application acknowledgements.

All transfers use the firmware's 256 kHz clock and 36 us minimum post-word gap.
The GBA program defines word meanings and response latency. A computed response
usually arrives in the next exchange. The library never automatically batches
or replays a request; applications choose chunk boundaries and checksums.

Keep one `Cable` open for a session. Methods require `&mut self` so requests
remain sequential. `Options` configures `command_timeout` (default 3 seconds),
DTR settling time, and a cloneable `Cancellation`. Another thread can call
`cancel()`; serial I/O polls are capped at 100 ms. `Cable::wait(duration)` is a
cancellable local wait for application protocols; it does not clock the GBA.

`Cable::connect(transport, options)` accepts a custom `Transport` for testing or
alternate backends. The transport must honor timeouts. The library owns
framing, partial I/O, USB CRC checks, response validation, and session cleanup.

## Failures and connections

Opening lowers DTR for 150 ms, discards stale input, raises DTR, waits another
150 ms, and checks INFO. The serial setting of 115200 is CDC metadata, not a USB
throughput limit. Port access is exclusive where supported; close other serial
monitors. Linux enumeration uses sysfs without a libudev development dependency.

Timeouts, I/O errors, malformed replies, device errors, and cancellation close
the session and lower DTR. Reopen before another operation. A lost reply never
causes an automatic resend: the device may already have consumed the words.
Dropping the cable closes the OS handle and lowers DTR. No background tasks or
async runtime are required. Reopening cannot reset a running GBA program.

Use matching PMB3 firmware. Incompatible firmware may reject requests or time
out during INFO; see [protocol compatibility](../protocol/README.md#compatibility).
The library has no upload state or abort command; close the cable to cancel
transfers.

## Diagnostic CLI

Use [`mb-cable`](../cable/README.md), or `cargo cable -- list`, to inspect the
cable and send raw commands.

```sh
cargo test --locked -p mb-host
cargo clippy --locked -p mb-host --all-targets --all-features -- --deny=warnings
```
