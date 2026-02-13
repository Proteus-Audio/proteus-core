use std::fmt::{Display, Formatter};

/// Error type for peak extraction and binary peak-file IO.
#[derive(Debug)]
pub enum PeaksError {
    Io(std::io::Error),
    Decode(String),
    InvalidFormat(String),
}

impl Display for PeaksError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(err) => write!(f, "io error: {}", err),
            Self::Decode(err) => write!(f, "decode error: {}", err),
            Self::InvalidFormat(err) => write!(f, "invalid peaks format: {}", err),
        }
    }
}

impl std::error::Error for PeaksError {}

impl From<std::io::Error> for PeaksError {
    fn from(value: std::io::Error) -> Self {
        Self::Io(value)
    }
}
