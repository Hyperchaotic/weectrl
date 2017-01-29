
use slog::DrainExt;
use slog;
use slog_stdlog;
use slog_scope;
extern crate ssdp;

use std::sync::{Arc, Mutex};
use std::thread;
use std::net::IpAddr;

use hyper::header::Connection;

use std::io::Read;
use std::sync::mpsc;

use self::ssdp::message::{SearchRequest, SearchResponse};
use self::ssdp::header::{HeaderMut, Man, MX, ST, SearchPort};
use std::collections::HashMap;
use std::collections::hash_map::Entry;

use device::{State, Device};
use rpc;
use xml;
use xml::Root;
use error::Error;
use url::Url;

use cache::{DiskCache, DeviceAddress};

// pub const SUPPORTED_DEVICES: &'static [&'static str] = &["uuid:Socket", "uuid:Lightswitch"];

// Port number for daemon listening for notifications.
// Should be changed to OS assigned.
const DAEMON_PORT: u16 = 9193;

#[derive(Debug, Clone)]
/// Notification from a device on network that binary state have changed.
pub struct StateNotification {
    /// Same unique id as known from discovery and used to subscribe.
    pub unique_id: String,
    /// New binarystate of the device.
    pub state: State,
}

impl From<u8> for State {
    fn from(u: u8) -> Self {
        match u {
            0 => State::Off,
            1 => State::On,
            _ => State::Unknown,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
/// Specify the methods to be used for device discovery.
pub enum DiscoveryMode {
    /// Read and verify known devices from disk cache. Don't brodcast uPnP query.
    CacheOnly,
    /// Read and verify known devices from disk cache, then brodcast uPnP query.
    CacheAndBroadcast,
    /// Broadcast uPnP query, don't load devices from disk cache. This will cause
    /// disk cache to be overwritten with new list of devices.
    BroadcastOnly,
}

#[derive(Debug, Clone)]
// Object returned for each device found during discovery.
pub struct DeviceInfo {
    /// Human readable name returned from the device homepage.
    pub friendly_name: String,
    /// Unique identifier for this device, used for issuing commands to controller.
    pub unique_id: String,
    /// Network ostname, usually the IP address.
    pub hostname: String,
    /// Current switch binarystate
    pub state: State,
}

/// Controller entity used for finding and control Belkin WeMo, and compatible, devices.
pub struct WeeController {
    logger: slog::Logger,
    cache: Arc<Mutex<DiskCache>>,
    devices: Arc<Mutex<HashMap<String, Device>>>,
    subscription_daemon: bool,
}

impl WeeController {
    /// Create the controller with an optional logger. If no logger will default to slog_stdlog.
    pub fn new(logger: Option<slog::Logger>) -> WeeController {

        let object = WeeController {
            logger: logger.unwrap_or(slog::Logger::root(slog_stdlog::StdLog.fuse(), o!())),
            cache: Arc::new(Mutex::new(DiskCache::new())),
            devices: Arc::new(Mutex::new(HashMap::new())),
            subscription_daemon: false,
        };
        slog_scope::set_global_logger(object.logger.clone());
        info!(slog_scope::logger(), "Init");
        object
    }

    fn extract_sddp_header(message: &SearchResponse, header: &str) -> Option<String> {
        use weectrl::ssdp::header::HeaderRef;
        use std::str;

        if let Some(data) = message.get_raw(header) {
            match str::from_utf8(&data[0]) {
                Ok(v) => return Some(String::from(v)),
                Err(_) => return None,
            };
        }
        None
    }

    // Get the IP address of this PC, from the same interface talking to the device.
    // It will be needed if subscriptions are used. It's not pretty but easiest way to Make
    // sure we can talk to multiple devices on separate interfaces.
    fn get_local_ip(location: &str) -> Result<IpAddr, Error> {

        use std::net::TcpStream;

        // extract ip:port from URL
        let location_url = Url::parse(&location)?;
        if let Some(host) = location_url.host_str() {

            let mut destination = String::from(host);
            if let Some(port) = location_url.port() {
                let port_str = format!(":{}", port);
                destination.push_str(&port_str);
            }

            // Create TCP connection from which we can get local IP.
            let destination_str: &str = &destination;
            let stream = TcpStream::connect(&destination_str)?;
            let loc_ip = stream.local_addr()?;
            return Ok(loc_ip.ip());
        }
        Err(Error::NoResponse)
    }

    // Retrieve a device home page. Need to configure connect_timeout once hyper supports it.
    fn get_device_home(location: &str) -> Result<String, Error> {
        use hyper::client::Client;
        // let mut client = Client::configure().connect_timeout(
        // Duration::from_millis(2000)).build().unwrap();
        let client = Client::new();
        let mut response = client.get(location).header(Connection::close()).send()?;

        let mut body = String::new();
        response.read_to_string(&mut body)?;
        Ok(body)
    }

    // Write list of active devices to disk cache
    fn refresh_cache(cache: Arc<Mutex<DiskCache>>, devices: Arc<Mutex<HashMap<String, Device>>>) {
        let mut list = Vec::new();
        for (mac, dev) in devices.lock().unwrap().iter() {
            list.push(DeviceAddress {
                location: dev.location.clone(),
                mac_address: mac.clone(),
            })
        }
        cache.lock().unwrap().write(list);
    }

    // Given location URL, query a device. If successful add to list of active devices
    fn retrieve_device(location: &str) -> Result<Device, Error> {

        let body = WeeController::get_device_home(&location)?;
        let local_ip = WeeController::get_local_ip(&location)?;
        let root: Root = xml::parse_services(&body)?;

        info!(slog_scope::logger(),
              "Device {:?} {:?} ",
              root.device.friendly_name,
              location);

        let mut base_url = Url::parse(location)?;
        base_url.set_path("/");

        let mut dev = Device::new(State::Unknown, &base_url.to_string(), location, local_ip, &root);

        if let Some(_) = dev.update_binary_state().ok() {
            if dev.valid_device() {
                return Ok(dev);
            }
        }
        info!(slog_scope::logger(), "Device not supported.");
        return Err(Error::UnsupportedDevice);
    }

    // Add device to hashmap.
    fn register_device(location: &str,
                       devices: &Arc<Mutex<HashMap<String, Device>>>)
                       -> Result<DeviceInfo, Error> {

        let newdev = WeeController::retrieve_device(location)?;
        let mut devs = devices.lock().unwrap();
        if !devs.contains_key(&newdev.root.device.mac_address) {
            info!(slog_scope::logger(),
                  "Registering device {:?}.",
                  &newdev.root.device.mac_address);
            newdev.print_info();

            let mut hostname = String::new();
            if let Some(host) = Url::parse(location).ok() {
                if let Some(hn) = host.host_str() {
                    hostname = hn.to_owned();
                }
            }

            let new = DeviceInfo {
                friendly_name: newdev.root.device.friendly_name.clone(),
                unique_id: newdev.root.device.mac_address.clone(),
                hostname: hostname,
                state: newdev.state,
            };

            devs.insert(newdev.root.device.mac_address.clone(), newdev);
            return Ok(new);
        }
        Err(Error::DeviceAlreadyRegistered)
    }

    /// Read list of know devices from disk cache as well as new devices responding on the network.
    /// Returns immediately and send discovered devices back on the mpsc as they're found.
    /// Allow network devices max `mx` seconds to respond.
    /// When discovery ends, after mx seconds and a bit, the channel will be closed.
    /// `forget_devices` = true will clear the internal list of devices. Discovery will only
    /// return devices to the client not already known internally.
    pub fn discover_async(&mut self, mode: DiscoveryMode, forget_devices: bool, mx: u8) -> mpsc::Receiver<DeviceInfo> {
        if forget_devices {
            self.clear(false);
        }
        let (tx, rx) = mpsc::channel(); // love channels
        let devices = self.devices.clone();
        let cache = self.cache.clone();
        thread::spawn(move || {

            info!(slog_scope::logger(), "Starting discover");

            if mode == DiscoveryMode::CacheOnly || mode == DiscoveryMode::CacheAndBroadcast {
                info!(slog_scope::logger(), "Loading devices from cache.");
                if let Some(cache_list) = cache.lock().unwrap().read() {

                    info!(slog_scope::logger(), "Cached devices {:?}", cache_list);

                    for cache_entry in cache_list.into_iter() {
                        let device =
                            WeeController::register_device(&cache_entry.location, &devices).ok();
                        if let Some(new) = device {
                            let _ = tx.send(new);
                        }
                    }
                }
            }

            if mode == DiscoveryMode::BroadcastOnly || mode == DiscoveryMode::CacheAndBroadcast {
                info!(slog_scope::logger(), "Broadcasting uPnP query.");
                // Create Our Search Request
                let mut request = SearchRequest::new();

                let mut cache_dirty = false;

                // Set Our Desired Headers
                request.set(Man);
                request.set(SearchPort(1900));
                request.set(MX(mx));
                request.set(ST::Target(ssdp::FieldMap::UPnP(String::from("rootdevice"))));

                // Iterate over network responses to our broadcast.
                if let Some(reqs) = request.multicast().ok() {
                    for (msg, _) in reqs {
                        if let Some(location) = WeeController::extract_sddp_header(&msg,
                                                                                   "LOCATION") {
                            let device = WeeController::register_device(&location, &devices).ok();
                            if let Some(new) = device {
                                cache_dirty = true; // This device wasn't in the disk cache.
                                let _ = tx.send(new);
                            }
                        }
                    }
                }
                // We added devices not already in diskcache, time to update it.
                if cache_dirty {
                    WeeController::refresh_cache(cache, devices);
                }
            }
            info!(slog_scope::logger(), "Done! Ending discover thread.");
        });
        rx
    }

    /// Retrieve and query devices. First read list stored on disk, then Broadcast
    /// network query and wait for responses. Allow devices max `mx` seconds to respond.
    /// This function is synchronous and will return everything found after mx seconds and a bit.
    /// For an asynchronous version use `discover_async`.
    /// `forget_devices` = true will clear the internal list of devices. Discovery will only
    /// return devices to the client not already known internally.
    pub fn discover(&mut self, mode: DiscoveryMode, forget_devices: bool, mx: u8) -> Option<Vec<DeviceInfo>> {

        let receiver = self.discover_async(mode, forget_devices, mx);
        let mut list = Vec::new();

        loop {
            if let Some(device) = receiver.recv().ok() {
                list.push(device);
            } else {
                break;
            }
        }

        if list.is_empty() { None } else { Some(list) }
    }

    /// Clear registered devices, optionally also disk cache
    pub fn clear(&mut self, clear_cache: bool) {

        info!(slog_scope::logger(),
              "Clearing list of devices. Clear disk; {:?}.",
              clear_cache);
        self.unsubscribe_all();
        if clear_cache {
            self.cache.lock().unwrap().clear();
        }
        self.devices.lock().unwrap().clear();
    }

    /// Set the device BinaryState (On/Off)
    pub fn set_binary_state(&mut self, unique_id: &str, state: State) -> Result<State, Error> {

        info!(slog_scope::logger(),
              "Set binary state for Device {:?} {:?} ",
              unique_id,
              state);
        let mut devices = self.devices.lock().unwrap();
        if let Entry::Occupied(mut o) = devices.entry(unique_id.to_owned()) {
            let mut device = o.get_mut();
            return device.set_binary_state(state);
        }
        Err(Error::UnknownDevice)
    }

    /// Return all information available about the device
    pub fn get_device_description(&mut self, unique_id: &str) -> Result<Device, Error> {
        let mut devices = self.devices.lock().unwrap();

        info!(slog_scope::logger(),
              "get_device_description for device {:?}.",
              unique_id);
        if let Entry::Occupied(mut o) = devices.entry(unique_id.to_owned()) {
            let mut device = o.get_mut();
            device.update_binary_state()?;
            return Ok(device.clone());
        }
        Err(Error::UnknownDevice)
    }

    /// Query the device for BinaryState (On/Off)
    pub fn get_binary_state(&mut self, unique_id: &str) -> Result<State, Error> {

        info!(slog_scope::logger(),
              "get_binary_state for device {:?}.",
              unique_id);
        let mut devices = self.devices.lock().unwrap();
        if let Entry::Occupied(mut o) = devices.entry(unique_id.to_owned()) {
            let mut device = o.get_mut();
            return device.update_binary_state();
        }
        Err(Error::UnknownDevice)
    }

    /// Cancel subscription for notifications from all devices.
    /// Make sure to cancel all subscriptions before dropping controller.
    pub fn unsubscribe_all(&mut self) {

        info!(slog_scope::logger(), "unsubscribe_all.");

        let mut devices = self.devices.lock().unwrap();
        let unique_ids: Vec<String> = devices.keys().map(|d| d.clone()).collect();
        for unique_id in unique_ids {
            if let Entry::Occupied(mut o) = devices.entry(unique_id.clone()) {
                let mut device = o.get_mut();
                info!(slog_scope::logger(), "unsubscribe {:?}.", unique_id);
                let _ = device.unsubscribe();
            }
        }
    }

    /// Cancel subscription for notifications from a device.
    pub fn unsubscribe(&mut self, unique_id: &str) -> Result<(), Error> {

        if !self.subscription_daemon {
            return Err(Error::ServiceNotRunning);
        }

        let mut devices = self.devices.lock().unwrap();
        if let Entry::Occupied(mut o) = devices.entry(unique_id.to_owned()) {
            let mut device = o.get_mut();

            info!(slog_scope::logger(), "unsubscribe {:?}.", unique_id);
            let _ = device.unsubscribe()?;
        }
        Err(Error::UnknownDevice)
    }

    /// Renew a subscription for a device. Must happen before the subscription expires.
    pub fn resubscribe(&mut self, unique_id: &str, seconds: u32) -> Result<u32, Error> {

        if !self.subscription_daemon {
            return Err(Error::ServiceNotRunning);
        }

        if seconds < 15 {
            return Err(Error::TimeoutTooShort);
        }

        let mut devices = self.devices.lock().unwrap();
        if let Entry::Occupied(mut o) = devices.entry(unique_id.to_owned()) {
            let mut device = o.get_mut();
            info!(slog_scope::logger(), "resubscribe {:?}.", unique_id);
            let timeout = device.resubscribe(seconds)?;
            return Ok(timeout);
        }
        Err(Error::UnknownDevice)
    }

    /// Subscribe to a device for notifications on state change.
    /// Typically 120-600 seconds. If auto_resubscribe==true the subscription will
    /// be renewed automatically.
    /// Notifications will be returned via the mpsc returned from start_subscription_service.
    pub fn subscribe(&mut self,
                     unique_id: &str,
                     seconds: u32,
                     auto_resubscribe: bool)
                     -> Result<u32, Error> {

        if !self.subscription_daemon {
            return Err(Error::ServiceNotRunning);
        }

        if seconds < 15 {
            return Err(Error::TimeoutTooShort);
        }

        let mut devices = self.devices.lock().unwrap();
        if let Entry::Occupied(mut o) = devices.entry(unique_id.to_owned()) {

            let mut device = o.get_mut();

            info!(slog_scope::logger(), "subscribe {:?}.", unique_id);
            let (_, time) = device.subscribe(DAEMON_PORT, seconds, auto_resubscribe)?;
            return Ok(time);
        }
        Err(Error::UnknownDevice)
    }

    /// Setup the subscription service and retrieve a mpsc on which to get notifications.
    /// Required before subscribing to notifications for devices.
    pub fn start_subscription_service(&mut self)
                                      -> Result<mpsc::Receiver<StateNotification>, Error> {

        // Start daemon listening for subscription updates, if not running.
        if !self.subscription_daemon {
            self.subscription_daemon = true;

            let (tx, rx) = mpsc::channel();
            let devices = self.devices.clone();
            thread::spawn(move || {
                WeeController::subscription_server(tx, devices);
            });
            return Ok(rx);
        }
        Err(Error::ServiceAlreadyRunning)
    }

    // This daemon will run until the process ends, listening for notifications.
    fn subscription_server(tx: mpsc::Sender<StateNotification>,
                           devices: Arc<Mutex<HashMap<String, Device>>>) {
        use std::io;
        use hyper::server::{Server, Request, Response};

        info!(slog_scope::logger(), "Subscription daemon started.");

        let address: String = format!("0.0.0.0:{}", DAEMON_PORT);
        let add_str: &str = &address;
        let cp = Mutex::new(tx.clone()); // clone sender mpsc for the handler.
        Server::http(add_str)
            .unwrap()
            .handle(move |mut req: Request, mut res: Response| {

                let mut data = String::new();
                let _ = req.read_to_string(&mut data);

                // Construct response
                let mut reader: &[u8] = b"<html><body><h1>200 OK</h1></body></html>";
                let len_string = reader.len().to_string();
                use hyper::header::ContentType;
                use hyper::mime::{Mime, TopLevel, SubLevel};
                res.headers_mut().set(ContentType(Mime(TopLevel::Text, SubLevel::Html, vec![])));
                res.headers_mut().set_raw("Content-Length", vec![len_string.into_bytes()]);
                res.headers_mut().set_raw("Connection", vec!["Close".to_string().into_bytes()]);
                io::copy(&mut reader, &mut res.start().unwrap()).unwrap();

                if let Some(notification_sid) = rpc::extract_sid(&req.headers) {

                    if let Some(state) = xml::get_binary_state(&data) {

                        // Iterate through the devices to find the subscribed one
                        for (unique_id, dev) in devices.lock().unwrap().iter() {
                            if let Some(device_sid) = dev.sid() {

                                // Found a match, send notification to the client
                                if notification_sid == device_sid {
                                    info!(slog_scope::logger(),
                                          "Got switch update for {:?}. sid: {:?}. State: {:?}.",
                                          unique_id,
                                          notification_sid,
                                          State::from(state));
                                    let notification = StateNotification {
                                        unique_id: unique_id.to_owned(),
                                        state: State::from(state),
                                    };
                                    let _ = cp.lock().unwrap().send(notification);
                                    break;
                                }
                            }
                        }
                    }
                }
            })
            .unwrap();
    }
}
