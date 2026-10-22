//! The crate's error type.

use std::io;

/// What can go wrong talking to a SAM bridge.
#[derive(Debug)]
#[non_exhaustive]
pub enum Error {
    /// The socket to the bridge failed or closed.
    Io(io::Error),
    /// The bridge answered with something that is not the expected reply.
    Protocol(String),
    /// The bridge refused: its `RESULT` value (`CANT_REACH_PEER`, `DUPLICATED_DEST`,
    /// `INVALID_KEY`, `TIMEOUT`, `I2P_ERROR`, ...) and `MESSAGE`, if any.
    Refused {
        /// The `RESULT` value.
        result: String,
        /// The `MESSAGE` value.
        message: Option<String>,
    },
    /// A reply line exceeded the bound this client reads.
    LineTooLong,
    /// A value given to this client cannot be written onto a SAM line.
    InvalidInput(&'static str),
}

impl Error {
    /// The bridge's `RESULT` value, for a refusal.
    pub fn result(&self) -> Option<&str> {
        match self {
            Self::Refused { result, .. } => Some(result),
            _ => None,
        }
    }
}

impl From<io::Error> for Error {
    fn from(e: io::Error) -> Self {
        Self::Io(e)
    }
}

impl From<Error> for io::Error {
    fn from(e: Error) -> Self {
        match e {
            Error::Io(e) => e,
            Error::InvalidInput(_) => io::Error::new(io::ErrorKind::InvalidInput, e),
            Error::Refused { ref result, .. } if result == "TIMEOUT" => {
                io::Error::new(io::ErrorKind::TimedOut, e)
            }
            Error::Refused { ref result, .. }
                if result == "CANT_REACH_PEER" || result == "PEER_NOT_FOUND" =>
            {
                io::Error::new(io::ErrorKind::HostUnreachable, e)
            }
            other => io::Error::other(other),
        }
    }
}

impl core::fmt::Display for Error {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "SAM bridge i/o: {e}"),
            Self::Protocol(line) => write!(f, "unexpected SAM reply: {line}"),
            Self::Refused {
                result,
                message: Some(message),
            } => write!(f, "SAM bridge refused: {result} ({message})"),
            Self::Refused { result, .. } => write!(f, "SAM bridge refused: {result}"),
            Self::LineTooLong => write!(f, "SAM reply line too long"),
            Self::InvalidInput(why) => write!(f, "invalid SAM input: {why}"),
        }
    }
}

impl core::error::Error for Error {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        match self {
            Self::Io(e) => Some(e),
            _ => None,
        }
    }
}
