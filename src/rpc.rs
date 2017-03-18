extern crate slog_scope;
extern crate futures;
extern crate tokio_core;
extern crate tokio_timer;

use hyper;
use hyper::Method;
use hyper::header::{ContentType, ContentLength, Headers};
use hyper::mime::{Mime, TopLevel, SubLevel, Attr, Value};
use hyper::client::Request;

use self::futures::{future, Future, Stream};

use std::time::Duration;
use self::tokio_timer::Timer;

pub use hyper::StatusCode;
pub use hyper::Error as HttpError;
pub use error::Error;

use url::Url;

header! { (SoapAction, "SOAPACTION") => [String] }
header! { (SubscribeCallback, "CALLBACK") => [String] }
header! { (SubscribeNT, "NT") => [String] }
header! { (SubscribeTimeout, "TIMEOUT") => [String] }
header! { (SubscribeSID, "SID") => [String] }

#[derive(Debug)]
pub struct SubscribeResponse {
    pub sid: String,
    pub timeout: u32,
}

struct RpcResponse {
    pub status: StatusCode,
    pub body: String,
    pub sid: Option<String>,
    pub timeout: Option<String>,
}

// Retrieve a device home page.
fn rpc_run(request: Request) -> Result<RpcResponse, hyper::Error> {

    let mut core = tokio_core::reactor::Core::new().unwrap();
    let handle = core.handle();
    let client = hyper::Client::new(&handle);

    // Future getting home page
    let work = client.request(request)
            .and_then(|res| {
                let sta = *res.status();

                let sid = match res.headers().get() {
                    Some(&SubscribeSID(ref sid)) => Some(sid.clone()),
                    _ => None,
                };

                let tim = match res.headers().get() {
                    Some(&SubscribeTimeout(ref string)) => Some(string.clone()),
                    _ => None,
                };

                res.body().fold((Vec::new(), sta, sid, tim), |mut v, chunk| {
                    v.0.extend(&chunk[..]);
                    future::ok::<_, hyper::Error>(v)
                }).and_then(|chunks| {
                    let s = String::from_utf8(chunks.0).unwrap();
                    future::ok::<_, hyper::Error>((s, chunks.1, chunks.2, chunks.3))
                })
            });

    // Timeout future
    let timer = Timer::default();
    let timeout_future = timer.sleep(Duration::from_secs(10))
        .then(|_| future::err::<_, hyper::Error>(hyper::Error::Timeout));

    let winner = timeout_future.select(work).map(|(win, _)| win);

    match core.run(winner) {
        Ok(body) => Ok(RpcResponse {status: body.1, body: body.0, sid: body.2, timeout: body.3 }),
        Err(e) => Err(e.0),
    }
}

pub fn extract_sid(headers: &Headers) -> Option<String> {

    match headers.get() {
        Some(&SubscribeSID(ref sid)) => Some(sid.clone()),
        None => None,
    }
}

/// Perform a HTTP unsubscribe action to specified URL.
pub fn unsubscribe_action(url_str: &str, sid: &str) -> Result<StatusCode, Error> {


    let unsubscribe = Method::Extension("UNSUBSCRIBE".to_string());
    let url = Url::parse(url_str)?;

    let mut request = Request::new(unsubscribe, url);
    request.headers_mut().set(SubscribeSID(sid.into()));

    let response = rpc_run(request)?;

    if response.status != StatusCode::Ok {
        error!(slog_scope::logger(),
               "Received status code: {:?}",
               response.status);
        return Err(Error::InvalidResponse(response.status));
    }

    Ok(response.status)
}

/// Perform a HTTP subscribe action to specified URL, renewing an active subscription.
pub fn resubscribe(url_str: &str, sid: &str, timeout: u32) -> Result<SubscribeResponse, Error> {

    let resubscribe = Method::Extension("SUBSCRIBE".to_string());
    let url = Url::parse(url_str)?;
    let timeout_str = format!("Second-{}", timeout);

    let mut request = Request::new(resubscribe, url);
    request.headers_mut().set(SubscribeSID(sid.into()));
    request.headers_mut().set(SubscribeTimeout(timeout_str.into()));

    info!(slog_scope::logger(), "RESUBSCRIBE sid: {:?}.", sid);

    let response = rpc_run(request)?;
    handle_subscription_response(response)
}

/// Perform a HTTP subscribe action to specified URL, starting a new subscription.
pub fn subscribe(url_str: &str, timeout: u32, callback: &str) -> Result<SubscribeResponse, Error> {

    let resubscribe = Method::Extension("SUBSCRIBE".to_string());
    let url = Url::parse(url_str)?;
    let timeout_str = format!("Second-{}", timeout);

    let mut request = Request::new(resubscribe, url);
    request.headers_mut().set(SubscribeCallback(callback.into()));
    request.headers_mut().set(SubscribeNT("upnp:event".into()));
    request.headers_mut().set(SubscribeTimeout(timeout_str.into()));

    info!(slog_scope::logger(),
          "SUBSCRIBE url: {:?}. timeout: {:?}s. callback: {:?}.",
          url_str,
          timeout,
          callback);

    let response = rpc_run(request)?;
    handle_subscription_response(response)
}

pub fn http_get(url_str: &str) -> Result<Vec<u8>, Error> {

    let url = Url::parse(url_str)?;
    let request = Request::new(Method::Get, url);

    let mut core = tokio_core::reactor::Core::new().unwrap();
    let handle = core.handle();
    let client = hyper::Client::new(&handle);

    // Future getting home page
    let work = client.request(request)
            .and_then(|res| {
                let sta = *res.status();
                res.body().fold((Vec::new(), sta), |mut v, chunk| {
                    v.0.extend(&chunk[..]);
                    future::ok::<_, hyper::Error>(v)
                }).and_then(|chunks| {
                    future::ok::<_, hyper::Error>(chunks)
                })
            });

    let response = core.run(work)?;

    if response.1 != StatusCode::Ok {
        error!(slog_scope::logger(),
               "Received status code: {:?}",
               response.1);
        return Err(Error::InvalidResponse(response.1));
    }

    Ok(response.0)
}

fn handle_subscription_response(response: RpcResponse) -> Result<SubscribeResponse, Error> {

    if response.status != StatusCode::Ok {
        error!(slog_scope::logger(),
               "Received status code: {:?}",
               response.status);
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

    info!(slog_scope::logger(),
          "RESPONSE: {:?} - {:?} - {:?}s",
          sid,
          response.status,
          timeout);

    Ok(SubscribeResponse {
        sid: sid,
        timeout: timeout,
    })
}

/// Perform a HTTP SOAP action to specified URL.
pub fn soap_action(url_str: &str, action: &str, xml: &str) -> Result<String, Error> {

    use hyper::header::Connection;

    let url = Url::parse(url_str)?;
    let req_body = xml.to_owned();

    let mut request = Request::new(Method::Get, url);
    request.headers_mut().set(SoapAction(action.into()));
    request.headers_mut().set(ContentLength(xml.len() as u64));
    request.headers_mut().set(ContentType(Mime(TopLevel::Text,
                             SubLevel::Xml,
                             vec![(Attr::Charset, Value::Utf8)])));
    request.headers_mut().set(Connection::close());
    request.set_body(req_body);

    let response = rpc_run(request)?;

    if response.body.contains("<BinaryState>Error</BinaryState>") {
        error!(slog_scope::logger(),
               "Sent to: {:?}. Action: {:?}. Received status: Error",
               url_str,
               action);
        return Err(Error::DeviceError);
    }
    info!(slog_scope::logger(),
          "Sent to: {:?}. Action: {:?}. Received status: {:?}",
          url_str,
          action,
          response.status);

    if response.status != StatusCode::Ok {
        error!(slog_scope::logger(),
               "Received status code: {:?}",
               response.status);
        return Err(Error::InvalidResponse(response.status));
    }

    Ok(response.body)
}
