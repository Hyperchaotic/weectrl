use hyper::Error as HttpError;
use std::io;
use url::ParseError;
use hyper::status::StatusCode;
use serde_xml::Error as SerdeError;
use std::num::ParseIntError;

/// If a semaphore fails, panic immediately with this message.
pub const FATAL_LOCK: &'static str = "FATAL Error, Lock failed!";

#[derive(Debug)]
pub enum Error {
    InvalidState,
    NoResponse,
    UnknownDevice,
    UnsupportedDevice,
    DeviceAlreadyRegistered,
    DeviceError,
    ServiceAlreadyRunning,
    ServiceNotRunning,
    NotSubscribed,
    AutoResubscribeRunning,
    TimeoutTooShort,
    SoapResponseError(SerdeError),
    InvalidResponse(StatusCode),
    NetworkError(HttpError),
    IoError(io::Error),
    UrlError(ParseError),
    ParseError(ParseIntError),
}

impl From<ParseIntError> for Error {
    fn from(err: ParseIntError) -> Error {
        Error::ParseError(err)
    }
}

impl From<SerdeError> for Error {
    fn from(err: SerdeError) -> Error {
        Error::SoapResponseError(err)
    }
}

impl From<ParseError> for Error {
    fn from(err: ParseError) -> Error {
        Error::UrlError(err)
    }
}

impl From<io::Error> for Error {
    fn from(err: io::Error) -> Error {
        Error::IoError(err)
    }
}

impl From<HttpError> for Error {
    fn from(err: HttpError) -> Error {
        Error::NetworkError(err)
    }
}
