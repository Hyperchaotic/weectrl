use hyper::body::HttpBody;
use hyper::service::{make_service_fn, service_fn};
use hyper::{Body, HeaderMap, Request, Response, Server};
use std::convert::Infallible;
use std::net::SocketAddr;
use std::str::FromStr;
use std::time::Duration;
use tracing::info;

use std::collections::hash_map::Entry;
use std::collections::HashMap;
use std::sync::{mpsc, Arc, Mutex};

use std::net::IpAddr;
use std::net::TcpListener;
use std::net::UdpSocket;

use crate::cache::{DeviceAddress, DiskCache};
use crate::device::Device;
use crate::error;
use crate::error::{Error, FATAL_LOCK};
use crate::xml;
pub use mime::Mime;
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
            0 => Self::Off,
            1 => Self::On,
            _ => Self::Unknown,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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

#[derive(Debug, Clone, PartialEq, Eq)]
/// Type of device
pub enum Model {
    Lightswitch,
    Socket,
    Unknown(String),
}

impl std::fmt::Display for Model {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Lightswitch => write!(f, "Light Switch"),
            Self::Socket => write!(f, "Socket"),
            Self::Unknown(str) => write!(f, "{}", str),
        }
    }
}

impl<'a> From<&'a str> for Model {
    fn from(string: &'a str) -> Self {
        match string {
            "LightSwitch" => Self::Lightswitch,
            "Socket" => Self::Socket,
            _ => Self::Unknown(string.to_owned()),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// Represents whether a device binary state is on or off.
pub enum State {
    /// Device is switched on
    On,
    /// Device is switched off
    Off,
    /// Device is not responding to queries or commands
    Unknown,
}

impl std::fmt::Display for State {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match *self {
            Self::On => f.write_str(xml::SETBINARYSTATEON),
            Self::Off => f.write_str(xml::SETBINARYSTATEOFF),
            Self::Unknown => f.write_str("UNKNOWN STATE"),
        }
    }
}

#[derive(Debug, Clone)]
// A WeMo swicth have an associated icon
pub struct Icon {
    pub mimetype: Mime,
    pub width: u64,
    pub height: u64,
    pub depth: u64,
    pub data: Vec<u8>,
}

#[derive(Debug, Clone)]
// Data returned for each device found during discovery.
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
    /// Device information, data from XML homepage
    pub xml: String,
}

/// Controller entity used for finding and controlling Belkin WeMo, and compatible, devices.
pub struct WeeController {
    cache: Arc<Mutex<DiskCache>>,
    devices: Arc<Mutex<HashMap<String, Device>>>,
    notification_mpsc: Arc<Mutex<Option<mpsc::Sender<StateNotification>>>>,
    port: u16,
}

impl Default for WeeController {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for WeeController {
    fn drop(&mut self) {
        info!("Dropping WeeController");
        self.unsubscribe_all();
    }
}

impl WeeController {
    #[must_use]
    pub fn new() -> Self {
        info!("Init");

        // Find a free port number between 8000-9000
        let port = Self::get_available_port().unwrap();
        let notification_mpsc = Arc::new(Mutex::new(None));
        let devices = Arc::new(Mutex::new(HashMap::new()));

        // Start the webserver listeninh for incoming notifications
        let tx = notification_mpsc.clone();
        let devs = devices.clone();
        let _ignore = std::thread::Builder::new()
            .name("SUB_server".to_string())
            .spawn(move || {
                Self::notification_server(tx, devs, port);
            })
            .expect("UNABLE TO START NOTFICATION THREAD");

        // Build self
        Self {
            cache: Arc::new(Mutex::new(DiskCache::new())),
            devices,
            notification_mpsc,
            port,
        }
    }

    /// Forget all known devices, optionalle also clear the on-disk cache.
    pub fn clear(&self, clear_cache: bool) {
        info!("Clearing list of devices. Clear disk; {:?}.", clear_cache);
        self.unsubscribe_all();
        if clear_cache {
            self.cache.lock().expect(FATAL_LOCK).clear();
        }
        self.devices.lock().expect(FATAL_LOCK).clear();
    }

    /// Set the device binary state (On/Off), e.g. toggle a switch.
    pub fn set_binary_state(
        &self,
        unique_id: &str,
        state: State,
        conn_timeout: Duration,
    ) -> Result<State, Error> {
        info!("Set binary state for Device {:?} {:?} ", unique_id, state);
        let mut devices = self.devices.lock().expect(FATAL_LOCK);
        if let Entry::Occupied(mut o) = devices.entry(unique_id.to_owned()) {
            let device = o.get_mut();
            return device.set_binary_state(state, conn_timeout);
        }
        Err(Error::UnknownDevice)
    }

    /// Query the device for the binary state (On/Off)
    pub fn get_binary_state(
        &self,
        unique_id: &str,
        conn_timeout: Duration,
    ) -> Result<State, Error> {
        info!("get_binary_state for device {:?}.", unique_id);
        let mut devices = self.devices.lock().expect(FATAL_LOCK);
        if let Entry::Occupied(mut o) = devices.entry(unique_id.to_owned()) {
            let device = o.get_mut();
            return device.fetch_binary_state(conn_timeout);
        }
        Err(Error::UnknownDevice)
    }

    /// Query the device for a list of icons
    pub fn get_icons(&self, unique_id: &str, conn_timeout: Duration) -> Result<Vec<Icon>, Error> {
        info!("get_icons for device {:?}.", unique_id);
        let mut devices = self.devices.lock().expect(FATAL_LOCK);
        if let Entry::Occupied(mut o) = devices.entry(unique_id.to_owned()) {
            let device = o.get_mut();
            return device.fetch_icons(conn_timeout);
        }
        Err(Error::UnknownDevice)
    }

    /// Cancel subscription for notifications from all devices.
    // @TODO spin off in threads?
    pub fn unsubscribe_all(&self) {
        info!("unsubscribe_all.");
        let mut devices = self.devices.lock().expect(FATAL_LOCK);
        let unique_ids: Vec<String> = devices.keys().cloned().collect();
        let mut handles = Vec::with_capacity(10);

        for unique_id in unique_ids {
            if let Entry::Occupied(mut o) = devices.entry(unique_id.clone()) {
                let mut device = o.get_mut().clone();
                handles.push(std::thread::spawn(move || {
                    info!("unsubscribe {:?}.", unique_id);
                    let _ignore = device.unsubscribe(Duration::from_secs(5));
                }));
            }
        }
        for handle in handles {
            let _ignore = handle.join();
        }
        info!("Done unsubscribe_all.");
    }

    /// Cancel subscription for notifications from a device.
    pub fn unsubscribe(&self, unique_id: &str, conn_timeout: Duration) -> Result<String, Error> {
        let mut devices = self.devices.lock().expect(FATAL_LOCK);
        if let Entry::Occupied(mut o) = devices.entry(unique_id.to_owned()) {
            let device = o.get_mut();

            info!("unsubscribe {:?}.", unique_id);
            return device.unsubscribe(conn_timeout);
        }
        Err(Error::UnknownDevice)
    }

    /// Subscribe to a device for notifications on state change.
    /// Typically 120-600 seconds. If auto_resubscribe==true the subscription will
    /// be renewed automatically.
    /// Notifications will be returned via the mpsc returned from start_subscription_service.
    pub fn subscribe(
        &self,
        unique_id: &str,
        seconds: Duration,
        auto_resubscribe: bool,
        conn_timeout: Duration,
    ) -> Result<u32, Error> {
        if seconds < Duration::from_secs(15) {
            info!("Subscribe: TimeoutTooShort");
            return Err(Error::TimeoutTooShort);
        }

        let mut devices = self.devices.lock().expect(FATAL_LOCK);
        if let Entry::Occupied(mut o) = devices.entry(unique_id.to_owned()) {
            let device = o.get_mut();
            info!("subscribe {:?}.", unique_id);
            let (_, time) = device.subscribe(self.port, seconds, auto_resubscribe, conn_timeout)?;
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
    pub fn discover(
        &self,
        mode: DiscoveryMode,
        forget_devices: bool,
        mx: Duration,
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

                        for cache_entry in cache_list {
                            let device =
                                Self::register_device(&cache_entry.location, &devices).ok();
                            if let Some(new) = device {
                                let _ignore = tx.send(new);
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

                    let bind_address = Self::get_bind_addr().unwrap();
                    let socket = UdpSocket::bind(&bind_address).unwrap();

                    Self::send_sddp_request(&socket, mx);

                    let mut buf = [0u8; 2048];
                    socket
                        .set_read_timeout(Some(mx + Duration::from_secs(1)))
                        .unwrap();

                    let (cache_dirt_tx, cache_dirt_rx) = mpsc::channel();

                    let mut num_threads = 0;
                    while let Ok(e) = socket.recv(&mut buf) {
                        let message = std::str::from_utf8(&buf[..e]).unwrap().to_string();

                        // Handle message in different thread
                        let devs = devices.clone();
                        let nx = tx.clone();
                        let cache_dirt = cache_dirt_tx.clone();
                        num_threads += 1;

                        let _ignore = std::thread::Builder::new()
                            .name(format!("SSDP_handle_msg {}", num_threads).to_string())
                            .spawn(move || {
                                if let Some(location) = Self::parse_ssdp_response(&message) {
                                    let device = Self::register_device(&location, &devs).ok();

                                    device.map_or_else(
                                        || {
                                            let _ignore = cache_dirt.send(false);
                                        },
                                        |new| {
                                            let _ignore = nx.send(new);
                                            let _ignore = cache_dirt.send(true);
                                        },
                                    );
                                }
                            });
                    }

                    for _ in 0..num_threads {
                        if cache_dirt_rx.recv().unwrap() {
                            info!("CACHE DIRTY");
                            cache_dirty = true;
                        }
                    }

                    if cache_dirty {
                        Self::refresh_cache(&cache, &devices);
                    }
                }

                info!("Done! Ending discover thread.");
            });
        rx
    }

    /// Retrieve a mpsc on which to get notifications.
    pub fn notify(&self) -> mpsc::Receiver<StateNotification> {
        let (tx, rx) = mpsc::channel::<StateNotification>();
        let mut notification_mpsc = self.notification_mpsc.lock().expect(FATAL_LOCK);
        *notification_mpsc = Some(tx);
        rx
    }
    /// Setup the subscription service and retrieve a mpsc on which to get notifications.
    /// Required before subscribing to notifications for devices.
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
    fn get_device_home(location: &Url, conn_timeout: Duration) -> Result<String, Error> {
        let response = super::rpc::http_get_text(location, conn_timeout)?;
        Ok(response)
    }

    // Write list of active devices to disk cache
    fn refresh_cache(cache: &Arc<Mutex<DiskCache>>, devices: &Arc<Mutex<HashMap<String, Device>>>) {
        info!("Refreshing cache.");
        let mut list = Vec::new();
        for (mac, dev) in devices.lock().expect(FATAL_LOCK).iter() {
            let location = dev.info.location.clone();
            list.push(DeviceAddress {
                location,
                mac_address: mac.clone(),
            });
        }
        cache.lock().expect(FATAL_LOCK).write(list);
    }

    // Given location URL, query a device. If successful add to list of active devices
    fn retrieve_device(location: &str, conn_timeout: Duration) -> Result<Device, Error> {
        let mut url = Url::from_str(location)?;
        let xml = Self::get_device_home(&url, conn_timeout)?;
        let local_ip = Self::get_local_ip(location)?;
        let root: Root = xml::parse_services(&xml)?;

        info!("Device {:?} {:?} ", root.device.friendly_name, location);

        let mut hostname = String::new();
        //let mut base_url = Url::parse(location)?;
        if let Some(hn) = url.host_str() {
            hostname = hn.to_owned();
        }
        url.set_path("/");

        let model = root.device.model_name.clone();
        let model_str: &str = &model;
        let info = DeviceInfo {
            friendly_name: root.device.friendly_name.clone(),
            model: Model::from(model_str),
            unique_id: root.device.mac_address.clone(),
            hostname,
            base_url: url.to_string(),
            location: location.to_string(),
            state: State::Unknown,
            root,
            xml,
        };
        let mut dev = Device::new(info, local_ip);

        if dev.validate_device()
            && dev
                .fetch_binary_state(Duration::from_secs(5))
                .ok()
                .is_some()
        {
            return Ok(dev);
        }

        info!("Device not supported.");
        Err(Error::UnsupportedDevice)
    }

    // Add device to hashmap.
    fn register_device(
        location: &str,
        devices: &Arc<Mutex<HashMap<String, Device>>>,
    ) -> Result<DeviceInfo, Error> {
        let newdev = Self::retrieve_device(location, Duration::from_secs(5))?;
        let mut devs = devices.lock().expect(FATAL_LOCK);
        let unique_id = newdev.info.root.device.mac_address.clone();

        if let std::collections::hash_map::Entry::Vacant(e) = devs.entry(unique_id) {
            info!("Registering dev: {:?}", newdev.info.unique_id);
            let info = newdev.info.clone();
            e.insert(newdev);
            return Ok(info);
        }
        Err(Error::DeviceAlreadyRegistered)
    }

    async fn http_req_handler(
        notification_mspsc: Arc<Mutex<Option<mpsc::Sender<StateNotification>>>>,
        devices: Arc<Mutex<HashMap<String, Device>>>,
        req: Request<Body>,
    ) -> Result<Response<Body>, Infallible> {
        let (parts, mut stream) = req.into_parts();

        // Retrieve the entire message body
        let mut body = String::new();
        while let Some(d) = stream.data().await {
            if let Ok(s) = d {
                body += std::str::from_utf8(&s).unwrap();
            }
        }

        // Ok full message received, now process it
        if body.len() > 0 && body.find("</e:propertyset>") != None {
            // Extract SID and BinaryState
            if let Some((notification_sid, state)) = Self::get_sid_and_state(&body, &parts.headers)
            {
                info!("str, sid {notification_sid} {}", state);
                // If we have mathing SID, register and forward statechange
                let devs = devices.lock().expect(FATAL_LOCK);
                'search: for (unique_id, dev) in devs.iter() {
                    if let Some(device_sid) = dev.sid() {
                        // Found a match, send notification to the client
                        if notification_sid == device_sid {
                            info!(
                                "Server got switch update for {:?}. sid: {:?}. State: {:?}.",
                                unique_id,
                                notification_sid,
                                State::from(state)
                            );

                            let n = StateNotification {
                                unique_id: unique_id.clone(),
                                state: State::from(state),
                            };

                            let tx = notification_mspsc.lock().expect(FATAL_LOCK);
                            match &*tx {
                                None => info!("No listener, dropping notification."),
                                Some(tx) => {
                                    let _r = tx.send(n);
                                }
                            }
                            break 'search;
                        }
                    }
                }
            }
        }

        Ok(Response::new(Body::from("")))
    }

    #[tokio::main()]
    async fn notification_server(
        notification_mspsc: Arc<Mutex<Option<mpsc::Sender<StateNotification>>>>,
        devices: Arc<Mutex<HashMap<String, Device>>>,
        port: u16,
    ) {
        // A `MakeService` that produces a `Service` to handle each connection.
        let make_service = make_service_fn(move |_| {
            let notification_mspsc = notification_mspsc.clone();
            let devices = devices.clone();

            // Create a `Service` for responding to the request.
            let service = service_fn(move |req| {
                Self::http_req_handler(notification_mspsc.clone(), devices.clone(), req)
            });

            // Return the service to hyper.
            async move { Ok::<_, Infallible>(service) }
        });

        let addr: SocketAddr = ([0, 0, 0, 0], port).into();

        info!("Listening to notifications on: {:?}", addr.to_string());

        let server = Server::bind(&addr).serve(make_service);

        if let Err(e) = server.await {
            info!("server error: {}", e);
        }
    }

    fn get_sid_and_state(body: &str, headers: &HeaderMap) -> Option<(String, u8)> {
        let mut sid = None;

        let hsid = headers.get("SID");
        if let Some(h) = hsid {
            if let Ok(f) = h.to_str() {
                sid = Some(f.to_string());
            }
        }

        let state = xml::get_binary_state(body);

        if sid != None && state != None {
            return Some((sid.unwrap(), state.unwrap()));
        }
        None
    }

    fn port_is_available(port: u16) -> bool {
        TcpListener::bind(("127.0.0.1", port)).is_ok()
    }

    fn get_available_port() -> Option<u16> {
        (8000..9000).find(|port| Self::port_is_available(*port))
    }

    // Bind through a connected interface
    fn get_bind_addr() -> Result<SocketAddr, std::io::Error> {
        let any: SocketAddr = ([0, 0, 0, 0], 0).into();
        let dns: SocketAddr = ([1, 1, 1, 1], 80).into();
        let socket = UdpSocket::bind(any)?;
        socket.connect(dns)?;
        let bind_addr = socket.local_addr()?;

        Ok(bind_addr)
    }

    fn parse_ssdp_response(response: &str) -> Option<String> {
        for line in response.lines() {
            let line = line.to_lowercase();
            if line.contains("location") {
                if let Some(idx) = line.find("http") {
                    let url = &line[idx..line.len()];
                    return Some(url.trim().to_string());
                }
            }
        }
        None
    }

    fn send_sddp_request(socket: &UdpSocket, mx: Duration) {
        let msg = format!(
            "M-SEARCH * HTTP/1.1\r
Host: 239.255.255.250:1900\r
Man: \"ssdp:discover\"\r
ST: upnp:rootdevice\r
MX: {}\r
Content-Length: 0\r\n\r\n",
            mx.as_secs()
        );

        let broadcast_address: SocketAddr = ([239, 255, 255, 250], 1900).into();
        let _ = socket.set_broadcast(true);
        socket.send_to(msg.as_bytes(), &broadcast_address).unwrap();
    }
}
