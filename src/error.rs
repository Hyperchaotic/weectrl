use reqwest::header::InvalidHeaderValue;
use reqwest::Error as HttpError;
use reqwest::StatusCode;
use serde_xml_rs::Error as SerdeError;
use std::io;
use std::num::ParseIntError;
use url::ParseError;

/// If a semaphore fails, panic immediately with this message.
pub const FATAL_LOCK: &str = "FATAL Error, Lock failed!";

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
    HttpHeaderError,
}

impl From<InvalidHeaderValue> for Error {
    fn from(_err: InvalidHeaderValue) -> Self {
        Self::HttpHeaderError
    }
}

impl From<ParseIntError> for Error {
    fn from(err: ParseIntError) -> Self {
        Self::ParseError(err)
    }
}

impl From<SerdeError> for Error {
    fn from(err: SerdeError) -> Self {
        Self::SoapResponseError(err)
    }
}

impl From<ParseError> for Error {
    fn from(err: ParseError) -> Self {
        Self::UrlError(err)
    }
}

impl From<io::Error> for Error {
    fn from(err: io::Error) -> Self {
        Self::IoError(err)
    }
}

impl From<HttpError> for Error {
    fn from(err: HttpError) -> Self {
        Self::NetworkError(err)
    }
}
