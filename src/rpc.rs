extern crate mime;

use reqwest::header;
use reqwest::header::HeaderMap;
use tracing::info;

use std::sync::mpsc;
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
pub fn unsubscribe_action(
    url: &Url,
    sid: &str,
    conn_timeout: Duration,
) -> Result<StatusCode, Error> {
    info!("Unsubscribe {}.", sid);

    let mut headers = header::HeaderMap::new();
    headers.insert("SID", header::HeaderValue::from_str(sid)?);

    let method = reqwest::Method::from_bytes(b"UNSUBSCRIBE").unwrap();

    let response = http_request(&method, &headers, url, "", conn_timeout)?;

    if response.status() != StatusCode::OK {
        info!("Unsubscribe {}, Error {}.", sid, response.status());
        return Err(Error::InvalidResponse(response.status()));
    }

    info!("Unsubscribe {} ok.", sid);
    Ok(response.status())
}

fn rpc_run(
    method: &Method,
    headers: &HeaderMap,
    url: &Url,
    conn_timeout: Duration,
) -> Result<RpcResponse, Error> {
    info!("Request: {:?} {:?}", method.to_string(), &url);

    let response = http_request(method, headers, url, "", conn_timeout)?;

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
        sid,
        timeout: tim,
    })
}

/// Perform a HTTP subscribe action to specified URL, starting a new subscription.
pub fn subscribe(
    url: &Url,
    sub_timeout: Duration,
    callback: &str,
    conn_timeout: Duration,
) -> Result<SubscribeResponse, Error> {
    info!("Subscribe");

    let method = Method::from_bytes(b"SUBSCRIBE").unwrap();

    let mut headers = HeaderMap::new();
    headers.insert("CALLBACK", header::HeaderValue::from_str(callback)?);
    headers.insert("NT", header::HeaderValue::from_str("upnp:event")?);
    headers.insert(
        "TIMEOUT",
        header::HeaderValue::from_str(&format!("Second-{}", sub_timeout.as_secs()))?,
    );
    info!("REQ:");
    info!("{:?}", headers);
    let rpcresponse = rpc_run(&method, &headers, url, conn_timeout)?;

    handle_subscription_response(rpcresponse)
}

pub fn http_get_text(url: &Url, conn_timeout: Duration) -> Result<String, Error> {
    let headers = HeaderMap::new();

    let response = http_request(&reqwest::Method::GET, &headers, url, "", conn_timeout)?;

    if response.status() != StatusCode::OK {
        return Err(Error::InvalidResponse(response.status()));
    }

    Ok(response.text()?)
}

pub fn http_get_bytes(url: &Url, conn_timeout: Duration) -> Result<Vec<u8>, Error> {
    let headers = HeaderMap::new();

    let response = http_request(&reqwest::Method::GET, &headers, url, "", conn_timeout)?;

    if response.status() != StatusCode::OK {
        return Err(Error::InvalidResponse(response.status()));
    }

    Ok(response.bytes()?.to_vec())
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
        number.parse::<u32>().unwrap_or(0)
    } else {
        0
    };

    Ok(SubscribeResponse { sid, timeout })
}

/// Perform a HTTP SOAP action to specified URL.
pub fn soap_action(
    url: &Url,
    action: &str,
    xml: &str,
    conn_timeout: Duration,
) -> Result<String, Error> {
    info!("soap_action, url: {:?}", url.to_string());

    let method = Method::GET;

    let mut headers = HeaderMap::new();

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

    let response = http_request(&method, &headers, url, xml, conn_timeout)?;

    if response.status() != StatusCode::OK {
        return Err(Error::InvalidResponse(response.status()));
    }

    Ok(response.text()?)
}

// Blocking Reqwest runs an async executor, if the app using this library does the same
// the reactors will be nested and request will explode horribly.
// So we give request its own thread.
pub fn http_request(
    method: &Method,
    headers: &HeaderMap,
    url: &Url,
    body: &str,
    conn_timeout: Duration,
) -> Result<reqwest::blocking::Response, Error> {
    let (tx, rx) = mpsc::channel();

    let (method, headers, url, body) = (
        method.clone(),
        headers.clone(),
        url.clone(),
        body.to_string(),
    );

    std::thread::spawn(move || {
        let client = reqwest::blocking::Client::new();

        let request = reqwest::blocking::Client::request(&client, method.clone(), url.clone())
            .headers(headers.clone())
            .body(body.to_string())
            .timeout(conn_timeout);
        let response = request.send();
        let _ignore = tx.send(response.map_err(|e| Error::from(e)));
    });

    rx.recv().unwrap()
}
