#![no_std]

//! PC <-> Pico PMB3 serial protocol.
//!
//! This crate handles framing and messages, not the GBA BIOS multiboot algorithm.
//! It uses no allocator, operating system, USB driver, or hardware dependencies.
//! See the repository's `protocol/README.md` for the wire format and session rules.
//!
//! [`Frame`] holds raw command and payload bytes. [`Command`] identifies the
//! Pico operation; [`Request`] and [`Response`] describe its payload.
//!
//! USB packets may split or combine frames. Feed every received byte to a
//! [`Decoder`], retaining it between reads. Process its borrowed [`Frame`] before
//! feeding the next byte. Reset the decoder on disconnect or a framing timeout.
//!
//! ```
//! use protocol::{Decoder, DeviceInfo, Request, Response, MAX_FRAME};
//!
//! let mut output = [0; MAX_FRAME];
//! let count = Request::Info.encode(&mut output)?;
//! let mut decoder = Decoder::new();
//! let mut reply = [0; MAX_FRAME];
//! for &byte in &output[..count] {
//!     if let Some(result) = decoder.push(byte) {
//!         let request = Request::decode(result?)?;
//!         assert_eq!(request, Request::Info);
//!         let reply_len = Response::Info(DeviceInfo { bulk_capacity: 256 }).encode_for(request, &mut reply)?;
//!         // Send reply[..reply_len] using your serial/USB transport.
//!         assert_eq!(reply_len, 24);
//!     }
//! }
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```

mod frame;
mod message;

pub use frame::{DecodeError, Decoder, Frame, crc32};
pub use message::{Command, DeviceInfo, ErrorCode, Request, Response, Words};

/// Frame synchronization marker; the version is part of the marker.
pub const MAGIC: &[u8; 4] = b"PMB3";
/// Identity prefix in a successful INFO reply, followed by the bulk word limit.
pub const IDENTITY: &[u8; 8] = b"PICO-MB3";
/// Set on reply command bytes, clear on request command bytes.
pub const RESPONSE_BIT: u8 = 0x80;
/// Largest payload: reply status (1), followed by bulk word data.
pub const MAX_PAYLOAD: usize = 1 + MAX_DATA_LEN;
/// Magic (4), command (1), length (2), and CRC (4).
pub const FRAME_OVERHEAD: usize = 11;
pub const MAX_FRAME: usize = MAX_PAYLOAD + FRAME_OVERHEAD;
pub const MAX_DATA_LEN: usize = MAX_BULK_WORDS * 4;
/// Maximum number of 32-bit words in one bulk request or reply.
pub const MAX_BULK_WORDS: usize = 256;

/// Local encoding/decoding errors, distinct from device-reported [`ErrorCode`]s.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error {
    PayloadTooLarge,
    BufferTooSmall { needed: usize },
    ChecksumMismatch { expected: u32, received: u32 },
    UnknownCommand(u8),
    UnexpectedCommand { expected: u8, received: u8 },
    UnknownStatus(u8),
    InvalidPayload,
    InvalidWordLength,
}

impl core::fmt::Display for Error {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::PayloadTooLarge => f.write_str("payload exceeds the PMB3 limit"),
            Self::BufferTooSmall { needed } => write!(f, "buffer requires {needed} bytes"),
            Self::ChecksumMismatch { expected, received } => {
                write!(
                    f,
                    "CRC mismatch: expected {expected:08x}, received {received:08x}"
                )
            }
            Self::UnknownCommand(command) => write!(f, "unknown request command {command:#04x}"),
            Self::UnexpectedCommand { expected, received } => {
                write!(
                    f,
                    "expected reply command {expected:#04x}, received {received:#04x}"
                )
            }
            Self::UnknownStatus(status) => write!(f, "unknown error status {status}"),
            Self::InvalidPayload => f.write_str("invalid payload for this command"),
            Self::InvalidWordLength => {
                f.write_str("word data must contain 1..=256 complete u32 words")
            }
        }
    }
}

impl core::error::Error for Error {}
