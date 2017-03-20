
extern crate slog_scope;

extern crate futures;
extern crate hyper;

use self::futures::task::Task;

use self::futures::Stream;
use self::futures::Future;
use self::futures::future;

use hyper::header::{ContentLength, ContentType};
use hyper::server::{Service, Request, Response};
use hyper::mime::{Mime, TopLevel, SubLevel};
use hyper::server;
use hyper::status::StatusCode;

use std::sync::{Arc, Mutex, mpsc};
use std::collections::HashMap;

use device::Device;
use weectrl::State;
use weectrl::StateNotification;
use error;
use xml;
use rpc;

#[derive(Clone)]
pub struct IncomingNotification {
    pub tx: Arc<Mutex<mpsc::Sender<Option<StateNotification>>>>,
    pub devices: Arc<Mutex<HashMap<String, Device>>>,
    pub task: Arc<Mutex<Option<Task>>>,
}

fn bad_request() -> Box<Future<Error = hyper::Error, Item = server::Response>> {
    let mut res = server::Response::new();
    res.set_status(StatusCode::BadRequest);
    future::ok(res).boxed()
}

impl Service for IncomingNotification {
    type Request = Request;
    type Response = Response;
    type Error = hyper::Error;
    // type Future = FutureResult<Response, hyper::Error>;
    type Future = Box<Future<Item = Response, Error = hyper::Error>>;

    fn call(&self, req: Request) -> Self::Future {

        let (method, _, _, headers, body) = req.deconstruct();

        info!(slog_scope::logger(), "MESSAGE: {:?}", method);

        let notification_sid = match rpc::extract_sid(&headers) {
            Some(sid) => sid,
            None => return bad_request(),
        };

        let devices = self.devices.clone();
        let tx = self.tx.clone();
        let task = self.task.clone();

        body.fold(Vec::new(), |mut v, chunk| {
                v.extend(&chunk[..]);
                future::ok::<_, hyper::Error>(v)
            })
            .map(move |buffer| {

                let data = String::from_utf8(Vec::from(&buffer[..])).unwrap();
                if let Some(state) = xml::get_binary_state(&data) {

                    for (unique_id, dev) in devices.lock().expect(error::FATAL_LOCK).iter() {
                        if let Some(device_sid) = dev.sid() {
                            // Found a match, send notification to the client
                            if notification_sid == device_sid {
                                info!(slog_scope::logger(),
                                      "Got switch update for {:?}. sid: {:?}. State: {:?}.",
                                      unique_id,
                                      notification_sid,
                                      State::from(state));

                                let n = Some(StateNotification {
                                    unique_id: unique_id.to_owned(),
                                    state: State::from(state),
                                });

                                let r = tx.lock().expect(error::FATAL_LOCK).send(n);

                                use std::ops::Deref;
                                // If channel/future closed, forget Task.
                                match r {
                                    Ok(_) => {
                                        let mtx = task.lock().expect(error::FATAL_LOCK);
                                        match mtx.deref() {
                                            &Some(ref s) => s.unpark(),
                                            &None => (),
                                        }
                                    }
                                    Err(e) => {
                                        let mut task = task.lock().expect(error::FATAL_LOCK);
                                        *task = None;
                                        info!(slog_scope::logger(), "Future cancelled. {:?}", e);
                                    }
                                }
                                break; // Found match
                            }
                        }
                    }
                }
                let body: &[u8] = b"<html><body><h1>200 OK</h1></body></html>";
                Response::new()
                    .with_header(ContentLength(body.len() as u64))
                    .with_header(ContentType(Mime(TopLevel::Text, SubLevel::Html, vec![])))
                    .with_body(body)
            })
            .boxed()
    }
}
