extern crate slog_scope;
extern crate ssdp;

use std::net::IpAddr;
use std::{thread, time};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};

use rpc;
use xml;
use xml::Root;
use error;
use error::Error;

#[derive(Debug, Clone, Copy, PartialEq)]
/// Represents whether a device binarystate is on or off.
pub enum State {
    /// Device is switched on
    On,
    /// Device is switched off
    Off,
    /// Device is not responding to queries or commands
    Unknown,
}

#[derive(Debug, Clone)]
/// One WeMo or similar device
pub struct Device {
    /// BinaryState of the device
    pub state: State,
    /// Device IP
    pub base_url: String,
    /// Device "home" URL
    pub location: String,
    /// IP address of local interface.
    local_ip: IpAddr,
    /// Path for retrieve and toggle switch state.
    binary_control_path: Option<String>,
    /// Path for subscribing to events
    event_path: Option<String>,
    /// Subscription ID for notifications, can be updated from daemon thread if it changes.
    sid: Option<Arc<Mutex<String>>>,
    /// If subscribed for notifications this is used to cancel th daemon.
    subscription_daemon: Option<mpsc::Sender<()>>,
    /// Device information, from XML homepage
    pub root: Root,
}

impl Device {
    pub fn new(state: State, base_url: &str, location: &str, local_ip: IpAddr, root: &Root) -> Device {
        let dev = Device {
            state: state,
            base_url: base_url.to_owned(),
            location: location.to_owned(),
            local_ip: local_ip,
            binary_control_path: None,
            event_path: None,
            sid: None,
            subscription_daemon: None,
            root: root.clone(),
        };
        info!(slog_scope::logger(),
              "New device {}, {} @ {}",
              dev.root.device.friendly_name,
              dev.root.device.mac_address,
              base_url);
        dev
    }

    /// Returns copy of current subscription ID.
    pub fn sid(&self) -> Option<String> {
        if let Some(dev_sid) = self.sid.clone() {
            let sid = dev_sid.lock().expect(error::FATAL_LOCK).clone();
            return Some(sid);
        }
        None
    }

    /// Any supported device needs to have the Belkin basicevent service. Check for its
    /// existence and save the paths for control/subscriptions for future use.
    pub fn validate_device(&mut self) -> bool {
        for service in &self.root.device.service_list.service {
            if service.service_type == "urn:Belkin:service:basicevent:1" {
                self.binary_control_path = Some(service.control_url.clone());
                self.event_path = Some(service.event_sub_url.clone());
                return true;
            }
        }
        false
    }

    /// Print basic information to logger
    pub fn print_info(&self) {
        info!(slog_scope::logger(),
              "  -> Friendly name: {:?}. State: {:?}. Location: {:?}.",
              self.root.device.friendly_name,
              self.state,
              self.location);

        let mut list = String::from("    Sevices: ");
        for service in &self.root.device.service_list.service {
            list.push_str(&service.service_type);
            list.push_str(" ");
        }
        info!(slog_scope::logger(), "{:?}", list);
    }

    fn cancel_subscription_daemon(&mut self) {
        if let Some(daemon) = self.subscription_daemon.clone() {
            let _ = daemon.send(());
            self.subscription_daemon = None;
        }
    }

    /// Send unsubscribe command and cancel the daemon if active.
    pub fn unsubscribe(&mut self) -> Result<String, Error> {

        self.cancel_subscription_daemon();

        if let Some(sid_shared) = self.sid.clone() {
            let req_url = Device::make_request_url(&self.base_url, &self.event_path)?;
            let sid = sid_shared.lock().expect(error::FATAL_LOCK).clone();
            let _ = rpc::unsubscribe_action(&req_url, &sid)?; // TODO check statuscode
            self.sid = None;
            return Ok(sid);
        }
        Err(Error::NotSubscribed)
    }

    // Concatenate basic URL with request path.
    // e.g. "http://192.168.0.10/ with "upnp/event/basicevent1".
    fn make_request_url(base_url: &str, req_path: &Option<String>) -> Result<String, Error> {
        if let Some(ref path) = *req_path {
            let mut req_url = base_url.to_owned();
            if req_url.ends_with("/") && path.starts_with("/") {
                req_url = req_url.trim_right_matches("/").to_owned();
            }
            req_url.push_str(path);
            return Ok(req_url);
        }
        Err(Error::UnsupportedDevice)
    }

    /// Send subscribe comand, cancel any currrent daemon and start a new if auto resub on.
    pub fn subscribe(&mut self,
                     port: u16,
                     seconds: u32,
                     auto_resubscribe: bool)
                     -> Result<(String, u32), Error> {

        let callback = format!("<http://{}:{}/>", self.local_ip, port);

        let req_url = Device::make_request_url(&self.base_url, &self.event_path)?;
        self.cancel_subscription_daemon();
        let res = rpc::subscribe(&req_url, seconds, &callback)?;

        let new_sid = Arc::new(Mutex::new(res.sid.clone()));
        self.sid = Some(new_sid.clone());

        if auto_resubscribe {
            let (tx, rx) = mpsc::channel();
            self.subscription_daemon = Some(tx);
            thread::spawn(move || {
                Device::subscription_daemon(rx, req_url, new_sid, seconds, callback);
            });
        }

        return Ok((res.sid, res.timeout));
    }

    /// Resubscribe for notifications. This will cancel auto resubscribe is active.
    pub fn resubscribe(&mut self, seconds: u32) -> Result<u32, Error> {

        self.cancel_subscription_daemon();

        if let Some(sid_shared) = self.sid.clone() {
            let sid = sid_shared.lock().expect(error::FATAL_LOCK).clone();
            let req_url = Device::make_request_url(&self.base_url, &self.event_path)?;
            let res = rpc::resubscribe(&req_url, &sid, seconds)?;
            return Ok(res.timeout);
        }
        Err(Error::NotSubscribed)
    }

    fn subscription_daemon(rx: mpsc::Receiver<()>,
                           url: String,
                           device_sid: Arc<Mutex<String>>,
                           initial_seconds: u32,
                           callback: String) {
        use std::sync::mpsc::TryRecvError;
        use hyper::status::StatusCode;
        let mut duration = time::Duration::from_secs((initial_seconds - 10) as u64);

        loop {
            thread::sleep(duration);
            // Were we signalled to end, or channel nonfunctional?
            match rx.try_recv() {
                Err(TryRecvError::Empty) => (),
                _ => break,
            }
            let sid: String;
            {
                sid = device_sid.lock().expect(error::FATAL_LOCK).clone();
            }
            match rpc::resubscribe(&url, &sid, initial_seconds) {
                // Resub successful, register new timeout and SID
                Ok(res) => {
                    duration = time::Duration::from_secs((res.timeout - 10) as u64);
                    let mut sid_ref = device_sid.lock().expect(error::FATAL_LOCK);
                    sid_ref.clear();
                    sid_ref.push_str(&res.sid);
                },
                // We were too late (perhaps computer was asleep). Subscribe anew.
                Err(Error::InvalidResponse(StatusCode::PreconditionFailed)) => {
                    match rpc::subscribe(&url, initial_seconds, &callback) {
                        Ok(res) => {
                            duration = time::Duration::from_secs((res.timeout - 10) as u64);
                            let mut sid_ref = device_sid.lock().expect(error::FATAL_LOCK);
                            sid_ref.clear();
                            sid_ref.push_str(&res.sid);
                        }
                        Err(_) => break,
                    };
                }
                // Any other error, give up
                Err(_) => break,
            }
        }
    }

    /// Retrieve current switch binarystate.
    pub fn fetch_binary_state(&mut self) -> Result<State, Error> {

        self.state = State::Unknown;

        let req_url = Device::make_request_url(&self.base_url, &self.binary_control_path)?;

        let http_response = rpc::soap_action(&req_url,
                                             "\"urn:Belkin:service:basicevent:1#GetBinaryState\"",
                                             xml::GETBINARYSTATE)?;

        if let Some(state) = xml::get_binary_state(&http_response) {
            self.state = State::from(state);
        } else {
            return Err(Error::InvalidState);
        }

        Ok(self.state)
    }

    /// Send command to toggle switch state. If toggeling to same state an Error is returned.
    pub fn set_binary_state(&mut self, state: State) -> Result<State, Error> {

        let request: &str;
        match state {
            State::On => request = xml::SETBINARYSTATEON,
            State::Off => request = xml::SETBINARYSTATEOFF,
            State::Unknown => return Err(Error::InvalidState),
        }
        let req_url = Device::make_request_url(&self.base_url, &self.binary_control_path)?;
        let _ = rpc::soap_action(&req_url,
                                 "\"urn:Belkin:service:basicevent:1#SetBinaryState\"",
                                 request)?;

        self.state = state;
        Ok(state)
    }
}
