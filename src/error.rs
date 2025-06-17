use thiserror::Error;

#[derive(Error, Debug)]
pub enum MxError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[cfg(feature = "serial")]
    #[error("Serial port error: {0}")]
    Serial(#[from] serialport::Error),

    #[error("Parse error: {0}")]
    Parse(String),

    #[error("Device reported command error: {0}")]
    CommandError(String),

    #[error("Device reported execution error (Code {code}): {error_type} - {description}")]
    ExecutionError {
        code: i32,
        error_type: String,
        description: String,
    },

    #[error("Device reported verify timeout error: {0}")]
    VerifyTimeoutError(String),

    #[error("Device reported query error: {0}")]
    QueryError(String),

    #[error("Undefined error code {0} from device. Command was: {1}")]
    UndefinedDeviceErrorCode(i32, String),

    #[error("Connection not established or invalid")]
    NotConnected,

    #[error("Invalid parameter: {0}")]
    InvalidParameter(String),

    #[error("Feature not supported for current connection or not implemented: {0}")]
    UnsupportedFeature(String),
}

impl From<std::num::ParseFloatError> for MxError {
    fn from(err: std::num::ParseFloatError) -> Self {
        MxError::Parse(format!("Failed to parse float: {}", err))
    }
}

impl From<std::num::ParseIntError> for MxError {
    fn from(err: std::num::ParseIntError) -> Self {
        MxError::Parse(format!("Failed to parse int: {}", err))
    }
}