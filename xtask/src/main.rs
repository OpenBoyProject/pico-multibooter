use clap::{Parser, Subcommand, ValueEnum};
use std::{
    io,
    path::Path,
    process::{Command, ExitCode},
};

#[derive(Parser)]
#[command(about = "Workspace build tasks")]
struct Args {
    #[command(subcommand)]
    command: Task,
}

#[derive(Subcommand)]
enum Task {
    /// Build GBA multiboot ROMs with devkitPro and Make.
    Gba {
        #[arg(value_enum, default_value = "all")]
        target: GbaTarget,
    },
}

#[derive(Clone, Copy, ValueEnum)]
enum GbaTarget {
    All,
    HelloWorld,
    RomDumper,
}

fn main() -> ExitCode {
    let Args { command } = Args::parse();
    let result = match command {
        Task::Gba { target } => build_gba(target),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::FAILURE
        }
    }
}

fn build_gba(target: GbaTarget) -> io::Result<()> {
    let targets: &[&str] = match target {
        GbaTarget::All => &["hello-world", "rom-dumper"],
        GbaTarget::HelloWorld => &["hello-world"],
        GbaTarget::RomDumper => &["rom-dumper"],
    };
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    for target in targets {
        let directory = root.join("gba").join(target);
        let status = Command::new("make")
            .arg("-C")
            .arg(&directory)
            .arg("all")
            .status()
            .map_err(|error| io::Error::new(error.kind(), format!("cannot run make: {error}")))?;
        if !status.success() {
            return Err(io::Error::other(format!(
                "{target} build failed ({status}); check devkitPro activation and the build output"
            )));
        }
        println!(
            "ROM: {}",
            directory
                .join("build")
                .join(format!("{target}_mb.gba"))
                .display()
        );
    }
    Ok(())
}
