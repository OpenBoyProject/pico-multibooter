//! Typed PMB3 payloads. All multibyte fields use little-endian byte order.

use crate::{Error, Frame, IDENTITY, MAX_BULK_WORDS, RESPONSE_BIT, frame::encode_parts};

/// Operation byte in a PC-to-Pico frame. These values are part of PMB3.
/// GBA commands travel as data; the Pico does not interpret them.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum Command {
    // Read = 0x01,
    // Write = 0x02,
    Exchange = 0x03,
    BulkRead = 0x05,
    BulkWrite = 0x06,
    BulkExchange = 0x07,
    Info = 0x08,
}

impl Command {
    /// Reply opcode, for example Exchange (0x03) becomes 0x83.
    pub const fn response_byte(self) -> u8 {
        self as u8 | RESPONSE_BIT
    }
}

impl TryFrom<u8> for Command {
    type Error = Error;
    fn try_from(value: u8) -> Result<Self, Error> {
        match value {
            0x03 => Ok(Self::Exchange),
            0x05 => Ok(Self::BulkRead),
            0x06 => Ok(Self::BulkWrite),
            0x07 => Ok(Self::BulkExchange),
            0x08 => Ok(Self::Info),
            _ => Err(Error::UnknownCommand(value)),
        }
    }
}

/// Cable failures. Successful replies use a zero status byte.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum ErrorCode {
    InvalidRequest = 1,
    TransferFailed = 2,
    Disconnected = 3,
    InvalidFrame = 4,
}

impl TryFrom<u8> for ErrorCode {
    type Error = Error;
    fn try_from(value: u8) -> Result<Self, Error> {
        match value {
            1 => Ok(Self::InvalidRequest),
            2 => Ok(Self::TransferFailed),
            3 => Ok(Self::Disconnected),
            4 => Ok(Self::InvalidFrame),
            _ => Err(Error::UnknownStatus(value)),
        }
    }
}

/// Borrowed, unaligned little-endian words for an application exchange.
/// USB byte order is independent of the GBA SPI bit/byte order.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Words<'a>(&'a [u8]);

impl<'a> Words<'a> {
    /// Borrow 1..=256 words without copying or requiring aligned storage.
    pub fn from_le_bytes(bytes: &'a [u8]) -> Result<Self, Error> {
        if bytes.is_empty() || bytes.len() > MAX_BULK_WORDS * 4 || !bytes.len().is_multiple_of(4) {
            return Err(Error::InvalidWordLength);
        }
        Ok(Self(bytes))
    }

    /// Serialize native words into caller-owned storage for an outgoing exchange.
    pub fn from_words(words: &[u32], buffer: &'a mut [u8]) -> Result<Self, Error> {
        if words.is_empty() || words.len() > MAX_BULK_WORDS {
            return Err(Error::InvalidWordLength);
        }
        let needed = words.len() * 4;
        if buffer.len() < needed {
            return Err(Error::BufferTooSmall { needed });
        }
        for (word, bytes) in words.iter().zip(buffer[..needed].as_chunks_mut::<4>().0) {
            bytes.copy_from_slice(&word.to_le_bytes());
        }
        Ok(Self(&buffer[..needed]))
    }

    pub fn as_le_bytes(self) -> &'a [u8] {
        self.0
    }

    pub fn iter(self) -> impl ExactSizeIterator<Item = u32> + 'a {
        self.0
            .as_chunks::<4>()
            .0
            .iter()
            .map(|bytes| u32::from_le_bytes(*bytes))
    }
}

/// A command and its arguments. Word data borrows the caller's buffer.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Request<'a> {
    Info,
    /// Exactly one word.
    Exchange(u32),
    /// Receive `count` words while repeatedly sending `fill`.
    BulkRead {
        count: u16,
        fill: u32,
    },
    /// Send words unchanged and discard received words.
    BulkWrite {
        words: Words<'a>,
    },
    BulkExchange {
        words: Words<'a>,
    },
}

impl<'a> Request<'a> {
    pub const fn command(self) -> Command {
        match self {
            Self::Info => Command::Info,
            Self::Exchange(_) => Command::Exchange,
            Self::BulkRead { .. } => Command::BulkRead,
            Self::BulkWrite { .. } => Command::BulkWrite,
            Self::BulkExchange { .. } => Command::BulkExchange,
        }
    }

    /// Words validates itself on construction; read counts need a separate check.
    pub fn validate(self) -> Result<(), Error> {
        if let Self::BulkRead { count, .. } = self
            && (count == 0 || usize::from(count) > MAX_BULK_WORDS)
        {
            return Err(Error::InvalidWordLength);
        }
        Ok(())
    }

    /// Validate a request payload. Use Decoder first to check the frame CRC.
    pub fn decode(frame: Frame<'a>) -> Result<Self, Error> {
        let payload = frame.payload;
        let request = match Command::try_from(frame.command)? {
            Command::Info if payload.is_empty() => Self::Info,
            Command::Exchange if payload.len() == 4 => Self::Exchange(read_u32(payload)),
            Command::BulkRead if payload.len() == 6 => Self::BulkRead {
                count: u16::from_le_bytes(payload[..2].try_into().unwrap()),
                fill: read_u32(&payload[2..6]),
            },
            command @ (Command::BulkWrite | Command::BulkExchange) if !payload.is_empty() => {
                let words = Words::from_le_bytes(payload)?;
                if command == Command::BulkWrite {
                    Self::BulkWrite { words }
                } else {
                    Self::BulkExchange { words }
                }
            }
            _ => return Err(Error::InvalidPayload),
        };
        request.validate()?;
        Ok(request)
    }

    /// Encode a complete frame without modifying output on error.
    pub fn encode(self, output: &mut [u8]) -> Result<usize, Error> {
        self.validate()?;
        let command = self.command() as u8;
        match self {
            Self::Exchange(word) => encode_parts(command, &[&word.to_le_bytes()], output),
            Self::BulkWrite { words } | Self::BulkExchange { words } => {
                encode_parts(command, &[words.as_le_bytes()], output)
            }
            Self::BulkRead { count, fill } => encode_parts(
                command,
                &[&count.to_le_bytes(), &fill.to_le_bytes()],
                output,
            ),
            Self::Info => encode_parts(command, &[], output),
        }
    }
}

/// Cable limits returned with the fixed protocol identity. No GBA state is queried.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DeviceInfo {
    /// Maximum 32-bit words per bulk read, write, or exchange.
    pub bulk_capacity: u32,
}

/// Reply body. Encoding adds the status byte and response opcode.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Response<'a> {
    Info(DeviceInfo),
    Exchange(u32),
    /// Number of bytes clocked, not an acknowledgement from the GBA.
    Written(u32),
    BulkData(Words<'a>),
    Error(ErrorCode),
}

impl<'a> Response<'a> {
    /// Check the reply command and data length against the outstanding request.
    /// PMB3 has no request IDs; the transport must process requests sequentially.
    pub fn decode_for(request: Request<'_>, frame: Frame<'a>) -> Result<Self, Error> {
        request.validate()?;
        let expected = request.command().response_byte();
        if frame.command != expected {
            return Err(Error::UnexpectedCommand {
                expected,
                received: frame.command,
            });
        }
        let (&status, payload) = frame.payload.split_first().ok_or(Error::InvalidPayload)?;
        let response = if status != 0 {
            let code = ErrorCode::try_from(status)?;
            if !payload.is_empty() {
                return Err(Error::InvalidPayload);
            }
            Self::Error(code)
        } else {
            match request {
                Request::Info if payload.len() == 12 && &payload[..8] == IDENTITY => {
                    Self::Info(DeviceInfo {
                        bulk_capacity: read_u32(&payload[8..]),
                    })
                }
                Request::Exchange(_) if payload.len() == 4 => Self::Exchange(read_u32(payload)),
                Request::BulkWrite { .. } if payload.len() == 4 => Self::Written(read_u32(payload)),
                Request::BulkRead { .. } | Request::BulkExchange { .. } => {
                    Self::BulkData(Words::from_le_bytes(payload)?)
                }
                _ => return Err(Error::InvalidPayload),
            }
        };
        response.validate_for(request)?;
        Ok(response)
    }

    fn validate_for(self, request: Request<'_>) -> Result<(), Error> {
        request.validate()?;
        match (self, request) {
            (Self::Error(_), _) => {}
            (Self::Info(info), Request::Info)
                if info.bulk_capacity > 0 && info.bulk_capacity <= MAX_BULK_WORDS as u32 => {}
            (Self::Written(count), Request::BulkWrite { words, .. })
                if count == words.as_le_bytes().len() as u32 => {}
            (Self::Exchange(_), Request::Exchange(_)) => {}
            (Self::BulkData(reply), Request::BulkExchange { words, .. })
                if reply.0.len() == words.0.len() => {}
            (Self::BulkData(reply), Request::BulkRead { count, .. })
                if reply.0.len() == usize::from(count) * 4 => {}
            _ => return Err(Error::InvalidPayload),
        }
        Ok(())
    }

    /// Encode a checked reply. Use Frame::encode to reject an unknown command.
    pub fn encode_for(self, request: Request<'_>, output: &mut [u8]) -> Result<usize, Error> {
        self.validate_for(request)?;
        let command = request.command().response_byte();
        match self {
            Self::Info(info) => encode_parts(
                command,
                &[&[0], IDENTITY, &info.bulk_capacity.to_le_bytes()],
                output,
            ),
            Self::Written(value) | Self::Exchange(value) => {
                encode_parts(command, &[&[0], &value.to_le_bytes()], output)
            }
            Self::BulkData(words) => encode_parts(command, &[&[0], words.as_le_bytes()], output),
            Self::Error(code) => encode_parts(command, &[&[code as u8]], output),
        }
    }
}

// Only called after validating an exact four-byte length.
fn read_u32(bytes: &[u8]) -> u32 {
    u32::from_le_bytes(bytes.try_into().unwrap())
}
