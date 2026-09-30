//! Frame layout: magic (4), command (1), payload length (2), payload, CRC (4).
//! Length and CRC are little endian. CRC covers command, length, and payload.

use crate::{Error, FRAME_OVERHEAD, MAGIC, MAX_FRAME, MAX_PAYLOAD};

/// A CRC-verified decoded frame, or a raw frame to encode.
///
/// Unknown command bytes are preserved so firmware can return an invalid-request
/// reply. Typed message validation is separate from framing.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Frame<'a> {
    /// Request opcode, or opcode | RESPONSE_BIT for a reply.
    pub command: u8,
    /// Reply payloads start with a status byte; request payloads do not.
    pub payload: &'a [u8],
}

impl Frame<'_> {
    /// Encode a frame. The output is unchanged on error.
    pub fn encode(self, output: &mut [u8]) -> Result<usize, Error> {
        encode_parts(self.command, &[self.payload], output)
    }
}

// Join payload slices directly into the frame without a temporary allocation.
pub(crate) fn encode_parts(
    command: u8,
    parts: &[&[u8]],
    output: &mut [u8],
) -> Result<usize, Error> {
    let mut length = 0usize;
    for part in parts {
        length = length
            .checked_add(part.len())
            .ok_or(Error::PayloadTooLarge)?;
        if length > MAX_PAYLOAD {
            return Err(Error::PayloadTooLarge);
        }
    }
    let needed = length + FRAME_OVERHEAD;
    if output.len() < needed {
        return Err(Error::BufferTooSmall { needed });
    }
    // All checks precede writes so an error leaves the caller's buffer intact.
    output[..4].copy_from_slice(MAGIC);
    output[4] = command;
    output[5..7].copy_from_slice(&(length as u16).to_le_bytes());
    let mut end = 7;
    for part in parts {
        output[end..end + part.len()].copy_from_slice(part);
        end += part.len();
    }
    let crc = crc32(&output[4..end]);
    output[end..needed].copy_from_slice(&crc.to_le_bytes());
    Ok(needed)
}

// Build the 1 KiB byte table at compile time; no RAM initialization is needed.
static CRC32_TABLE: [u32; 256] = {
    let mut table = [0; 256];
    let mut index = 0;
    while index < table.len() {
        let mut crc = index as u32;
        let mut bit = 0;
        while bit < 8 {
            crc = (crc >> 1) ^ (0xedb8_8320 & 0u32.wrapping_sub(crc & 1));
            bit += 1;
        }
        table[index] = crc;
        index += 1;
    }
    table
};

/// CRC-32/ISO-HDLC, matching Python's `zlib.crc32`.
pub fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = u32::MAX;
    for &byte in bytes {
        crc = (crc >> 8) ^ CRC32_TABLE[usize::from(crc as u8 ^ byte)];
    }
    !crc
}

/// A framing failure with the untrusted command byte, for an error reply.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DecodeError {
    pub command: u8,
    pub error: Error,
}

impl core::fmt::Display for DecodeError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "frame command {:#04x}: {}", self.command, self.error)
    }
}

impl core::error::Error for DecodeError {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        Some(&self.error)
    }
}

/// Incremental, fixed-buffer decoder for requests and replies.
///
/// Noise before magic is skipped. An oversized header is rejected immediately;
/// a CRC failure is rejected after the declared frame length. After either
/// error, scanning resumes at the next input byte. No commands are executed or
/// retried by this decoder. A truncated frame needs an external timeout/reset.
pub struct Decoder {
    buffer: [u8; MAX_FRAME],
    used: usize,
}

impl Default for Decoder {
    fn default() -> Self {
        Self::new()
    }
}

impl Decoder {
    pub const fn new() -> Self {
        Self {
            buffer: [0; MAX_FRAME],
            used: 0,
        }
    }

    /// Discard a partial frame, for example on timeout or disconnection.
    pub fn reset(&mut self) {
        self.used = 0;
    }

    /// Returns a frame or framing error when enough bytes have arrived.
    /// The returned frame borrows this decoder until it is processed.
    pub fn push(&mut self, byte: u8) -> Option<Result<Frame<'_>, DecodeError>> {
        if self.used < MAGIC.len() && byte != MAGIC[self.used] {
            // PMB3 has no self-overlap other than a new leading 'P'.
            self.used = usize::from(byte == MAGIC[0]);
            self.buffer[0] = byte;
            return None;
        }
        self.buffer[self.used] = byte;
        self.used += 1;
        if self.used < 7 {
            return None;
        }
        let length = u16::from_le_bytes([self.buffer[5], self.buffer[6]]) as usize;
        let command = self.buffer[4];
        if length > MAX_PAYLOAD {
            self.reset();
            return Some(Err(DecodeError {
                command,
                error: Error::PayloadTooLarge,
            }));
        }
        if self.used < length + FRAME_OVERHEAD {
            return None;
        }
        // Reset the cursor; the returned payload still borrows the buffer.
        self.reset();
        let end = 7 + length;
        let received = u32::from_le_bytes(self.buffer[end..end + 4].try_into().unwrap());
        let expected = crc32(&self.buffer[4..end]);
        if received != expected {
            return Some(Err(DecodeError {
                command,
                error: Error::ChecksumMismatch { expected, received },
            }));
        }
        Some(Ok(Frame {
            command,
            payload: &self.buffer[7..end],
        }))
    }
}
