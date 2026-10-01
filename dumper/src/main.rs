//! Client for gba/rom-dumper. Upload the GBA image first, then run this program.
//! cargo run -p mb-dumper -- --port /dev/ttyACM0 --size 0x800000 game.gba

use clap::{Parser, ValueEnum};
use mb_host::{Cable, Cancellation, Options, list_ports, select_port};
use std::{
    error::Error,
    fs::OpenOptions,
    io::{self, Write},
    path::PathBuf,
    process::ExitCode,
    time::{Duration, Instant},
};

// Application protocol v3: gba/rom-dumper/source/dumper.h and README.md.
const HELLO: u32 = 0x5244_4d50;
const ID: u32 = 0x5244_4d03;
const BEGIN: u32 = 0x4245_474e;
const READ: u32 = 0x5242_0000;
const DONE: u32 = 0x444f_4e45;
const CANCEL: u32 = 0x4341_4e43;
const ACK: u32 = 0x4f4b_0003;
const SAVE: u32 = 0x5341_0000;
const SAVE_STATUS: u32 = 0x5354_4154;
const SAVE_START: u32 = 0x0e00_0000;
const SAVE_SIZE_MAX: u32 = 128 * 1024;
const ROM_START: u32 = 0x0800_0000;
const ROM_SIZE_MAX: u32 = 32 * 1024 * 1024;
// GBA application limit, independent of the Pico's USB bulk capacity.
const MAX_READ_WORDS: usize = 253;
type Result<T> = std::result::Result<T, Box<dyn Error>>;

#[derive(Clone, Copy, Debug, Default, ValueEnum)]
#[repr(u32)]
enum SaveType {
    #[default]
    Auto = 0,
    Sram = 1,
    Flash64 = 2,
    Flash128 = 3,
    Eeprom512 = 4,
    Eeprom8k = 5,
}

impl SaveType {
    fn size(self) -> Option<u32> {
        match self {
            Self::Auto => None,
            Self::Sram => Some(32768),
            Self::Flash64 => Some(65536),
            Self::Flash128 => Some(131072),
            Self::Eeprom512 => Some(512),
            Self::Eeprom8k => Some(8192),
        }
    }
}

#[derive(Parser)]
#[command(
    about = "Read a cartridge through the running gba/rom-dumper application",
    version
)]
struct Args {
    #[arg(short, long)]
    port: Option<String>,
    /// ROM byte count: decimal or hex, a multiple of 4, at most 32 MiB.
    #[arg(long, value_parser = parse_size, required_unless_present = "save", conflicts_with = "save")]
    size: Option<u32>,
    /// Dump only cartridge save memory. Detect SRAM/Flash from the ROM signature.
    #[arg(long)]
    save: bool,
    /// Override save detection. EEPROM requires an explicit size/type.
    #[arg(long, value_enum, requires = "save", conflicts_with = "size")]
    save_type: Option<SaveType>,
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
        let start = if args.save { SAVE_START } else { ROM_START };
        let size = if args.save {
            eprintln!("Reading cartridge save memory...");
            prepare_save(
                |word| {
                    // One clock per request leaves time for the GBA's EEPROM DMA
                    // and serial IRQ. Bulk streaming starts after the snapshot.
                    cable.wait(Duration::from_millis(20))?;
                    let reply = cable.bulk_exchange(&[word])?;
                    reply.first().copied().ok_or(mb_host::Error::SessionLost)
                },
                args.save_type.unwrap_or_default(),
            )?
        } else {
            args.size.expect("clap requires --size for ROM dumps")
        };
        let mut output = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&args.output)?;
        eprintln!(
            "Writing {size} bytes to {}. On failure, this file will be incomplete.",
            args.output.display()
        );
        let mut exchange = |words: &[u32]| cable.bulk_exchange(words);
        begin_transfer(&mut exchange, start, size)?;
        let mut offset = 0;
        let mut last_progress = Instant::now();
        while offset < size {
            let count = (((size - offset) / 4) as usize).min(block_words);
            let bytes = read_block(&mut exchange, start, offset, count)?;
            // Only write blocks that passed both count and checksum checks.
            output.write_all(&bytes)?;
            offset += bytes.len() as u32;
            if offset == size || last_progress.elapsed() >= Duration::from_millis(200) {
                let elapsed = started.elapsed().as_secs_f64();
                eprint!(
                    "\rVerified {offset}/{size} bytes ({}%)  {:.1} KiB/s",
                    u64::from(offset) * 100 / u64::from(size),
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
    start: u32,
    size: u32,
) -> Result<()> {
    // HELLO clears the save snapshot; use an idle identity clock for save dumps.
    let hello = if start == SAVE_START { 0 } else { HELLO };
    let reply = exchange(&[hello, BEGIN, start, size, 0])?;
    if reply.len() != 5 || reply[1..] != [ID, ACK, start, size] {
        let reason = match reply.get(1) {
            Some(0x5244_4d01 | 0x5244_4d02) => {
                "the GBA replied with an older dumper; upload the v3 image"
            }
            Some(&ID) => {
                "the GBA identified as v3, but BEGIN/address/size was not acknowledged correctly"
            }
            _ => {
                "the GBA did not return the dumper v3 identity; check that its ready screen is visible"
            }
        };
        return Err(io::Error::other(format!(
            "{reason}; expected RX [ignored, {ID:08x}, {ACK:08x}, {start:08x}, {size:08x}], got {reply:08x?}. The request was not retried"
        )).into());
    }
    Ok(())
}

fn prepare_save(
    mut step: impl FnMut(u32) -> std::result::Result<u32, mb_host::Error>,
    save_type: SaveType,
) -> Result<u32> {
    step(HELLO)?;
    if step(SAVE | save_type as u32)? != ID {
        return Err(io::Error::other(
            "save dumping requires the GBA dumper v3; upload it and wait for the ready screen",
        )
        .into());
    }
    if step(SAVE_STATUS)? != ACK {
        return Err(io::Error::other("GBA dumper did not acknowledge the save request").into());
    }
    let deadline = Instant::now() + Duration::from_secs(60);
    while Instant::now() < deadline {
        match step(SAVE_STATUS)? {
            0 => continue,
            size @ (512 | 8192 | 32768 | 65536 | 131072) => {
                if save_type.size().is_some_and(|expected| size != expected)
                    || matches!(save_type, SaveType::Auto) && size < 32768
                {
                    return Err(io::Error::other(
                        "GBA save size does not match the requested type",
                    )
                    .into());
                }
                return Ok(size);
            }
            0xbad0_0004 => return Err(io::Error::other(
                "save type was not detected; use --save-type with the cartridge's known save type",
            )
            .into()),
            0xbad0_0005 => {
                return Err(io::Error::other(
                    "EEPROM detected; specify --save-type eeprom512 or --save-type eeprom8k",
                )
                .into());
            }
            reply => {
                return Err(
                    io::Error::other(format!("unexpected save status {reply:#010x}")).into(),
                );
            }
        }
    }
    Err(io::Error::other("timed out preparing cartridge save").into())
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
    start: u32,
    offset: u32,
    count: usize,
) -> Result<Vec<u8>> {
    let limit = match start {
        ROM_START => ROM_SIZE_MAX,
        SAVE_START => SAVE_SIZE_MAX,
        _ => return Err(io::Error::other("invalid cartridge source").into()),
    };
    if !(1..=MAX_READ_WORDS).contains(&count)
        || !offset.is_multiple_of(4)
        || offset >= limit
        || count as u32 > (limit - offset) / 4
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
        let address = start + offset + i as u32 * 4;
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

    fn response(words: &[u32], start: u32, offset: u32) -> Vec<u32> {
        let count = words.len() - 3;
        assert_eq!(words[0], READ | count as u32);
        assert!(words[1..].iter().all(|word| *word == 0));
        let mut response = vec![0xdead_beef, words[0]];
        let mut transcript = Vec::new();
        for i in 0..count {
            let address = start + offset + i as u32 * 4;
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
            let bytes = read_block(
                |words| Ok(response(words, ROM_START, offset)),
                ROM_START,
                offset,
                count,
            )
            .unwrap();
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
                read_block(
                    |_| panic!("invalid range clocked the link"),
                    ROM_START,
                    offset,
                    count
                )
                .is_err()
            );
        }
    }

    #[test]
    fn rejects_corruption_wrong_count_and_short_replies_without_retry() {
        for index in [1, 2, 3] {
            assert!(
                read_block(
                    |words| {
                        let mut reply = response(words, ROM_START, 0);
                        reply[index] ^= 1;
                        Ok(reply)
                    },
                    ROM_START,
                    0,
                    1,
                )
                .is_err()
            );
        }
        for length in [0, 1, 2, 3, 5] {
            assert!(read_block(|_| Ok(vec![0; length]), ROM_START, 0, 1).is_err());
        }
        assert!(read_block(|_| Err(mb_host::Error::SessionLost), ROM_START, 0, 1).is_err());
        // A valid but wrong address received by the GBA must fail too, even if
        // its reply data and checksum agree with each other.
        assert!(
            read_block(
                |words| { Ok(response(words, ROM_START, 4)) },
                ROM_START,
                0,
                1
            )
            .is_err()
        );
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
                        Ok(response(words, ROM_START, offset as u32))
                    },
                    ROM_START,
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
            ROM_START,
            0x800000,
        )
        .unwrap();
        for bad in [
            vec![],
            vec![0, ID, ACK, ROM_START + 4, 8],
            vec![0, ID, ACK, ROM_START, 12],
        ] {
            assert!(begin_transfer(|_| Ok(bad), ROM_START, 8).is_err());
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
            (
                vec![0, 0x5244_4d01, 0, 0, 0],
                "replied with an older dumper",
            ),
            (vec![0, ID, ACK, ROM_START, 4], "identified as v3"),
            (vec![u32::MAX; 5], "did not return the dumper v3 identity"),
            (vec![], "did not return the dumper v3 identity"),
        ] {
            let received = format!("got {reply:08x?}");
            let error = begin_transfer(|_| Ok(reply), ROM_START, 8)
                .unwrap_err()
                .to_string();
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
        assert!(Args::try_parse_from(["mb-dumper", "--save", "game.sav"]).is_ok());
        assert!(
            Args::try_parse_from(["mb-dumper", "--save", "--save-type", "eeprom8k", "game.sav"])
                .is_ok()
        );
        assert!(
            Args::try_parse_from(["mb-dumper", "--save", "--size", "512", "game.sav"]).is_err()
        );
        assert!(
            Args::try_parse_from([
                "mb-dumper",
                "--size",
                "512",
                "--save-type",
                "sram",
                "game.sav"
            ])
            .is_err()
        );
        assert!(
            Args::try_parse_from(["mb-dumper", "--save", "--save-type", "unknown", "game.sav"])
                .is_err()
        );
    }

    #[test]
    fn save_preparation_polls_without_restarting_the_snapshot() {
        for kind in [
            SaveType::Auto,
            SaveType::Sram,
            SaveType::Flash64,
            SaveType::Flash128,
            SaveType::Eeprom512,
            SaveType::Eeprom8k,
        ] {
            let size = kind.size().unwrap_or(131072);
            let mut script = [
                (HELLO, 0),
                (SAVE | kind as u32, ID),
                (SAVE_STATUS, ACK),
                (SAVE_STATUS, 0),
                (SAVE_STATUS, 0),
                (SAVE_STATUS, size),
            ]
            .into_iter();
            assert_eq!(
                prepare_save(
                    |word| {
                        let (expected, reply) = script.next().unwrap();
                        assert_eq!(word, expected);
                        Ok(reply)
                    },
                    kind
                )
                .unwrap(),
                size
            );
            assert!(script.next().is_none());
        }
        for (reply, expected) in [
            (0xbad0_0004, "not detected"),
            (0xbad0_0005, "EEPROM detected"),
            (0xdead_beef, "unexpected save status"),
            (512, "does not match"),
        ] {
            let mut replies = [0, ID, ACK, reply].into_iter();
            let error = prepare_save(|_| Ok(replies.next().unwrap()), SaveType::Auto).unwrap_err();
            assert!(error.to_string().contains(expected), "{error}");
        }
        let mut replies = [0, ID, ACK, 32768].into_iter();
        assert!(prepare_save(|_| Ok(replies.next().unwrap()), SaveType::Flash128).is_err());
        let mut replies = [0, 0x5244_4d02].into_iter();
        assert!(
            prepare_save(|_| Ok(replies.next().unwrap()), SaveType::Auto)
                .unwrap_err()
                .to_string()
                .contains("requires the GBA dumper v3")
        );
        assert!(prepare_save(|_| Err(mb_host::Error::Cancelled), SaveType::Auto).is_err());
    }

    #[test]
    fn save_stream_preserves_snapshot_and_checks_save_addresses() {
        begin_transfer(
            |words| {
                assert_eq!(words, [0, BEGIN, SAVE_START, SAVE_SIZE_MAX, 0]);
                Ok(vec![0, ID, ACK, SAVE_START, SAVE_SIZE_MAX])
            },
            SAVE_START,
            SAVE_SIZE_MAX,
        )
        .unwrap();
        for offset in [0, 65532, SAVE_SIZE_MAX - 4] {
            let bytes = read_block(
                |words| Ok(response(words, SAVE_START, offset)),
                SAVE_START,
                offset,
                1,
            )
            .unwrap();
            assert_eq!(bytes, ((SAVE_START + offset) ^ 0x89ab_cdef).to_le_bytes());
        }
        assert!(read_block(|words| Ok(response(words, ROM_START, 0)), SAVE_START, 0, 1).is_err());
        assert!(
            read_block(
                |_| panic!("out of bounds"),
                SAVE_START,
                SAVE_SIZE_MAX - 4,
                2
            )
            .is_err()
        );
    }
}
