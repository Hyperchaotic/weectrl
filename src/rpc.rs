extern crate mime;

use tracing::info;

use std::time::Duration;

pub use reqwest::{Method, Request, Response, StatusCode};

pub use crate::error::Error;
pub use reqwest::Error as HttpError;

use url::Url;

#[derive(Debug)]
pub struct SubscribeResponse {
    pub sid: String,
    pub timeout: u32,
}

struct RpcResponse {
    pub status: StatusCode,
    pub sid: Option<String>,
    pub timeout: Option<String>,
}

/// Perform a HTTP unsubscribe action to specified URL.
pub fn unsubscribe_action(url_str: &str, sid: &str) -> Result<StatusCode, Error> {
    use reqwest::header;

    let mut headers = header::HeaderMap::new();
    headers.insert("SID", header::HeaderValue::from_str(sid)?);
    let url = Url::parse(url_str)?;

    let method = reqwest::Method::from_bytes(b"UNSUBSCRIBE").unwrap();

    let client = reqwest::blocking::Client::new();

    info!("Unsubscribe {}.", sid);
    let response = reqwest::blocking::Client::request(&client, method, url)
        .headers(headers)
        .timeout(Duration::from_secs(10))
        .send()?;

    if response.status() != StatusCode::OK {
        info!("Error {}.", response.status());
        return Err(Error::InvalidResponse(response.status()));
    }

    info!("Unsubscribe ok.");
    Ok(response.status())
}

fn rpc_run(
    method: reqwest::Method,
    headers: reqwest::header::HeaderMap,
    url: Url,
) -> Result<RpcResponse, Error> {
    let client = reqwest::blocking::Client::new();
    info!("Request: {:?}", method.to_string());

    let response = reqwest::blocking::Client::request(&client, method, url)
        .headers(headers)
        .timeout(Duration::from_secs(5))
        .send()?;

    if response.status() != StatusCode::OK {
        return Err(Error::InvalidResponse(response.status()));
    }

    let sid = if let Some(sid) = response.headers().get("SID") {
        Some(sid.to_str().map_err(|_| (Error::InvalidState))?.to_string())
    } else {
        None
    };

    let tim = if let Some(tim) = response.headers().get("TIMEOUT") {
        Some(tim.to_str().map_err(|_| (Error::InvalidState))?.to_string())
    } else {
        None
    };

    Ok(RpcResponse {
        status: response.status(),
        sid: sid,
        timeout: tim,
    })
}

/// Perform a HTTP subscribe action to specified URL, starting a new subscription.
pub fn subscribe(url_str: &str, timeout: u32, callback: &str) -> Result<SubscribeResponse, Error> {
    use reqwest::header;
    info!("Subscribe");

    let method = reqwest::Method::from_bytes(b"SUBSCRIBE").unwrap();

    let mut headers = header::HeaderMap::new();
    headers.insert("CALLBACK", header::HeaderValue::from_str(callback)?);
    headers.insert("NT", header::HeaderValue::from_str("upnp:event")?);
    headers.insert(
        "TIMEOUT",
        header::HeaderValue::from_str(&format!("Second-{}", timeout))?,
    );
    info!("REQ:");
    info!("{:?}", headers);
    let rpcresponse = rpc_run(method, headers, Url::parse(url_str)?)?;

    handle_subscription_response(rpcresponse)
}

pub fn http_get(url_str: &str) -> Result<Vec<u8>, Error> {
    let res = reqwest::blocking::get(Url::parse(url_str)?)?;
    if res.status() != StatusCode::OK {
        return Err(Error::InvalidResponse(res.status()));
    }

    Ok(res.bytes()?.to_vec())
}

fn handle_subscription_response(response: RpcResponse) -> Result<SubscribeResponse, Error> {
    if response.status != StatusCode::OK {
        return Err(Error::InvalidResponse(response.status));
    }

    let sid = match response.sid {
        Some(sid) => sid,
        None => return Err(Error::DeviceError),
    };

    let timeout_string = match response.timeout {
        Some(timeout) => timeout,
        None => return Err(Error::DeviceError),
    };

    let timeout = if timeout_string.starts_with("Second-") {
        let (_, number) = timeout_string.split_at("Second-".len());
        match number.parse::<u32>() {
            Ok(n) => n,
            Err(_) => 0,
        }
    } else {
        0
    };

    Ok(SubscribeResponse {
        sid: sid,
        timeout: timeout,
    })
}

/// Perform a HTTP SOAP action to specified URL.
pub fn soap_action(url_str: &str, action: &str, xml: &str) -> Result<String, Error> {
    use reqwest::header;

    let url = Url::parse(url_str)?;

    let method = reqwest::Method::GET;

    let mut headers = header::HeaderMap::new();

    headers.insert("SOAPACTION", header::HeaderValue::from_str(action)?);
    headers.insert(
        reqwest::header::CONTENT_LENGTH,
        header::HeaderValue::from_str(&xml.len().to_string())?,
    );
    headers.insert(
        reqwest::header::CONTENT_TYPE,
        header::HeaderValue::from_static("text/xml; charset=utf-8"),
    );
    headers.insert(
        reqwest::header::CONNECTION,
        header::HeaderValue::from_str("close")?,
    );

    let client = reqwest::blocking::Client::new();

    let response = reqwest::blocking::Client::request(&client, method, url)
        .headers(headers)
        .body(xml.to_owned())
        .timeout(Duration::from_secs(5))
        .send()?;

    if response.status() != StatusCode::OK {
        return Err(Error::InvalidResponse(response.status()));
    }

    Ok(response.text()?)
}
