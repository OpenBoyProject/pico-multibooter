use std::{fmt, io};

#[derive(Debug)]
pub enum Error {
    Cable(mb_host::Error),
    Io {
        operation: &'static str,
        source: io::Error,
    },
    InvalidRomSize(usize),
    NotReady {
        first_reply: u32,
        last_reply: u32,
        probes: u32,
    },
    HandshakeFailed {
        reply: u16,
    },
    HandshakeTimeout,
    Acknowledgement {
        offset: u32,
        reply: u32,
    },
    ChecksumMismatch {
        expected: u16,
        received: u16,
    },
    ChecksumTimeout,
    InvalidReplyLength {
        expected: usize,
        received: usize,
    },
}

impl Error {
    pub(crate) fn io(operation: &'static str, source: io::Error) -> Self {
        Self::Io { operation, source }
    }
}

impl From<mb_host::Error> for Error {
    fn from(value: mb_host::Error) -> Self {
        Self::Cable(value)
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Cable(error) => error.fmt(f),
            Self::Io { operation, source } => write!(f, "{operation}: {source}"),
            Self::InvalidRomSize(size) => write!(
                f,
                "ROM is {size} bytes; expected 400–262144 bytes of a multiboot-compatible image"
            ),
            Self::NotReady {
                first_reply,
                last_reply,
                probes,
            } => write!(
                f,
                "GBA readiness timed out after {probes} probes, first RX={first_reply:#010x}, last RX={last_reply:#010x}; check wiring and boot holding START+SELECT"
            ),
            Self::HandshakeFailed { reply } => {
                write!(f, "GBA key handshake failed: received {reply:#06x}")
            }
            Self::HandshakeTimeout => f.write_str("GBA key handshake timed out"),
            Self::Acknowledgement { offset, reply } => write!(
                f,
                "GBA did not acknowledge ROM offset {offset:#010x}: RX={reply:#010x}; upload stopped without retry"
            ),
            Self::ChecksumMismatch { expected, received } => write!(
                f,
                "GBA checksum mismatch: expected {expected:#06x}, received {received:#06x}"
            ),
            Self::ChecksumTimeout => f.write_str("GBA did not become ready for the checksum"),
            Self::InvalidReplyLength { expected, received } => {
                write!(f, "expected {expected} SPI replies, received {received}")
            }
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Cable(error) => Some(error),
            Self::Io { source, .. } => Some(source),
            _ => None,
        }
    }
}
