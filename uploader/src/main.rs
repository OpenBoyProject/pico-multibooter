use clap::Parser;
use mb_host::{Cable, Cancellation, Options, list_ports, select_port};
use mb_uploader::{Error, Rom, UploadPhase, upload_rom};
use std::{
    io::{self, Write},
    path::PathBuf,
    process::ExitCode,
    time::Instant,
};

#[derive(Parser)]
#[command(about = "Upload a GBA multiboot ROM through a Pico cable", version)]
struct Args {
    /// Serial port; auto-selects a single Pico GBA Multibooter if omitted.
    #[arg(short, long)]
    port: Option<String>,
    /// Multiboot-compatible ROM (400 bytes to 256 KiB); padded to 16 bytes.
    rom: PathBuf,
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

fn run(args: Args, cancellation: Cancellation) -> Result<(), Error> {
    // Validate the file before opening the device or changing its session.
    let rom = Rom::load(args.rom)?;
    let port = match args.port {
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
    if rom.original_len() != rom.as_bytes().len() {
        eprintln!(
            "Padded ROM from {} to {} bytes.",
            rom.original_len(),
            rom.as_bytes().len()
        );
    }
    let started = Instant::now();
    upload_rom(&mut cable, &rom, |progress| match progress.phase {
        UploadPhase::WaitingForGba => {
            eprintln!("Waiting for the GBA. Boot without a cartridge or hold START+SELECT.")
        }
        UploadPhase::Transferring => {
            eprint!(
                "\rTransferred {}/{} bytes ({}%)",
                progress.transferred,
                progress.total,
                progress.transferred * 100 / progress.total
            );
            let _ = io::stderr().flush();
        }
        UploadPhase::Verifying => eprintln!("\nVerifying GBA checksum..."),
        UploadPhase::Complete => eprintln!(
            "GBA verified the ROM ({:.1}s).",
            started.elapsed().as_secs_f64()
        ),
    })?;
    cable.close()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;
    #[test]
    fn cli_accepts_rom_and_optional_port() {
        Args::command().debug_assert();
        assert!(Args::try_parse_from(["mb-uploader", "game.gba"]).is_ok());
        assert!(Args::try_parse_from(["mb-uploader", "--port", "COM3", "game.gba"]).is_ok());
        assert!(Args::try_parse_from(["mb-uploader"]).is_err());
    }
}
