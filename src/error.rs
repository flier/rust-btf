#[cfg(feature = "std")]
use thiserror::Error;

#[cfg_attr(feature = "std", derive(Error))]
#[derive(Debug)]
pub enum Error {
    #[cfg_attr(feature = "std", error("unexpected end of input"))]
    EndOfInput,

    #[cfg_attr(feature = "std", error("incomplete BTF file"))]
    Incomplete(&'static str),

    #[cfg_attr(feature = "std", error("malformed BTF file, {0}"))]
    Malformed(&'static str),

    #[cfg_attr(feature = "std", error("offset {0} out of range"))]
    OutOfRange(&'static str, u64),

    #[cfg_attr(feature = "std", error("unexpected {0}"))]
    Unexpected(&'static str),

    #[cfg_attr(feature = "std", error("expected {0}"))]
    Expected(&'static str),

    #[cfg_attr(feature = "std", error(transparent))]
    FmtError(#[cfg_attr(feature = "std", from)] core::fmt::Error),

    #[cfg_attr(feature = "std", error(transparent))]
    Utf8Error(#[cfg_attr(feature = "std", from)] core::str::Utf8Error),

    #[cfg(feature = "std")]
    #[error("read file")]
    IO(#[from] std::io::Error),
}

impl From<untrusted::EndOfInput> for Error {
    fn from(_: untrusted::EndOfInput) -> Self {
        Error::EndOfInput
    }
}
