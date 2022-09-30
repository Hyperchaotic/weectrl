use std::net::IpAddr;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::{thread, time};
use tracing::info;

use crate::error;
use crate::error::Error;
use crate::rpc;
use crate::weectrl::{DeviceInfo, Icon, State};
use crate::xml;

#[derive(Debug, Clone)]
/// One WeMo or similar device
pub struct Device {
    /// IP address of local interface on the same network as the "remote" device.
    local_ip: IpAddr,
    /// Path for retrieve and toggle switch state.
    binary_control_path: Option<String>,
    /// Path for subscribing to events
    event_path: Option<String>,
    /// Subscription ID for notifications, can be updated from daemon thread if it changes.
    sid: Option<Arc<Mutex<String>>>,
    /// If subscribed for notifications this is used to cancel th daemon.
    subscription_daemon: Option<mpsc::Sender<()>>,
    // Information about device, including that returned by 'home' url.
    pub info: DeviceInfo,
}

impl Device {
    pub fn new(info: DeviceInfo, local_ip: IpAddr) -> Self {
        let dev = Self {
            local_ip,
            binary_control_path: None,
            event_path: None,
            sid: None,
            subscription_daemon: None,
            info,
        };
        info!(
            "New device {}, {} @ {}",
            dev.info.root.device.friendly_name, dev.info.root.device.mac_address, dev.info.base_url
        );
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
        for service in &self.info.root.device.service_list.service {
            if service.service_type == "urn:Belkin:service:basicevent:1" {
                self.binary_control_path = Some(service.control_url.clone());
                self.event_path = Some(service.event_sub_url.clone());
                return true;
            }
        }
        false
    }

    fn cancel_subscription_daemon(&mut self) {
        if let Some(daemon) = self.subscription_daemon.clone() {
            info!("Cancel subscription daemon");
            let _ = daemon.send(());
            self.subscription_daemon = None;
        }
    }

    /// Send unsubscribe command and cancel the daemon if active.
    pub fn unsubscribe(&mut self) -> Result<String, Error> {
        info!("UnSubscribe");
        self.cancel_subscription_daemon();

        if let Some(sid_shared) = self.sid.clone() {
            let req_url = Self::make_request_url(&self.info.base_url, &self.event_path)?;
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
            if req_url.ends_with('/') && path.starts_with('/') {
                req_url = req_url.trim_end_matches('/').to_owned();
            }
            req_url.push_str(path);
            return Ok(req_url);
        }
        Err(Error::UnsupportedDevice)
    }

    /// Send subscribe comand, cancel any currrent daemon and start a new if auto resub on.
    pub fn subscribe(
        &mut self,
        port: u16,
        seconds: u32,
        auto_resubscribe: bool,
    ) -> Result<(String, u32), Error> {
        info!("Subscribe");
        let callback = format!("<http://{}:{}/>", self.local_ip, port);
        let req_url = Self::make_request_url(&self.info.base_url, &self.event_path)?;
        self.cancel_subscription_daemon();
        let res = rpc::subscribe(&req_url, seconds, &callback)?;

        let new_sid = Arc::new(Mutex::new(res.sid.clone()));
        self.sid = Some(new_sid.clone());

        if auto_resubscribe {
            let (tx, rx) = mpsc::channel();
            self.subscription_daemon = Some(tx);

            std::thread::Builder::new()
                .name(format!("DEV_resub_thread {}", self.info.friendly_name))
                .spawn(move || {
                    Self::subscription_daemon(&rx, &req_url, &new_sid, seconds, &callback);
                })?;
        }

        Ok((res.sid, res.timeout))
    }

    fn subscription_daemon(
        rx: &mpsc::Receiver<()>,
        url: &str,
        device_sid: &Arc<Mutex<String>>,
        initial_seconds: u32,
        callback: &str,
    ) {
        use std::sync::mpsc::TryRecvError;
        use time::Duration;

        let mut duration = Duration::from_secs(u64::from(initial_seconds - 10));

        loop {
            thread::sleep(duration);
            // Were we signalled to end, or channel nonfunctional?
            match rx.try_recv() {
                Err(TryRecvError::Empty) => (),
                _ => break,
            }
            info!("Resubscribing.");
            match rpc::subscribe(url, initial_seconds, callback) {
                Ok(res) => {
                    duration = time::Duration::from_secs(u64::from(res.timeout - 10)); // Switch returns timeout we need to obey.
                    let mut sid_ref = device_sid.lock().expect(error::FATAL_LOCK);
                    sid_ref.clear();
                    sid_ref.push_str(&res.sid);
                }
                Err(e) => {
                    info!("Err: {:?}", e);
                    continue; // Could be error due to temporary network issue, just keep trying after a bit.
                }
            };
        }
    }

    /// Retrieve current switch binarystate.
    pub fn fetch_icons(&mut self) -> Result<Vec<Icon>, Error> {
        let mut icon_list = Vec::new();

        self.info.state = State::Unknown;

        for xmlicon in &self.info.root.device.icon_list.icon {
            let width = xmlicon.width.parse::<u64>()?;
            let height = xmlicon.height.parse::<u64>()?;
            let depth = xmlicon.depth.parse::<u64>()?;

            let req_file = Some(xmlicon.url.clone());
            let req_url = Self::make_request_url(&self.info.base_url, &req_file)?;

            let data = rpc::http_get(&req_url)?;

            // Image type is unreliable, so we have to peek in the data stream to detect it.
            let mimetype: mime::Mime = match data.as_slice() {
                // JPEG
                [0xFF, 0xD8, 0xFF, ..] => mime::IMAGE_JPEG,
                // GIF
                [0x47, 0x49, 0x46, 0x38, ..] => mime::IMAGE_GIF,
                // PNG
                [0x89, 0x50, 0x4E, 0x47, ..] => mime::IMAGE_PNG,
                _ => return Err(Error::InvalidResponse(reqwest::StatusCode::IM_A_TEAPOT)),
            };

            info!(
                "Fetch Icon {}. MimeType returned: {:#?}",
                &req_url, &mimetype
            );

            let icon = Icon {
                mimetype,
                width,
                height,
                depth,
                data,
            };

            icon_list.push(icon);
        }

        Ok(icon_list)
    }

    /// Retrieve current switch binarystate.
    pub fn fetch_binary_state(&mut self) -> Result<State, Error> {
        self.info.state = State::Unknown;

        let req_url = Self::make_request_url(&self.info.base_url, &self.binary_control_path)?;

        let http_response = rpc::soap_action(
            &req_url,
            "\"urn:Belkin:service:basicevent:1#GetBinaryState\"",
            xml::GETBINARYSTATE,
        )?;

        if let Some(state) = xml::get_binary_state(&http_response) {
            self.info.state = State::from(state);
        } else {
            return Err(Error::InvalidState);
        }

        Ok(self.info.state)
    }

    /// Send command to toggle switch state. If toggeling to same state an Error is returned.
    pub fn set_binary_state(&mut self, state: State) -> Result<State, Error> {
        if state == State::Unknown {
            return Err(Error::InvalidState);
        }

        let req_url = Self::make_request_url(&self.info.base_url, &self.binary_control_path)?;

        rpc::soap_action(
            &req_url,
            "\"urn:Belkin:service:basicevent:1#SetBinaryState\"",
            &state.to_string(),
        )?;

        self.info.state = state;
        Ok(state)
    }
}
