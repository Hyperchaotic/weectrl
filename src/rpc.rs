extern crate slog_scope;

use std::io::Read;
use hyper::Client;
use hyper::method::Method;
use hyper::header::{ContentType, ContentLength};
use hyper::header::Headers;
use hyper::mime::{Mime, TopLevel, SubLevel, Attr, Value};
use hyper::client::response::Response;
pub use hyper::status::StatusCode;
pub use hyper::Error as HttpError;
pub use error::Error;

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

pub fn extract_sid(headers: &Headers) -> Option<String> {

    let sid = match headers.get() {
        Some(&SubscribeSID(ref sid)) => Some(sid.clone()),
        None => None,
    };

    sid
}

/// Perform a HTTP unsubscribe action to specified URL.
pub fn unsubscribe_action(url: &str, sid: &str) -> Result<StatusCode, Error> {

    let client = Client::new();
    let unsubscribe = Method::Extension("UNSUBSCRIBE".to_string());

    let response = client.request(unsubscribe, url)
        .header(SubscribeSID(sid.into()))
        .body("")
        .send()?;

    if response.status != StatusCode::Ok {
        error!(slog_scope::logger(),
               "Received status code: {:?}",
               response.status);
        return Err(Error::InvalidResponse(response.status));
    }

    Ok(response.status)
}

fn handle_subscription_response(response: &mut Response) -> Result<SubscribeResponse, Error> {

    if response.status != StatusCode::Ok {
        error!(slog_scope::logger(),
               "Received status code: {:?}",
               response.status);
        return Err(Error::InvalidResponse(response.status));
    }

    let sid = match response.headers.get() {
        Some(&SubscribeSID(ref sid)) => sid.clone(),
        _ => String::new(),
    };

    let timeout_string = match response.headers.get() {
        Some(&SubscribeTimeout(ref string)) => string.clone(),
        _ => String::new(),
    };

    let mut timeout = 0;

    if timeout_string.starts_with("Second-") {
        let (_, number) = timeout_string.split_at("Second-".len());
        timeout = match number.parse::<u32>() {
            Ok(n) => n,
            Err(_) => 0,
        };
    }

    let mut body = String::new();
    response.read_to_string(&mut body)?;

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

/// Perform a HTTP subscribe action to specified URL, renewing an active subscription.
pub fn resubscribe(url: &str, sid: &str, timeout: u32) -> Result<SubscribeResponse, Error> {

    let client = Client::new();
    let subscribe = Method::Extension("SUBSCRIBE".to_string());
    let timeout_str = format!("Second-{}", timeout);

    info!(slog_scope::logger(), "RESUBSCRIBE sid: {:?}.", sid);

    let mut response = client.request(subscribe, url)
        .header(SubscribeSID(sid.into()))
        .header(SubscribeTimeout(timeout_str.into()))
        .body("")
        .send()?;

    handle_subscription_response(&mut response)
}

/// Perform a HTTP subscribe action to specified URL, starting a new subscription.
pub fn subscribe(url: &str, timeout: u32, callback: &str) -> Result<SubscribeResponse, Error> {

    let client = Client::new();
    let subscribe = Method::Extension("SUBSCRIBE".to_string());
    let timeout_str = format!("Second-{}", timeout);

    info!(slog_scope::logger(),
          "SUBSCRIBE url: {:?}. timeout: {:?}s. callback: {:?}.",
          url,
          timeout,
          callback);

    let mut response = client.request(subscribe, url)
        .header(SubscribeCallback(callback.into()))
        .header(SubscribeNT("upnp:event".into()))
        .header(SubscribeTimeout(timeout_str.into()))
        .body("")
        .send()?;

    handle_subscription_response(&mut response)
}

pub fn http_get(url: &str) -> Result<Vec<u8>, Error> {
    let client = Client::new();

    let mut response = client.get(url).send()?;

    if response.status != StatusCode::Ok {
        error!(slog_scope::logger(), "Received status code: {:?}", response.status);
        return Err(Error::InvalidResponse(response.status));
    }
    // Copy data into vec
    let mut data: Vec<u8> = Vec::new();
    response.read_to_end(&mut data)?;
    Ok(data)
}

/// Perform a HTTP SOAP action to specified URL.
pub fn soap_action(url: &str, action: &str, xml: &str) -> Result<String, Error> {
    let client = Client::new();
    use hyper::header::Connection;
    let mut response = client.post(url)
        .header(SoapAction(action.into()))
        .header(ContentLength(xml.len() as u64))
        .header(ContentType(Mime(TopLevel::Text,
                                 SubLevel::Xml,
                                 vec![(Attr::Charset, Value::Utf8)])))
        .header(Connection::close())
        .body(xml)
        .send()?;

    let mut body = String::new();
    response.read_to_string(&mut body)?;

    if body.contains("<BinaryState>Error</BinaryState>") {
        error!(slog_scope::logger(),
               "Sent to: {:?}. Action: {:?}. Received status: Error",
               url,
               action);
        return Err(Error::DeviceError);
    }
    info!(slog_scope::logger(),
          "Sent to: {:?}. Action: {:?}. Received status: {:?}",
          url,
          action,
          response.status);

    if response.status != StatusCode::Ok {
        error!(slog_scope::logger(), "Received status code: {:?}", response.status);
        return Err(Error::InvalidResponse(response.status));
    }

    Ok(body)
}
