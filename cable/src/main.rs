use clap::{Parser, Subcommand};
use mb_host::{Cable, Cancellation, Error, Options, list_ports, select_port};
use std::{
    io::{self, Write},
    process::ExitCode,
    time::{Duration, Instant},
};

#[derive(Parser)]
#[command(
    name = "mb-cable",
    version,
    about = "Inspect a Pico cable and exchange application words"
)]
struct Cli {
    /// Serial port (auto-selects a single Pico GBA Multibooter device if omitted).
    #[arg(short, long, global = true, value_name = "PORT")]
    port: Option<String>,
    #[command(subcommand)]
    command: Action,
}

#[derive(Subcommand)]
enum Action {
    /// List ports whose product name matches Pico GBA Multibooter.
    List {
        /// List every serial port, including devices with other product names.
        #[arg(long)]
        all: bool,
    },
    /// Connect and verify the cable's PMB3 identity without clocking the GBA.
    Info,
    /// Exchange exactly one 32-bit word.
    Exchange {
        #[arg(value_parser = parse_word)]
        word: u32,
    },
    /// Repeatedly send a word on one connection until a masked reply matches.
    Poll {
        #[arg(value_parser = parse_word)]
        word: u32,
        #[arg(long, value_parser = parse_word)]
        expect: u32,
        #[arg(long, default_value = "0xffffffff", value_parser = parse_word)]
        mask: u32,
        /// Maximum polling time in seconds; an outstanding USB request may finish later.
        #[arg(long, default_value = "10", value_parser = clap::value_parser!(u32).range(1..=300))]
        timeout: u32,
        /// Words per USB request. A matching word does not stop the rest of its batch.
        #[arg(long, default_value = "16", value_parser = clap::value_parser!(u16).range(1..=256))]
        batch: u16,
    },
    /// Exchange up to 256 words with a running custom ROM.
    BulkExchange {
        #[arg(value_parser = parse_word, num_args = 1..=256, required = true)]
        words: Vec<u32>,
    },
    /// Clock incoming words while repeatedly sending a fill word.
    BulkRead {
        #[arg(value_parser = clap::value_parser!(u16).range(1..=256))]
        count: u16,
        #[arg(long, default_value = "0", value_parser = parse_word)]
        fill: u32,
    },
    /// Write raw application words and discard incoming words.
    BulkWrite {
        #[arg(value_parser = parse_word, num_args = 1..=256, required = true)]
        words: Vec<u32>,
    },
}

fn parse_word(text: &str) -> Result<u32, String> {
    let value = if let Some(hex) = text.strip_prefix("0x").or_else(|| text.strip_prefix("0X")) {
        u32::from_str_radix(hex, 16)
    } else {
        text.parse()
    };
    value.map_err(|_| format!("{text:?} is not a u32 (use decimal or 0x-prefixed hex)"))
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let cancellation = Cancellation::default();
    let signal = cancellation.clone();
    if let Err(error) = ctrlc::set_handler(move || signal.cancel()) {
        eprintln!("error: cannot install Ctrl-C handler: {error}");
        return ExitCode::FAILURE;
    }
    match run(cli, cancellation) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("\nerror: {error}");
            if matches!(error, Error::Cancelled) {
                ExitCode::from(130)
            } else {
                ExitCode::FAILURE
            }
        }
    }
}

fn run(cli: Cli, cancellation: Cancellation) -> Result<(), Error> {
    if let Action::List { all } = cli.command {
        let ports = list_ports(all)?;
        let mut output = io::stdout().lock();
        if ports.is_empty() {
            let message = if all {
                "No serial ports found."
            } else {
                "No Pico GBA Multibooter ports found; use list --all to see every port."
            };
            writeln!(output, "{message}").map_err(stdout_error)?;
        }
        for port in ports {
            let marker = if port.is_candidate() {
                " [Pico candidate]"
            } else {
                ""
            };
            writeln!(
                output,
                "{}\t{}{}",
                port.name,
                port.product.as_deref().unwrap_or("Unknown device"),
                marker
            )
            .map_err(stdout_error)?;
        }
        return Ok(());
    }
    let port = match cli.port {
        Some(port) => port,
        None => select_port(&list_ports(false)?)?,
    };
    eprintln!("Connecting to {port}...");
    let mut cable = Cable::open_with_options(
        &port,
        Options {
            cancellation,
            ..Options::default()
        },
    )?;
    match cli.command {
        Action::List { .. } => unreachable!(),
        Action::Info => {
            let capacity = cable.bulk_capacity()?;
            writeln!(
                io::stdout().lock(),
                "PICO-MB3 on {port}: {capacity} words ({} bytes) per bulk transfer",
                capacity * 4,
            )
            .map_err(stdout_error)?;
        }
        Action::Exchange { word } => print_words(&[cable.exchange(word)?])?,
        Action::Poll {
            word,
            expect,
            mask,
            timeout,
            batch,
        } => {
            let count = usize::from(batch).min(cable.bulk_capacity()?);
            eprintln!("Polling 0x{word:08x} for up to {timeout}s, {count} words per request...");
            poll(
                || cable.bulk_read(count, word),
                expect,
                mask,
                Duration::from_secs(u64::from(timeout)),
                &mut io::stdout().lock(),
            )?;
        }
        Action::BulkExchange { words } => {
            print_words(&cable.bulk_exchange(&words)?)?;
        }
        Action::BulkRead { count, fill } => {
            print_words(&cable.bulk_read(usize::from(count), fill)?)?;
        }
        Action::BulkWrite { words } => cable.bulk_write(&words)?,
    }
    cable.close()
}

fn poll(
    mut exchange: impl FnMut() -> Result<Vec<u32>, Error>,
    expect: u32,
    mask: u32,
    timeout: Duration,
    output: &mut impl Write,
) -> Result<(), Error> {
    let start = Instant::now();
    let mut total = 0usize;
    let mut first = None;
    let mut last = None;
    while start.elapsed() < timeout {
        let words = exchange()?; // Propagate uncertain transfers; never replay them.
        let previous_total = total;
        total += words.len();
        for (index, &word) in words.iter().enumerate() {
            first.get_or_insert(word);
            last = Some(word);
            if previous_total + index < 16 {
                writeln!(output, "RX[{}] = 0x{word:08x}", previous_total + index)
                    .map_err(stdout_error)?;
            }
            if word & mask == expect & mask {
                writeln!(
                    output,
                    "Matched 0x{word:08x}; {total} words clocked in {:.3}s",
                    start.elapsed().as_secs_f64()
                )
                .map_err(stdout_error)?;
                return Ok(());
            }
        }
    }
    Err(Error::Io {
        operation: "poll response pattern",
        source: io::Error::new(
            io::ErrorKind::TimedOut,
            format!(
                "no match for 0x{expect:08x} with mask 0x{mask:08x} after {total} words; first={first:08x?}, last={last:08x?}"
            ),
        ),
    })
}

fn print_words(words: &[u32]) -> Result<(), Error> {
    let mut output = io::stdout().lock();
    for word in words {
        writeln!(output, "0x{word:08x}").map_err(stdout_error)?;
    }
    Ok(())
}

fn stdout_error(source: io::Error) -> Error {
    Error::Io {
        operation: "write stdout",
        source,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn cli_contract_and_word_parsing() {
        Cli::command().debug_assert();
        assert!(matches!(
            Cli::try_parse_from(["cable", "list"]).unwrap().command,
            Action::List { all: false }
        ));
        assert!(matches!(
            Cli::try_parse_from(["cable", "list", "--all"])
                .unwrap()
                .command,
            Action::List { all: true }
        ));
        assert!(Cli::try_parse_from(["cable", "info", "--port", "COM3"]).is_ok());
        assert!(Cli::try_parse_from(["cable", "bulk-exchange", "0x12345678", "42"]).is_ok());
        assert!(Cli::try_parse_from(["cable", "bulk-read", "256"]).is_ok());
        for args in [
            vec!["exchange"],
            vec!["exchange", "-1"],
            vec!["exchange", "0x100000000"],
            vec!["bulk-read", "0"],
            vec!["bulk-read", "257"],
            vec!["exchange", "1", "2"],
            vec!["bulk-write"],
            vec!["bulk-exchange"],
            vec!["exchange", "--fast", "1"],
            vec!["poll", "0x6200"],
            vec!["poll", "0x6200", "--expect", "0", "--batch", "0"],
            vec!["poll", "0x6200", "--expect", "0", "--timeout", "0"],
        ] {
            assert!(Cli::try_parse_from(std::iter::once("cable").chain(args)).is_err());
        }
        assert!(
            Cli::try_parse_from([
                "cable",
                "poll",
                "0x6200",
                "--expect",
                "0x72020000",
                "--mask",
                "0xffff0000"
            ])
            .is_ok()
        );
        assert_eq!(parse_word("0Xffffffff"), Ok(u32::MAX));
        assert_eq!(parse_word("42"), Ok(42));
    }
    #[test]
    fn polling_matches_mask_and_stops_before_another_batch() {
        let mut batches = vec![vec![0xffffffff, 0], vec![0x72026200, 0x72026200]].into_iter();
        let mut output = Vec::new();
        poll(
            || Ok(batches.next().expect("extra request")),
            0x72020000,
            0xffff0000,
            Duration::from_secs(1),
            &mut output,
        )
        .unwrap();
        let output = String::from_utf8(output).unwrap();
        assert!(output.contains("Matched 0x72026200; 4 words clocked"));
        assert!(batches.next().is_none());
    }

    #[test]
    fn polling_propagates_transport_failures_and_honors_deadline() {
        let mut calls = 0;
        assert!(matches!(
            poll(
                || {
                    calls += 1;
                    Err(Error::SessionLost)
                },
                0,
                u32::MAX,
                Duration::from_secs(1),
                &mut Vec::new()
            ),
            Err(Error::SessionLost)
        ));
        assert_eq!(calls, 1);
        assert!(
            matches!(poll(|| panic!("expired poll must not clock words"),
            0, u32::MAX, Duration::ZERO, &mut Vec::new()), Err(Error::Io { source, .. })
            if source.kind() == io::ErrorKind::TimedOut)
        );
    }
}
