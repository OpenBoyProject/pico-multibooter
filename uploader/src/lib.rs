//! GBA BIOS multiboot sender running on the PC, using raw PMB3 bulk exchanges.
//!
//! ```no_run
//! use mb_host::{Cable, list_ports, select_port};
//! use mb_uploader::{Rom, upload_rom};
//!
//! let rom = Rom::load("game.gba")?;
//! let mut cable = Cable::open(&select_port(&list_ports(false)?)?)?;
//! upload_rom(&mut cable, &rom, |progress| eprintln!("{progress:?}"))?;
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```

mod error;
mod multiboot;
mod rom;

pub use error::Error;
pub use multiboot::{UploadPhase, UploadProgress, upload_rom};
pub use rom::{MAX_ROM_SIZE, MIN_ROM_SIZE, ROM_HEADER_LEN, Rom};
