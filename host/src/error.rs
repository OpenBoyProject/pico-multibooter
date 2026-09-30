use protocol::{Command, ErrorCode};
use std::{fmt, io};

#[derive(Debug)]
pub enum Error {
    Io {
        operation: &'static str,
        source: io::Error,
    },
    Protocol {
        command: Command,
        source: protocol::Error,
    },
    Device {
        command: Command,
        code: ErrorCode,
    },
    Timeout {
        command: Command,
    },
    Cancelled,
    SessionLost,
    InvalidTimeout,
    InvalidWordCount(usize),
    BulkCapacity {
        requested: usize,
        maximum: usize,
    },
    NoCable,
    AmbiguousPorts(Vec<String>),
}

impl Error {
    pub(crate) fn io(operation: &'static str, source: impl Into<io::Error>) -> Self {
        Self::Io {
            operation,
            source: source.into(),
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io { operation, source } => write!(f, "{operation}: {source}"),
            Self::Protocol { command, source } => write!(
                f,
                "invalid {command:?} reply: {source}; reconnect before another operation"
            ),
            Self::Timeout { command } => write!(
                f,
                "{command:?} timed out; delivery is uncertain, so the request was not retried; reconnect before another operation"
            ),
            Self::Cancelled => f.write_str("cancelled; reopen the cable before another operation"),
            Self::SessionLost => {
                f.write_str("cable session is closed or failed; open a new connection")
            }
            Self::InvalidTimeout => {
                f.write_str("request timeouts must be nonzero and fit the monotonic clock")
            }
            Self::InvalidWordCount(count) => {
                write!(f, "bulk transfer requires 1-256 words, received {count}")
            }
            Self::BulkCapacity { requested, maximum } => write!(
                f,
                "requested {requested} words, but this cable supports {maximum} per bulk transfer"
            ),
            Self::NoCable => f.write_str(
                "no Pico GBA Multibooter port found; use `list --all`, then specify --port",
            ),
            Self::AmbiguousPorts(ports) => write!(
                f,
                "multiple Pico cables found ({}); specify --port",
                ports.join(", ")
            ),
            Self::Device { command, code } => {
                let reason = match code {
                    ErrorCode::InvalidRequest => {
                        "invalid request, or command unsupported by this firmware"
                    }
                    ErrorCode::TransferFailed => "SPI transfer failed",
                    ErrorCode::Disconnected => "USB session was disconnected",
                    ErrorCode::InvalidFrame => "firmware rejected a corrupt or oversized frame",
                };
                write!(
                    f,
                    "{command:?}: {reason} (status {}); reconnect before another operation",
                    *code as u8
                )
            }
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            Self::Protocol { source, .. } => Some(source),
            _ => None,
        }
    }
}
