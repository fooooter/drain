use std::error::Error;
use std::io::Error as IoError;
use std::fmt::Display;

#[derive(Debug)]
pub enum ServerError {
    InvalidStatusCode(u16),
    DecompressionError(IoError),
    UnsupportedEncoding,
    UnsupportedMediaType,
    MalformedPayload,
    InvalidRequest,
    BodyTooLarge,
    VersionNotSupported
}

impl Display for ServerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ServerError::InvalidStatusCode(status) => write!(f, "Invalid HTTP error status was provided to send_response(): {status}."),
            ServerError::DecompressionError(io_error) => write!(f, "An error occurred while decoding the payload: {io_error}."),
            ServerError::UnsupportedEncoding => write!(f, "Payload was encoded in an unsupported encoding."),
            ServerError::UnsupportedMediaType => write!(f, "Payload contained data in an unsupported media type."),
            ServerError::MalformedPayload => write!(f, "Payload contained malformed data."),
            ServerError::InvalidRequest => write!(f, "A request was malformed."),
            ServerError::BodyTooLarge => write!(f, "Content sent by the client was too large."),
            ServerError::VersionNotSupported => write!(f, "HTTP version not supported.")
        }
    }
}

impl Error for ServerError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        if let ServerError::DecompressionError(io_error) = self {
            Some(io_error)
        } else {
            None
        }
    }
}