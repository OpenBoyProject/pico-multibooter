# mb-cable

Cable inspection and raw transfers using [`mb-host`](../host/README.md).
Run these commands from the workspace root:

```sh
cargo cable -- --help
cargo cable -- list
cargo cable -- list --all
cargo cable -- --port /dev/ttyACM0 info
cargo cable -- exchange 0x12345678
cargo cable -- bulk-exchange 0x12345678 42 0
cargo cable -- bulk-read 256 --fill 0
cargo cable -- bulk-write 1 2 3 4
```

`cargo cable` expands to `cargo run -p mb-cable`.
Build with `cargo build --release -p mb-cable` to produce
`target/release/mb-cable` (`mb-cable.exe` on Windows). Received words go to
stdout; status goes to stderr.

For a repeated probe, `poll` holds one connection open and sends the chosen fill
word with BulkRead until a reply matches `(reply & mask) == (expect & mask)`.
It prints the first 16 samples and a match or timeout summary. This clocks the
GBA repeatedly; use it only with a word that the running BIOS/application can
accept repeatedly. Transport errors stop the command without replay.

```sh
cargo cable -- poll 0x6200 --expect 0x72020000 --mask 0xffff0000 --timeout 10
```

This probe tests multiboot recognition only; use `mb-uploader` to upload a ROM.
`--batch 1` uses one word per USB request; the default is 16. A match is inspected after
its entire batch has been clocked. The polling deadline is checked between
requests, so an outstanding request can extend it by the USB timeout.

```sh
cargo test --locked -p mb-cable
cargo clippy --locked -p mb-cable --all-targets --all-features -- --deny=warnings
```
