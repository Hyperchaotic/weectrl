use std::time::Duration;
use tracing::info;

use std::collections::hash_map::Entry;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};

use std::net::UdpSocket;
use std::net::{IpAddr, SocketAddr};
use std::{io::prelude::*, net::TcpListener};

use crate::cache::{DeviceAddress, DiskCache};
use crate::device::Device;
use crate::error;
use crate::error::{Error, FATAL_LOCK};
use crate::xml;
use url::Url;
use xml::Root;


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

#[derive(Debug, Clone, PartialEq)]
pub enum Model {
    Lightswitch,
    Socket,
    Unknown(String),
}

impl<'a> From<&'a str> for Model {
    fn from(string: &'a str) -> Model {
        match string {
            "LightSwitch" => Model::Lightswitch,
            "Socket" => Model::Socket,
            _ => Model::Unknown(string.to_owned()),
        }
    }
}

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
pub struct Icon {
    pub mimetype: String,
    pub width: u64,
    pub height: u64,
    pub depth: u64,
    pub data: Vec<u8>,
}

#[derive(Debug, Clone)]
// Object returned for each device found during discovery.
pub struct DeviceInfo {
    /// Human readable name returned from the device homepage.
    pub friendly_name: String,
    /// Basic model name, e.g. LightSwitch, Socket.
    pub model: Model,
    /// Unique identifier for this device, used for issuing commands to controller.
    pub unique_id: String,
    /// Network hostname, usually the IP address.
    pub hostname: String,
    /// http address including port number
    pub base_url: String,
    /// Device "home" URL
    pub location: String,
    /// Current switch binarystate
    pub state: State,
    /// Device information, from XML homepage
    pub root: Root,
}

/// Controller entity used for finding and control Belkin WeMo, and compatible, devices.
pub struct WeeController {
    cache: Arc<Mutex<DiskCache>>,
    devices: Arc<Mutex<HashMap<String, Device>>>,
    subscription_daemon: Arc<AtomicBool>,
    port: u16,
}

impl WeeController {
    pub fn new() -> WeeController {
        // Find a free port number between 8000-9000
        let available_port = WeeController::get_available_port().unwrap();

        let object = WeeController {
            cache: Arc::new(Mutex::new(DiskCache::new())),
            devices: Arc::new(Mutex::new(HashMap::new())),
            subscription_daemon: Arc::new(AtomicBool::new(false)),
            port: available_port,
        };

        info!("Init");
        object
    }

    // Get the IP address of this PC, from the same interface talking to the device.
    // It will be needed if subscriptions are used. It's not pretty but easiest way to Make
    // sure we can talk to multiple devices on separate interfaces.
    fn get_local_ip(location: &str) -> Result<IpAddr, Error> {
        use std::net::TcpStream;

        // extract ip:port from URL
        let location_url = Url::parse(location)?;
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

    // Retrieve a device home page.
    fn get_device_home(location: &str) -> Result<String, Error> {
        let res = reqwest::blocking::get(location)?;
        let body = res.text()?;
        Ok(body)
    }

    // Write list of active devices to disk cache
    fn refresh_cache(cache: Arc<Mutex<DiskCache>>, devices: Arc<Mutex<HashMap<String, Device>>>) {
        info!("Refreshing cache.");
        let mut list = Vec::new();
        for (mac, dev) in devices.lock().expect(FATAL_LOCK).iter() {
            let location = dev.info.location.clone();
            list.push(DeviceAddress {
                location: location,
                mac_address: mac.clone(),
            })
        }
        cache.lock().expect(FATAL_LOCK).write(list);
    }

    // Given location URL, query a device. If successful add to list of active devices
    fn retrieve_device(location: &str) -> Result<Device, Error> {
        let body = WeeController::get_device_home(location)?;
        let local_ip = WeeController::get_local_ip(location)?;
        let root: Root = xml::parse_services(&body)?;

        info!("Device {:?} {:?} ", root.device.friendly_name, location);

        let mut hostname = String::new();
        let mut base_url = Url::parse(location)?;
        if let Some(hn) = base_url.host_str() {
            hostname = hn.to_owned();
        }
        base_url.set_path("/");

        let model = root.device.model_name.clone();
        let model_str: &str = &model;
        let info = DeviceInfo {
            friendly_name: root.device.friendly_name.to_owned(),
            model: Model::from(model_str),
            unique_id: root.device.mac_address.to_owned(),
            hostname: hostname,
            base_url: base_url.to_string(),
            location: location.to_owned(),
            state: State::Unknown,
            root: root,
        };
        let mut dev = Device::new(info, local_ip);

        if dev.validate_device() {
            if dev.fetch_binary_state().ok().is_some() {
                return Ok(dev);
            }
        }

        info!("Device not supported.");
        Err(Error::UnsupportedDevice)
    }

    // Add device to hashmap.
    fn register_device(
        location: &str,
        devices: &Arc<Mutex<HashMap<String, Device>>>,
    ) -> Result<DeviceInfo, Error> {
        let newdev = WeeController::retrieve_device(location)?;
        let mut devs = devices.lock().expect(FATAL_LOCK);
        let unique_id = newdev.info.root.device.mac_address.clone();

        if !devs.contains_key(&unique_id) {
            info!("Registering dev: {:?}", newdev.sid());
            let info = newdev.info.clone();
            devs.insert(unique_id.to_owned(), newdev);
            return Ok(info);
        }
        Err(Error::DeviceAlreadyRegistered)
    }

    /// Retrieve and query devices. First read list stored on disk, then Broadcast
    /// network query and wait for responses. Allow devices max `mx` seconds to respond.
    /// This function is synchronous and will return everything found after mx seconds and a bit.
    /// For an asynchronous version use `discover_async`.
    /// `forget_devices` = true will clear the internal list of devices. Discovery will only
    /// return devices to the client not already known internally.
    pub fn discover(
        &mut self,
        mode: DiscoveryMode,
        forget_devices: bool,
        mx: u8,
    ) -> Option<Vec<DeviceInfo>> {
        let receiver = self.discover_async(mode, forget_devices, mx);
        let mut list = Vec::new();

        loop {
            if let Some(device) = receiver.recv().ok() {
                list.push(device);
            } else {
                break;
            }
        }

        if list.is_empty() {
            None
        } else {
            Some(list)
        }
    }

    /// Clear registered devices, optionally also disk cache
    pub fn clear(&mut self, clear_cache: bool) {
        info!("Clearing list of devices. Clear disk; {:?}.", clear_cache);
        self.unsubscribe_all();
        if clear_cache {
            self.cache.lock().expect(FATAL_LOCK).clear();
        }
        self.devices.lock().expect(FATAL_LOCK).clear();
    }

    /// Set the device BinaryState (On/Off)
    pub fn set_binary_state(&mut self, unique_id: &str, state: State) -> Result<State, Error> {
        info!("Set binary state for Device {:?} {:?} ", unique_id, state);
        let mut devices = self.devices.lock().expect(FATAL_LOCK);
        if let Entry::Occupied(mut o) = devices.entry(unique_id.to_owned()) {
            let device = o.get_mut();
            return device.set_binary_state(state);
        }
        Err(Error::UnknownDevice)
    }

    /// Query the device for BinaryState (On/Off)
    pub fn get_binary_state(&mut self, unique_id: &str) -> Result<State, Error> {
        info!("get_binary_state for device {:?}.", unique_id);
        let mut devices = self.devices.lock().expect(FATAL_LOCK);
        if let Entry::Occupied(mut o) = devices.entry(unique_id.to_owned()) {
            let device = o.get_mut();
            return device.fetch_binary_state();
        }
        Err(Error::UnknownDevice)
    }

    /// Query the device for BinaryState (On/Off)
    pub fn get_icons(&mut self, unique_id: &str) -> Result<Vec<Icon>, Error> {
        info!("get_icons for device {:?}.", unique_id);
        let mut devices = self.devices.lock().expect(FATAL_LOCK);
        if let Entry::Occupied(mut o) = devices.entry(unique_id.to_owned()) {
            let device = o.get_mut();
            return device.fetch_icons();
        }
        Err(Error::UnknownDevice)
    }

    /// Cancel subscription for notifications from all devices.
    /// Make sure to cancel all subscriptions before dropping controller.
    pub fn unsubscribe_all(&mut self) {
        info!("unsubscribe_all.");
        let mut devices = self.devices.lock().expect(FATAL_LOCK);
        let unique_ids: Vec<String> = devices.keys().map(|d| d.clone()).collect();
        for unique_id in unique_ids {
            if let Entry::Occupied(mut o) = devices.entry(unique_id.clone()) {
                let device = o.get_mut();
                info!("unsubscribe {:?}.", unique_id);
                let _ = device.unsubscribe();
            }
        }
    }

    /// Cancel subscription for notifications from a device.
    pub fn unsubscribe(&mut self, unique_id: &str) -> Result<(), Error> {
        if !self.subscription_daemon.load(Ordering::Relaxed) {
            return Err(Error::ServiceNotRunning);
        }

        let mut devices = self.devices.lock().expect(FATAL_LOCK);
        if let Entry::Occupied(mut o) = devices.entry(unique_id.to_owned()) {
            let device = o.get_mut();

            info!("unsubscribe {:?}.", unique_id);
            let _ = device.unsubscribe()?;
        }
        Err(Error::UnknownDevice)
    }

    /// Subscribe to a device for notifications on state change.
    /// Typically 120-600 seconds. If auto_resubscribe==true the subscription will
    /// be renewed automatically.
    /// Notifications will be returned via the mpsc returned from start_subscription_service.
    pub fn subscribe(
        &mut self,
        unique_id: &str,
        seconds: u32,
        auto_resubscribe: bool,
    ) -> Result<u32, Error> {
        if !self.subscription_daemon.load(Ordering::Relaxed) {
            info!("Subscribe: ServiceNotRunning");
            return Err(Error::ServiceNotRunning);
        }

        if seconds < 15 {
            info!("Subscribe: TimeoutTooShort");
            return Err(Error::TimeoutTooShort);
        }

        let mut devices = self.devices.lock().expect(FATAL_LOCK);
        if let Entry::Occupied(mut o) = devices.entry(unique_id.to_owned()) {
            let device = o.get_mut();
            info!("subscribe {:?}.", unique_id);
            let (_, time) = device.subscribe(self.port, seconds, auto_resubscribe)?;
            return Ok(time);
        }
        info!("Subscribe: UnknownDevice");
        Err(Error::UnknownDevice)
    }

    /// Read list of know devices from disk cache as well as new devices responding on the network.
    /// Returns immediately and send discovered devices back on the mpsc as they're found.
    /// Allow network devices max `mx` seconds to respond.
    /// When discovery ends, after mx seconds and a bit, the channel will be closed.
    /// `forget_devices` = true will clear the internal list of devices.
    pub fn discover_async(
        &mut self,
        mode: DiscoveryMode,
        forget_devices: bool,
        mx: u8,
    ) -> mpsc::Receiver<DeviceInfo> {
        if forget_devices {
            self.clear(false);
        }

        let (tx, rx) = mpsc::channel();
        let devices = self.devices.clone();
        let cache = self.cache.clone();

        let _ = std::thread::Builder::new()
            .name("SSDP_main".to_string())
            .spawn(move || {
                info!("Starting discover");

                if mode == DiscoveryMode::CacheOnly || mode == DiscoveryMode::CacheAndBroadcast {
                    info!("Loading devices from cache.");
                    if let Some(cache_list) = cache.lock().expect(error::FATAL_LOCK).read() {
                        info!("Cached devices {:?}", cache_list);

                        for cache_entry in cache_list.into_iter() {
                            let device =
                                WeeController::register_device(&cache_entry.location, &devices)
                                    .ok();
                            if let Some(new) = device {
                                let _ = tx.send(new);
                            }
                        }
                    }
                }

                if mode == DiscoveryMode::BroadcastOnly || mode == DiscoveryMode::CacheAndBroadcast
                {
                    info!("Broadcasting uPnP query.");
                    // Create Our Search Request

                    let devices = devices.clone(); // to move into the new thread
                    let mut cache_dirty = false;

                    let bind_address = WeeController::get_bind_addr().unwrap();
                    let socket = UdpSocket::bind(&bind_address).unwrap();

                    WeeController::send_sddp_request(&socket, mx);

                    let mut buf = [0u8; 2048];
                    socket
                        .set_read_timeout(Some(Duration::from_secs((mx + 1) as u64)))
                        .unwrap();

                    let (cache_dirt_tx, cache_dirt_rx) = mpsc::channel();

                    let mut num_threads = 0;
                    while let Ok(e) = socket.recv(&mut buf) {
                        let message = std::str::from_utf8(&buf[..e]).unwrap().to_string();

                        // Handle message in different thread
                        let devs = devices.clone();
                        let nx = tx.clone();
                        let cache_dirt = cache_dirt_tx.clone();
                        num_threads = num_threads + 1;

                        let _ = std::thread::Builder::new()
                            .name(format!("SSDP_handle_msg {}", num_threads).to_string())
                            .spawn(move || {
                                if let Some(location) = WeeController::parse_ssdp_response(&message)
                                {
                                    let device =
                                        WeeController::register_device(&location, &devs).ok();

                                    if let Some(new) = device {
                                        let _ = nx.send(new);
                                        let _ = cache_dirt.send(true);
                                    } else {
                                        let _ = cache_dirt.send(false);
                                    }
                                }
                            });
                    }

                    for _ in 0..num_threads {
                        if cache_dirt_rx.recv().unwrap() == true {
                            info!("CACHE DIRTY");
                            cache_dirty = true;
                        }
                    }

                    if cache_dirty {
                        WeeController::refresh_cache(cache, devices);
                    }
                }

                info!("Done! Ending discover thread.");
            });
        rx
    }

    /// Setup the subscription service and retrieve a mpsc on which to get notifications.
    /// Required before subscribing to notifications for devices.
    pub fn start_subscription_service(
        &mut self,
    ) -> Result<mpsc::Receiver<StateNotification>, error::Error> {
        let mut res = Err(Error::ServiceAlreadyRunning);
        let mutex = Mutex::new(0);
        {
            let _ = mutex.lock().unwrap();
            // Start daemon listening for subscription updates, if not running.
            if !self.subscription_daemon.load(Ordering::Relaxed) {
                let port = self.port;
                let (tx, rx) = mpsc::channel();
                let devices = self.devices.clone();

                let _ = std::thread::Builder::new()
                    .name("SUB_server".to_string())
                    .spawn(move || {
                        WeeController::subscription_server(tx, devices, port);
                    });
                self.subscription_daemon.store(true, Ordering::Relaxed);
                res = Ok(rx);
            }
        }
        res
    }

    // This daemon will run until the process ends, listening for notifications.
    fn subscription_server(
        tx: mpsc::Sender<StateNotification>,
        devices: Arc<Mutex<HashMap<String, Device>>>,
        port: u16,
    ) {
        let addr: SocketAddr = ([0, 0, 0, 0], port).into();

        info!("Listening to notifications on: {:?}", addr.to_string());

        let listener = TcpListener::bind(&addr).unwrap();
        let mut thread_count: u64 = 0;

        for stream in listener.incoming() {
            let mut stream2;
            match stream {
                Ok(stream) => stream2 = stream,
                Err(e) => {
                    info!("Subscription server ended {:?}", e);
                    break;
                }
            };

            // Just for debug info
            thread_count = thread_count + 1;
            let thread_number = thread_count;

            let devices = devices.clone();
            let tx = tx.clone();

            //Spawning off a thread for the connection.
            let _ = std::thread::Builder::new()
            .name("SUBSRV_handle_msg".to_string())
            .spawn(move || {
                let mut full_message = String::new();

                'read_message: loop {
                    let mut buffer = [0; 200];
                    let size = stream2.read(&mut buffer).unwrap();
                    if size == 0 {
                        break 'read_message;
                    }
                    let str = String::from_utf8_lossy(&buffer[0..size]).to_string();
                    full_message.push_str(&str);

                    // ok full message received, now process it
                    if str.find("</e:propertyset>") != None {
                        // Extract SID and BinaryState
                        if let Some((notification_sid, state)) =
                            WeeController::get_sid_and_state(&full_message)
                        {
                            // If we have mathing SID, register and forward statechange
                            'search: for (unique_id, dev) in
                                devices.lock().expect("FATAL_LOCK").iter()
                            {
                                if let Some(device_sid) = dev.sid() {
                                    // Found a match, send notification to the client
                                    if notification_sid == device_sid {
                                        info!(
                                            "Server thread {}. Got switch update for {:?}. sid: {:?}. State: {:?}.",
                                            thread_number,
                                            unique_id,
                                            notification_sid,
                                            State::from(state)
                                        );

                                        let n = StateNotification {
                                            unique_id: unique_id.to_owned(),
                                            state: State::from(state),
                                        };

                                        let _r = tx.send(n);

                                        break 'search;
                                    }
                                }
                            }
                        }

                        let status_line = "HTTP/1.1 200 OK";
                        let response = format!("{status_line}\r\ncontent-Length: 0\r\ncontent-type: text/html; charset=utf-8\r\nconnection: close\r\n");

                        stream2.write_all(response.as_bytes()).unwrap();
                        break 'read_message;
                    }
                }
            });
        }
    }

    fn get_sid_and_state(message: &str) -> Option<(String, u8)> {
        let mut sid = None;
        for line in message.lines() {
            if line.starts_with("SID: ") {
                sid = Some(line.to_string().split_off(5));
                break;
            }
        }

        let state = xml::get_binary_state(&message);

        if sid != None && state != None {
            return Some((sid.unwrap(), state.unwrap()));
        }
        None
    }

    fn port_is_available(port: u16) -> bool {
        match TcpListener::bind(("127.0.0.1", port)) {
            Ok(_) => true,
            Err(_) => false,
        }
    }

    fn get_available_port() -> Option<u16> {
        (8000..9000).find(|port| WeeController::port_is_available(*port))
    }

    // Bind through a connected interface
    fn get_bind_addr() -> Result<SocketAddr, std::io::Error> {
        let any: SocketAddr = ([0, 0, 0, 0], 0).into();
        let dns: SocketAddr = ([1, 1, 1, 1], 80).into();
        let socket = UdpSocket::bind(any)?;
        let _ = socket.connect(dns);
        let bind_addr = socket.local_addr()?;

        Ok(bind_addr)
    }

    fn parse_ssdp_response(response: &str) -> Option<String> {
        if response.find("HTTP/1.1 200 OK") == None {
            return None;
        }

        for line in response.lines() {
            let mut split = line.splitn(2, ':');

            if let Some(y) = split.next() {
                if y.eq_ignore_ascii_case("location") {
                    if let Some(location) = split.next() {
                        return Some(location.trim().to_string());
                    } else {
                        break;
                    }
                } else {
                    let _ = split.next();
                }
            } else {
                break;
            }
        }
        None
    }

    fn send_sddp_request(socket: &UdpSocket, mx: u8) {
        let msg = format!(
            "M-SEARCH * HTTP/1.1\r
Host:239.255.255.250:1900\r
Man:\"ssdp:discover\"\r
ST: upnp:rootdevice\r
MX: {}\r\n\r\n",
            mx
        );

        let broadcast_address: SocketAddr = ([239, 255, 255, 250], 1900).into();
        socket.send_to(msg.as_bytes(), &broadcast_address).unwrap();
    }
}
