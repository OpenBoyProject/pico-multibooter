//! Blocking PC client for the PMB3 USB cable. Each operation has a deadline;
//! ambiguous operations are never retried. Dropping a cable lowers DTR.
//!
//! ```no_run
//! use mb_host::{Cable, list_ports, select_port};
//!
//! let port = select_port(&list_ports(false)?)?;
//! let mut cable = Cable::open(&port)?;
//! // Your GBA application defines the meaning of these words.
//! let reply = cable.bulk_exchange(&[0x12345678, 0])?;
//! # Ok::<(), mb_host::Error>(())
//! ```

mod cable;
mod error;
mod serial;

pub use cable::{Cable, Cancellation, Options};
pub use error::Error;
pub use protocol::DeviceInfo;
pub use serial::{PortInfo, SerialTransport, Transport, list_ports, select_port};

#[cfg(test)]
mod tests;
