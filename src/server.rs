
extern crate slog_scope;

extern crate futures;
extern crate hyper;

use self::futures::future::FutureResult;
use self::futures::task::Task;

use self::futures::Stream;
use self::futures::future;
use self::futures::Future;

use hyper::StatusCode;
use hyper::header::{ContentLength, ContentType};
use hyper::server::{Service, Request, Response};
use hyper::Method::Extension;
use hyper::mime::{Mime, TopLevel, SubLevel};


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

impl Service for IncomingNotification {
    type Request = Request;
    type Response = Response;
    type Error = hyper::Error;
    // type Future = FutureResult<Response, hyper::Error>;
    type Future = Box<Future<Item = Response, Error = hyper::Error>>;

    fn call(&self, req: Request) -> Self::Future {

        info!(slog_scope::logger(), "MESSAGE: {:?}", req.method());

		let notification_sid = match rpc::extract_sid(req.headers()) {
			Some(sid) => sid,
			None => String::new()
		};

		let devices = self.devices.clone();
		let tx = self.tx.clone();
		let task = self.task.clone();

        req.body()
            .for_each(move |chunk| {
                let data = String::from_utf8(Vec::from(&chunk[..])).unwrap();

				let mut state = State::Unknown;
 				if let Some(s) = xml::get_binary_state(&data) {
					state = State::from(s);
				}

				if state!=State::Unknown && !notification_sid.is_empty() {

					for (unique_id, dev) in devices.lock().expect(error::FATAL_LOCK).iter() {
						if let Some(device_sid) = dev.sid() {

							// Found a match, send notification to the client
							if notification_sid == device_sid {
								info!(slog_scope::logger(),
									  "Got switch update for {:?}. sid: {:?}. State: {:?}.",
									  unique_id,
									  notification_sid,
									  state);

								let n = Some(StateNotification {
									unique_id: unique_id.to_owned(),
									state: state,
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
										info!(slog_scope::logger(),
											  "Future cancelled. {:?}",
											  e);
									}
								}
								break; // Found match
							}
						}
					}
					return Ok(());
				}
				Err(hyper::Error::Incomplete)
            })
            .map(move |_| {
                let body: &[u8] = b"<html><body><h1>200 OK</h1></body></html>";
                Response::new()
                    .with_header(ContentLength(body.len() as u64))
                    .with_header(ContentType(Mime(TopLevel::Text, SubLevel::Html, vec![])))
                    .with_body(body)

            })
            .boxed()
    }
}
