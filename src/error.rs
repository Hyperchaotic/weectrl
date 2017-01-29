use hyper::Error as HttpError;
use std::io;
use url::ParseError;
use hyper::status::StatusCode;
use serde_xml::Error as SerdeError;

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
