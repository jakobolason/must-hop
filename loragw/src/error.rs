use std::fmt;

/// A common result type for this crate.
pub type Result<T = ()> = std::result::Result<T, Error>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)] // Added some helpful standard derives
pub enum BuilderError {
    MissingBoard,
    NotConnected,
    InvalidBoard,
    InvalidRxRFConf,
    InvalidTxGain,
    InvalidChain,
    InvalidChannelConf,
}

/// A common error type for this crate.
#[derive(Debug, Clone, PartialEq, Eq)] // Added some helpful standard derives
pub enum Error {
    /// Device is currently opened in same process.
    Busy,
    /// Catch-all error returned by the low-level `libloragw` c code.
    HAL,
    /// A buffer, primarily transmit payloads, is too large for the LoRa packet format.
    Size,
    /// Represents an error when attempting to convert between this crate's high-level types
    /// and those defined in `libloragw`.
    Data,
    BuilderError(BuilderError),
    Toml(toml::de::Error),
}

impl From<toml::de::Error> for Error {
    fn from(err: toml::de::Error) -> Self {
        Error::Toml(err)
    }
}

// 1. Implement Display to provide the error messages
impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Busy => write!(f, "concentrator device is already in use"),
            Error::HAL => write!(f, "concentrator HAL returned a generic error"),
            Error::Size => write!(f, "provided buffer is too large"),
            Error::Data => write!(f, "failure to convert hardware val to symbolic val"),
            Error::BuilderError(err) => match err {
                BuilderError::InvalidBoard => write!(f, "builder error: invalid board"),
                BuilderError::InvalidChain => write!(f, "builder error: invalid frequency"),
                BuilderError::InvalidChannelConf => write!(f, "builder error: invalid channel"),
                BuilderError::InvalidRxRFConf => write!(f, "builder error: invalid data rate"),
                BuilderError::InvalidTxGain => write!(f, "builder error: invalid tx gain"),
                BuilderError::MissingBoard => write!(f, "builder error: missing board"),
                BuilderError::NotConnected => write!(f, "builder error: not connected"),
            },
            Error::Toml(_err) => write!(f, "Error from toml"),
        }
    }
}

// 2. Implement the Error trait (requires Debug and Display).
// The body can be left completely empty in modern Rust!
impl std::error::Error for Error {}

pub enum AppError {
    Concentrator(Error),
    IO(std::io::Error),
    Config(toml::de::Error),
    Generic(String),
}
impl From<Error> for AppError {
    fn from(err: Error) -> Self {
        AppError::Concentrator(err)
    }
}
impl From<AppError> for Error {
    fn from(err: AppError) -> Self {
        match err {
            AppError::Concentrator(err) => err,
            AppError::IO(err) => Error::Data,
            AppError::Config(err) => Error::Toml(err),
            AppError::Generic(err) => Error::Data,
        }
    }
}

/// Wraps a `libloragw-sys` function call and:
/// - wraps the return code in a `Result`
/// - logs name of FFI function on error
#[macro_export] // Optional: exposes the macro if you need it outside this module
macro_rules! hal_call {
    ( $fn:ident ( $($arg:expr),* ) ) => {
        match crate::llg::$fn ( $($arg),* ) {
            -1 => {
                eprintln!("HAL call {} returned an error", stringify!($fn));
                Err($crate::error::Error::HAL)
            }
            val if val >= 0 => Ok(val as usize),
            invalid => panic!("HAL call {} returned invalid value {}", stringify!($fn), invalid),
        }
    }
}
