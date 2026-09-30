//! Client for gba/rom-dumper. Upload the GBA image first, then run this program.
//! cargo run -p mb-dumper -- --port /dev/ttyACM0 --size 0x800000 game.gba

use clap::Parser;
use mb_host::{Cable, Cancellation, Options, list_ports, select_port};
use std::{
    error::Error,
    fs::OpenOptions,
    io::{self, Write},
    path::PathBuf,
    process::ExitCode,
    time::{Duration, Instant},
};

// Application protocol v2: gba/rom-dumper/source/dumper.h and README.md.
const HELLO: u32 = 0x5244_4d50;
const ID: u32 = 0x5244_4d02;
const BEGIN: u32 = 0x4245_474e;
const READ: u32 = 0x5242_0000;
const DONE: u32 = 0x444f_4e45;
const CANCEL: u32 = 0x4341_4e43;
const ACK: u32 = 0x4f4b_0002;
const ROM_START: u32 = 0x0800_0000;
const ROM_SIZE_MAX: u32 = 32 * 1024 * 1024;
// GBA application limit, independent of the Pico's USB bulk capacity.
const MAX_READ_WORDS: usize = 253;
type Result<T> = std::result::Result<T, Box<dyn Error>>;

#[derive(Parser)]
#[command(
    about = "Read a cartridge through the running gba/rom-dumper application",
    version
)]
struct Args {
    #[arg(short, long)]
    port: Option<String>,
    /// Exact byte count: decimal or 0x-prefixed hex, a multiple of 4, at most 32 MiB.
    #[arg(long, value_parser = parse_size)]
    size: u32,
    /// New output file; never overwrites. On failure this may contain a partial dump.
    output: PathBuf,
}

fn parse_size(text: &str) -> std::result::Result<u32, String> {
    let size = match text.strip_prefix("0x").or_else(|| text.strip_prefix("0X")) {
        Some(hex) => u32::from_str_radix(hex, 16),
        None => text.parse(),
    }
    .map_err(|_| "size must be decimal or 0x-prefixed hexadecimal".to_owned())?;
    if size == 0 || size > ROM_SIZE_MAX || !size.is_multiple_of(4) {
        return Err("size must be 4..33554432 bytes and a multiple of 4".to_owned());
    }
    Ok(size)
}

fn main() -> ExitCode {
    let args = Args::parse();
    let cancellation = Cancellation::default();
    let signal = cancellation.clone();
    if let Err(error) = ctrlc::set_handler(move || signal.cancel()) {
        eprintln!("error: cannot install Ctrl-C handler: {error}");
        return ExitCode::FAILURE;
    }
    match run(args, cancellation.clone()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("\nerror: {error}");
            if cancellation.is_cancelled() {
                ExitCode::from(130)
            } else {
                ExitCode::FAILURE
            }
        }
    }
}

fn run(args: Args, cancellation: Cancellation) -> Result<()> {
    let port = match args.port {
        Some(port) => port,
        None => select_port(&list_ports(false)?)?,
    };
    let mut output = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&args.output)?;
    eprintln!(
        "Writing {} bytes to {}. On failure, this file will be incomplete.",
        args.size,
        args.output.display()
    );
    let mut cable = Cable::open_with_options(
        &port,
        Options {
            cancellation,
            ..Options::default()
        },
    )?;
    let capacity = cable.bulk_capacity()?;
    let block_words = read_words_per_block(capacity)?;
    let started = Instant::now();
    let result = (|| -> Result<()> {
        let mut exchange = |words: &[u32]| cable.bulk_exchange(words);
        begin_transfer(&mut exchange, args.size)?;
        let mut offset = 0;
        let mut last_progress = Instant::now();
        while offset < args.size {
            let count = (((args.size - offset) / 4) as usize).min(block_words);
            let bytes = read_block(&mut exchange, offset, count)?;
            // Only write blocks that passed both count and checksum checks.
            output.write_all(&bytes)?;
            offset += bytes.len() as u32;
            if offset == args.size || last_progress.elapsed() >= Duration::from_millis(200) {
                let elapsed = started.elapsed().as_secs_f64();
                eprint!(
                    "\rVerified {offset}/{} bytes ({}%)  {:.1} KiB/s",
                    args.size,
                    u64::from(offset) * 100 / u64::from(args.size),
                    f64::from(offset) / elapsed.max(0.001) / 1024.0,
                );
                let _ = io::stderr().flush();
                last_progress = Instant::now();
            }
        }
        output.sync_all()?;
        // Only now may the GBA screen claim that the PC saved a verified dump.
        control(&mut exchange, DONE)?;
        Ok(())
    })();
    if result.is_err() && cable.is_usable() {
        // Independent cancellation, never a replay of an uncertain read.
        let _ = control(|words: &[u32]| cable.bulk_exchange(words), CANCEL);
    }
    result?;
    cable.close()?;
    eprintln!(
        "\nSaved {} ({:.1}s).",
        args.output.display(),
        started.elapsed().as_secs_f64()
    );
    Ok(())
}

fn begin_transfer(
    exchange: impl FnOnce(&[u32]) -> std::result::Result<Vec<u32>, mb_host::Error>,
    size: u32,
) -> Result<()> {
    let reply = exchange(&[HELLO, BEGIN, ROM_START, size, 0])?;
    if reply.len() != 5 || reply[1..] != [ID, ACK, ROM_START, size] {
        let reason = match reply.get(1) {
            Some(0x5244_4d01) => "the GBA replied with dumper v1; upload the v2 image",
            Some(&ID) => {
                "the GBA identified as v2, but BEGIN/address/size was not acknowledged correctly"
            }
            _ => {
                "the GBA did not return the dumper v2 identity; check that its ready screen is visible"
            }
        };
        return Err(io::Error::other(format!(
            "{reason}; expected RX [ignored, {ID:08x}, {ACK:08x}, {ROM_START:08x}, {size:08x}], got {reply:08x?}. The request was not retried"
        )).into());
    }
    Ok(())
}

fn control(
    exchange: impl FnOnce(&[u32]) -> std::result::Result<Vec<u32>, mb_host::Error>,
    command: u32,
) -> Result<()> {
    let reply = exchange(&[command, 0])?;
    if reply.len() != 2 || reply[1] != ACK {
        return Err(
            io::Error::other("ROM dumper did not acknowledge completion/cancellation").into(),
        );
    }
    Ok(())
}

fn read_words_per_block(capacity: usize) -> Result<usize> {
    // BEGIN needs five words; READ needs one command and two trailing clocks.
    if capacity < 5 {
        return Err(io::Error::other("cable must support at least 5 words per exchange").into());
    }
    Ok((capacity - 3).min(MAX_READ_WORDS))
}

fn read_block(
    exchange: impl FnOnce(&[u32]) -> std::result::Result<Vec<u32>, mb_host::Error>,
    offset: u32,
    count: usize,
) -> Result<Vec<u8>> {
    if !(1..=MAX_READ_WORDS).contains(&count)
        || !offset.is_multiple_of(4)
        || offset >= ROM_SIZE_MAX
        || count as u32 > (ROM_SIZE_MAX - offset) / 4
    {
        return Err(io::Error::other("invalid cartridge read range").into());
    }
    let mut words = vec![0; count + 3];
    words[0] = READ | count as u32;
    // Send READ and all clocks in one USB request. The GBA's replies include
    // the command echo, data, and CRC, so no second request is needed.
    let reply = exchange(&words)?;
    if reply.len() != words.len() || reply[1] != words[0] {
        return Err(io::Error::other(
            "ROM dumper rejected the bulk block or returned the wrong word count",
        )
        .into());
    }
    // reply[0] is stale; every subsequent word answers the previous request.
    let mut bytes = Vec::with_capacity(count * 4);
    let mut transcript = Vec::with_capacity(count * 8);
    for (i, value) in reply[2..2 + count].iter().enumerate() {
        let address = ROM_START + offset + i as u32 * 4;
        transcript.extend_from_slice(&address.to_le_bytes());
        transcript.extend_from_slice(&value.to_le_bytes());
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    let expected = protocol::crc32(&transcript);
    let received = reply[count + 2];
    if received != expected {
        return Err(io::Error::other(format!(
            "cartridge block at {offset:#010x} failed CRC: expected {expected:08x}, received {received:08x}; block was not written or retried"
        ))
        .into());
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    fn response(words: &[u32], offset: u32) -> Vec<u32> {
        let count = words.len() - 3;
        assert_eq!(words[0], READ | count as u32);
        assert!(words[1..].iter().all(|word| *word == 0));
        let mut response = vec![0xdead_beef, words[0]];
        let mut transcript = Vec::new();
        for i in 0..count {
            let address = ROM_START + offset + i as u32 * 4;
            let value = address ^ 0x89ab_cdef;
            response.push(value);
            transcript.extend_from_slice(&address.to_le_bytes());
            transcript.extend_from_slice(&value.to_le_bytes());
        }
        response.push(protocol::crc32(&transcript));
        response
    }

    #[test]
    fn block_pipeline_byte_order_and_rom_boundaries() {
        for (offset, count) in [(0, MAX_READ_WORDS), (ROM_SIZE_MAX - 4, 1)] {
            let bytes = read_block(|words| Ok(response(words, offset)), offset, count).unwrap();
            assert_eq!(bytes.len(), count * 4);
            assert_eq!(
                &bytes[..4],
                &((ROM_START + offset) ^ 0x89ab_cdef).to_le_bytes()
            );
        }
        for (offset, count) in [
            (0, 0),
            (0, 254),
            (1, 1),
            (ROM_SIZE_MAX - 4, 2),
            (u32::MAX, 1),
        ] {
            assert!(
                read_block(|_| panic!("invalid range clocked the link"), offset, count).is_err()
            );
        }
    }

    #[test]
    fn rejects_corruption_wrong_count_and_short_replies_without_retry() {
        for index in [1, 2, 3] {
            assert!(
                read_block(
                    |words| {
                        let mut reply = response(words, 0);
                        reply[index] ^= 1;
                        Ok(reply)
                    },
                    0,
                    1,
                )
                .is_err()
            );
        }
        for length in [0, 1, 2, 3, 5] {
            assert!(read_block(|_| Ok(vec![0; length]), 0, 1).is_err());
        }
        assert!(read_block(|_| Err(mb_host::Error::SessionLost), 0, 1).is_err());
        // A valid but wrong address received by the GBA must fail too, even if
        // its reply data and checksum agree with each other.
        assert!(read_block(|words| { Ok(response(words, 4)) }, 0, 1).is_err());
    }

    #[test]
    fn blocks_fit_capacity_and_finish_with_a_short_block() {
        for capacity in [5, 8, 16, 256] {
            let limit = read_words_per_block(capacity).unwrap();
            let size = (limit * 2 + 1) * 4;
            let mut offset = 0;
            let mut requests = Vec::new();
            while offset < size {
                let count = ((size - offset) / 4).min(limit);
                let bytes = read_block(
                    |words| {
                        assert!(words.len() <= capacity);
                        requests.push(words.len());
                        Ok(response(words, offset as u32))
                    },
                    offset as u32,
                    count,
                )
                .unwrap();
                offset += bytes.len();
            }
            assert_eq!(offset, size);
            assert_eq!(requests, [limit + 3, limit + 3, 4]);
        }
        for capacity in 0..5 {
            assert!(read_words_per_block(capacity).is_err());
        }
    }

    #[test]
    fn session_handshake_and_explicit_completion() {
        begin_transfer(
            |words| {
                assert_eq!(words, [HELLO, BEGIN, ROM_START, 0x800000, 0]);
                Ok(vec![0, ID, ACK, ROM_START, 0x800000])
            },
            0x800000,
        )
        .unwrap();
        for bad in [
            vec![],
            vec![0, ID, ACK, ROM_START + 4, 8],
            vec![0, ID, ACK, ROM_START, 12],
        ] {
            assert!(begin_transfer(|_| Ok(bad), 8).is_err());
        }
        for command in [DONE, CANCEL] {
            control(
                |words| {
                    assert_eq!(words, [command, 0]);
                    Ok(vec![0, ACK])
                },
                command,
            )
            .unwrap();
            assert!(control(|_| Ok(vec![0, ID]), command).is_err());
        }
    }

    #[test]
    fn handshake_diagnostics_distinguish_version_from_bad_echoes() {
        for (reply, expected) in [
            (vec![0, 0x5244_4d01, 0, 0, 0], "replied with dumper v1"),
            (vec![0, ID, ACK, ROM_START, 4], "identified as v2"),
            (vec![u32::MAX; 5], "did not return the dumper v2 identity"),
            (vec![], "did not return the dumper v2 identity"),
        ] {
            let received = format!("got {reply:08x?}");
            let error = begin_transfer(|_| Ok(reply), 8).unwrap_err().to_string();
            assert!(error.contains(expected), "{error}");
            assert!(error.contains(&received), "{error}");
        }
    }

    #[test]
    fn size_and_cli_validation() {
        Args::command().debug_assert();
        for text in ["0", "3", "33554436", "-4", "0x100000000"] {
            assert!(parse_size(text).is_err());
        }
        assert_eq!(parse_size("0x2000000"), Ok(ROM_SIZE_MAX));
        assert_eq!(parse_size("4"), Ok(4));
        assert!(Args::try_parse_from(["mb-dumper", "game.gba"]).is_err());
        assert!(Args::try_parse_from(["mb-dumper", "--size", "0x800000", "game.gba"]).is_ok());
    }
}
