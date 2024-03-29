// On Windows don't create terminal window when opening app in release mode GUI
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

extern crate weectrl;

use fltk::app::MouseButton;
use tracing::info;

use weectrl::{DeviceInfo, DiscoveryMode, State, StateNotification, WeeController};

use fltk::{enums::Color, enums::Event, image::PngImage, image::SvgImage, prelude::*, *};
use fltk_theme::widget_schemes::fluent::colors::*;
use fltk_theme::{SchemeType, WidgetScheme};
use std::collections::HashMap;
use std::sync::mpsc::TryRecvError;
use std::time::Duration;

extern crate directories;
use directories::ProjectDirs;

use serde::{Deserialize, Serialize};

use std::fs::File;
use std::io::prelude::*;

#[derive(Debug, Clone)]
enum Message {
    Reload,
    Clear,
    StartDiscovery,
    PollDiscovery,
    EndDiscovery,
    AddButton(DeviceInfo),
    Notification(StateNotification),
    Clicked(DeviceInfo),
}

struct WeeApp {
    app: app::App,
    scroll: group::Scroll,
    pack: group::Pack,
    reloading_frame: frame::Frame,
    progress_frame: frame::Frame,
    sender: app::Sender<Message>,
    receiver: app::Receiver<Message>,
    controller: WeeController,
    discovering: bool,                        // Is discover currently in progress?
    buttons: HashMap<String, button::Button>, // List of deviceID's and indexes of the associated buttons
}

const SETTINGS_FILE: &str = "Settings.json";

const SUBSCRIPTION_DURATION: Duration = Duration::from_secs(180);
const SUBSCRIPTION_AUTO_RENEW: bool = true;
const DISCOVERY_MX: Duration = Duration::from_secs(5);
const CONN_TIMEOUT: Duration = Duration::from_secs(5);

const UNIT_SPACING: i32 = 40;
const BUTTON_HEIGHT: i32 = UNIT_SPACING;
const WINDOW_WIDTH: i32 = 350;
const TOP_BAR_HEIGHT: i32 = UNIT_SPACING + 10;
const LIST_HEIGHT: i32 = 170;
const WINDOW_HEIGHT: i32 = LIST_HEIGHT + TOP_BAR_HEIGHT + UNIT_SPACING;
const SCROLL_WIDTH: i32 = 15;

const BUTTON_ON_COLOR: Color = Color::from_rgb(114, 159, 207);
const BUTTON_OFF_COLOR: Color = Color::from_rgb(13, 25, 38);

const WINDOW_ICON: &[u8] = include_bytes!("images/titlebar.png");

const PROGRESS: &str = include_str!("images/progress.svg");

const RL_BTN1: &str = include_str!("images/refresh.svg");
const RL_BTN2: &str = include_str!("images/refresh-press.svg");
const RL_BTN3: &str = include_str!("images/refresh-hover.svg");

const CL_BTN1: &str = include_str!("images/clear.svg");
const CL_BTN2: &str = include_str!("images/clear-press.svg");
const CL_BTN3: &str = include_str!("images/clear-hover.svg");

const CLEAR_TOOLTIP: &str = "Forget all devices and clear the on-disk list of known devices.";
const RELOAD_TOOLTIP: &str =
    "Reload list of devices from on-disk list (if any) and then by network query.";
const SWITCH_TOOLTIP: &str = "Left click to toggle switch. Right click for additional information.";

macro_rules! read_image {
    ($data:ident) => {{
        let mut img = SvgImage::from_data($data).unwrap();
        img.scale(30, 30, true, true);
        img
    }};
}

// Builder for the information dialogue.
macro_rules! build_field {
    ($parent:ident, $name:expr, $string:expr) => {
        let st = format!("{}:  {:}", $name, $string);
        let mut name = Box::new(output::Output::default().with_size(0, 20));
        name.set_value(&st);
        name.set_color(Color::from_hex(0x2e3436));
        name.set_label_font(enums::Font::Helvetica);
        name.set_frame(enums::FrameType::FlatBox);
        name.set_text_size(16);
        $parent.add(&*name);
    };
}

impl WeeApp {
    pub fn new() -> Self {
        let app = app::App::default();
        app::background(0, 0, 0);
        app::background2(0x00, 0x00, 0x00);
        app::foreground(0xff, 0xff, 0xff);
        app::set_color(
            Color::Selection,
            SELECTION_COLOR.0,
            SELECTION_COLOR.1,
            SELECTION_COLOR.2,
        );

        let theme = WidgetScheme::new(SchemeType::Fluent);
        theme.apply();

        app::set_font_size(18);

        let (sender, receiver) = app::channel();

        // get version number from Cargp.toml
        let version = env!("CARGO_PKG_VERSION");

        // Create main Application window. Double buffered.
        let mut main_win =
            window::DoubleWindow::default().with_label(&format!("WeeApp {}", version));

        let args: Vec<String> = std::env::args().collect();

        // Option -r resets window to default size
        let storage = Storage::new();
        if args.len() == 2 && args[1] == "-r" {
            storage.clear();
        }

        // Set app size/position to saved values, or use defaults
        // If no position is set the OS decides.
        let settings = storage.read();
        if let Some(settings) = settings {
            main_win.set_size(settings.w, settings.h);
            main_win.set_pos(settings.x, settings.y);

        } else {
            main_win.set_size(WINDOW_WIDTH, WINDOW_HEIGHT);
        }

        main_win.set_color(Color::Gray0);

        main_win.set_icon(Some(PngImage::from_data(WINDOW_ICON).unwrap()));

        // Load all the images for clear/reload buttons
        let image_clear = read_image!(CL_BTN1);
        let image_clear_click = read_image!(CL_BTN2);
        let image_clear_hover = read_image!(CL_BTN3);

        let mut btn_clear = button::Button::default().with_size(50, 50);
        btn_clear.set_frame(enums::FrameType::FlatBox);
        btn_clear.set_pos(main_win.w() - UNIT_SPACING - 10 - UNIT_SPACING - 10, 0);
        btn_clear.set_image(Some(image_clear.clone()));

        let ic = image_clear.clone();
        let icc = image_clear_click.clone();
        let ich = image_clear_hover.clone();
        btn_clear.emit(sender.clone(), Message::Clear);
        btn_clear.set_tooltip(CLEAR_TOOLTIP);

        btn_clear.handle(move |b, e| match e {
            Event::Enter | Event::Released => {
                b.set_image(Some(ich.clone()));
                b.redraw();
                true
            }
            Event::Leave => {
                b.set_image(Some(ic.clone()));
                b.redraw();
                true
            }
            Event::Push => {
                b.set_image(Some(icc.clone()));
                true
            }
            _ => false,
        });

        let image_reload = read_image!(RL_BTN1);
        let image_reload_click = read_image!(RL_BTN2);
        let image_reload_hover = read_image!(RL_BTN3);

        let mut btn_reload = button::Button::default().with_size(50, 50);
        btn_reload.set_frame(enums::FrameType::FlatBox);
        btn_reload.set_pos(main_win.w() - UNIT_SPACING - 10, 0);
        btn_reload.set_image(Some(image_reload.clone()));

        let ir = image_reload.clone();
        let irc = image_reload_click.clone();
        let irh = image_reload_hover.clone();
        btn_reload.set_tooltip(RELOAD_TOOLTIP);
        btn_reload.emit(sender.clone(), Message::Reload);

        btn_reload.handle(move |b, e| match e {
            Event::Enter | Event::Released => {
                b.set_image(Some(irh.clone()));
                b.redraw();
                true
            }
            Event::Leave => {
                b.set_image(Some(ir.clone()));
                b.redraw();
                true
            }
            Event::Push => {
                b.set_image(Some(irc.clone()));
                true
            }
            _ => false,
        });

        let mut scroll = group::Scroll::new(
            UNIT_SPACING,
            TOP_BAR_HEIGHT,
            main_win.w() - 2 * UNIT_SPACING,
            main_win.h() - 2 * UNIT_SPACING - 5,
            "",
        );

        scroll.set_frame(enums::FrameType::BorderBox);
        scroll.set_type(group::ScrollType::VerticalAlways);
        scroll.make_resizable(false);
        scroll.set_color(Color::from_hex(0x2e3436));
        scroll.set_scrollbar_size(SCROLL_WIDTH);

        let mut pack = group::Pack::default()
            .with_size(scroll.w() - SCROLL_WIDTH, scroll.h())
            .center_of(&scroll);

        pack.set_type(group::PackType::Vertical);
        pack.set_spacing(2);
        pack.set_color(Color::Red);

        pack.end();

        main_win.end();

        // The part that says "Searching" when looking for new switches on the LAN
        let mut reloading_frame = frame::Frame::new(
            UNIT_SPACING,
            main_win.h() - UNIT_SPACING,
            90,
            UNIT_SPACING,
            "Searching",
        );

        reloading_frame.set_label_color(Color::Yellow);
        main_win.add(&reloading_frame);

        // The rotating imagee when searching on the LAN
        let mut progress_frame = frame::Frame::default().with_size(16, 16);
        progress_frame.set_pos(90 + UNIT_SPACING + 5, main_win.h() - 27);

        main_win.add(&progress_frame);

        let mut sc = scroll.clone();
        let mut pa = pack.clone();
        let mut reload = btn_reload.clone();
        let mut clear = btn_clear.clone();
        let mut rlfr = reloading_frame.clone();
        let mut prfr = progress_frame.clone();
        let controller = WeeController::new();

        main_win.handle(move |w, ev| match ev {
            // When quitting the App save Windows size/position
            Event::Hide => {
                let settings = Settings {
                    x: w.x(),
                    y: w.y(),
                    w: w.w(),
                    h: w.h(),
                };
                let storage = Storage::new();
                storage.write(settings);
                true
            }

            Event::Show => {
                info!("Event::Show");

                //Sometimes the buttons would mysteriously be double height
                //Trying this hack to mitigate
                for i in 0..pa.children() {
                    let mut btn = pa.child(i).unwrap();
                    btn.set_size(pa.w(), BUTTON_HEIGHT);
                }

                true
            }

            // When resizing the App window, reposition internal elements accordingly
            Event::Resize => {
                info!("Event::Resize");

                sc.resize(
                    UNIT_SPACING,
                    TOP_BAR_HEIGHT,
                    w.w() - 2 * UNIT_SPACING,
                    w.h() - 2 * UNIT_SPACING - 5,
                );

                pa.resize(sc.x(), sc.y(), sc.w() - SCROLL_WIDTH, sc.h());

                reload.set_size(50, 50);
                reload.set_pos(w.w() - UNIT_SPACING - 10, 0);

                clear.set_size(50, 50);
                clear.set_pos(w.w() - UNIT_SPACING - 10 - UNIT_SPACING - 10, 0);

                rlfr.set_size(90, UNIT_SPACING);
                rlfr.set_pos(UNIT_SPACING, w.h() - UNIT_SPACING);

                prfr.set_size(16, 16);
                prfr.set_pos(90 + UNIT_SPACING + 5, w.h() - 27);

                //Sometimes the buttons would mysteriously be the wrong height
                //Trying this hack to mitigate
                for i in 0..pa.children() {
                    let mut btn = pa.child(i).unwrap();
                    btn.set_size(pa.w(), BUTTON_HEIGHT);
                }
                true
            }
            _ => false,
        });

        main_win.make_resizable(true);

        // Create thread to receive notifications from switches that have changed state
        // It will forward the messages to the UI message loop so it can update the button color
        let rx = controller.notify();
        let sc = sender.clone();
        let _ignore = std::thread::Builder::new()
            .name("APP_notifiy".to_string())
            .spawn(move || {
                while let Ok(notification) = rx.recv() {
                    sc.send(Message::Notification(notification));
                }
            })
            .unwrap();

        main_win.show();

        // Start looking for switches, through the UI message loop
        sender.send(Message::StartDiscovery);

        Self {
            app,
            pack,
            scroll,
            reloading_frame,
            progress_frame,
            sender,
            receiver,
            controller,
            discovering: true,
            buttons: HashMap::new(),
        }
    }

    fn show_popup(device: &DeviceInfo, icons: Option<Vec<weectrl::Icon>>) {
        const WIND_WIDTH: i32 = 500;
        const WIND_HEIGHT: i32 = 500;
        const PADDING: i32 = 15;
        const TAB_WIDTH: i32 = WIND_WIDTH - 2 * PADDING;
        const TAB_HEIGHT: i32 = WIND_WIDTH - 2 * PADDING;
        const GROUP_HEIGHT: i32 = TAB_HEIGHT - 25;

        let mut window = window::Window::default().with_label(&device.friendly_name);
        window.set_size(WIND_WIDTH, WIND_HEIGHT);
        window.set_icon(Some(PngImage::from_data(WINDOW_ICON).unwrap()));

        let tab = group::Tabs::new(PADDING, PADDING, TAB_WIDTH, TAB_HEIGHT, "");

        let grp1 = group::Group::new(PADDING, PADDING + 25, TAB_WIDTH, GROUP_HEIGHT, "Info\t\t");

        let mut pack = group::Pack::default()
            .with_pos(25, 80)
            .with_size(grp1.w() - 25, grp1.h() - 150);

        pack.set_type(group::PackType::Vertical);
        pack.set_spacing(2);
        pack.set_color(Color::Red);
        pack.set_spacing(5);

        pack.end();
        grp1.end();

        if let Some(icons) = icons {
            let mut frame = frame::Frame::default().with_size(200, 200).center_of(&pack);
            frame.set_frame(enums::FrameType::FlatBox);
            frame.set_color(Color::from_hex(0x2e3436));

            let icon = icons.get(0).unwrap();

            if icon.mimetype == mime::IMAGE_PNG {
                if let Ok(img) = PngImage::from_data(icon.data.as_slice()) {
                    frame.set_image(Some(img));
                }
            } else if icon.mimetype == mime::IMAGE_JPEG {
                if let Ok(img) = image::JpegImage::from_data(icon.data.as_slice()) {
                    frame.set_image(Some(img));
                }
            } else if icon.mimetype == mime::IMAGE_GIF {
                if let Ok(img) = image::GifImage::from_data(icon.data.as_slice()) {
                    frame.set_image(Some(img));
                }
            }
            pack.add(&frame);
        }

        let spacer = frame::Frame::new(0, 0, 0, 30, "  ");
        pack.add(&spacer);
        build_field!(pack, "          Name", &device.friendly_name);
        build_field!(pack, "          Model", &device.model);
        build_field!(pack, "          Hostname", &device.hostname);
        build_field!(pack, "          Location", &device.location);

        use advmac::{MacAddr6, MacAddrFormat};

        if let Ok(mac) = MacAddr6::parse_str(&device.root.device.mac_address) {
            let str = mac.format_string(MacAddrFormat::ColonNotation);
            build_field!(pack, "          MAC Address", &str);
        }

        // TAB 2
        let grp2 = group::Group::new(PADDING, PADDING + 25, TAB_WIDTH, GROUP_HEIGHT, "Homepage\t");

        let mut scroll = group::Scroll::new(20, 50, grp2.w() - 20, grp2.h() - 40, "");

        scroll.set_frame(enums::FrameType::BorderBox);
        scroll.set_type(group::ScrollType::BothAlways);
        scroll.make_resizable(false);
        scroll.set_color(Color::from_hex(0x2e3436));
        scroll.set_scrollbar_size(SCROLL_WIDTH);

        let mut name = output::MultilineOutput::new(10, 35, grp2.w() + 100, grp2.h() * 5, "");
        name.set_value(&device.xml);
        name.set_color(Color::from_hex(0x2e3436));
        name.set_label_font(enums::Font::CourierBold);
        name.set_frame(enums::FrameType::FlatBox);
        name.set_text_size(14);

        scroll.end();
        scroll.scroll_to(0, 0);
        scroll.redraw();

        grp2.end();
        tab.end();

        window.end();

        window.show();
    }

    pub fn run(mut self) {
        let mut disc_mpsc: Option<std::sync::mpsc::Receiver<DeviceInfo>> = None;
        let mut timeout_handle: Option<app::TimeoutHandle> = None;
        let mut animation = AnimatedSvg::new(PROGRESS);
        while self.app.wait() {
            if let Some(msg) = self.receiver.recv() {
                match msg {
                    // Clear button pressed, forget all switches and clear the UI
                    Message::Clear => {
                        if !self.discovering {
                            info!("Message::Clear");
                            self.controller.clear(true);
                            self.buttons.clear();
                            self.pack.clear();
                            self.scroll.redraw();
                        }
                    }

                    // Reload button pressed, query WeeCtrl for devices in Cache and on LAN
                    Message::Reload => {
                        if !self.discovering {
                            info!("Message::Reload");
                            self.controller.clear(false);
                            self.buttons.clear();
                            self.pack.clear();
                            self.scroll.redraw();
                            self.sender.send(Message::StartDiscovery);
                        }
                    }

                    // WeeCtrl have found a switch, create a button for it
                    Message::AddButton(device) => {
                        info!(
                            "Message::AddButton {:?} {:?}",
                            device.unique_id, device.friendly_name
                        );
                        let _ignore = self.controller.subscribe(
                            &device.unique_id,
                            SUBSCRIPTION_DURATION,
                            SUBSCRIPTION_AUTO_RENEW,
                            CONN_TIMEOUT,
                        );

                        let mut but = button::Button::default()
                            .with_label(&format!("{}", device.friendly_name));

                        but.set_size(self.pack.w(), BUTTON_HEIGHT);
                        but.set_tooltip(SWITCH_TOOLTIP);

                        if device.state == State::On {
                            but.set_color(BUTTON_ON_COLOR);
                        } else {
                            but.set_color(BUTTON_OFF_COLOR);
                        }

                        but.handle(move |b, e| match e {
                            Event::Enter => {
                                b.set_frame(enums::FrameType::DiamondUpBox);
                                b.redraw();
                                true
                            }
                            Event::Leave => {
                                b.set_frame(enums::FrameType::UpBox);
                                b.redraw();
                                true
                            }
                            _ => false,
                        });

                        self.buttons.insert(device.unique_id.clone(), but.clone());
                        but.emit(self.sender.clone(), Message::Clicked(device));

                        self.pack.add(&but);

                        self.scroll.scroll_to(0, 0);
                        self.app.redraw();
                    }

                    // A button was clicked, flip state of the switch and update the color
                    // of the button according to returned result if it worked.
                    Message::Clicked(device) => {
                        info!(
                            "Message::Clicked MB({:?}) {:?} {:?}",
                            app::event_mouse_button(),
                            device.unique_id,
                            device.friendly_name
                        );

                        if let Some(btn) = self.buttons.get_mut(&device.unique_id) {
                            if app::event_mouse_button() == MouseButton::Right {
                                let mut icons: Option<Vec<weectrl::Icon>> = None;
                                if let Ok(res) =
                                    self.controller.get_icons(&device.unique_id, CONN_TIMEOUT)
                                {
                                    if res.len() > 0 {
                                        icons = Some(res);
                                    }
                                }
                                Self::show_popup(&device, icons);
                            } else {
                                let state = if btn.color() == BUTTON_ON_COLOR {
                                    State::Off
                                } else {
                                    State::On
                                };

                                if let Ok(ret_state) = self.controller.set_binary_state(
                                    &device.unique_id,
                                    state,
                                    CONN_TIMEOUT,
                                ) {
                                    if ret_state == State::On {
                                        btn.set_color(BUTTON_ON_COLOR);
                                    } else {
                                        btn.set_color(BUTTON_OFF_COLOR);
                                    }
                                }
                            }
                        }
                    }

                    // Display "Searching..." message and setup a thread to receive
                    // switches found, the thread will forward them to the UI message loop
                    // as Message::AddButton(Device) and end after mx seconds when the channel is closed.
                    Message::StartDiscovery => {
                        info!("Message::StartDiscovery");
                        self.discovering = true;
                        self.reloading_frame.set_label("Searching");
                        self.reloading_frame.show();

                        disc_mpsc = Some(self.controller.discover(
                            DiscoveryMode::CacheAndBroadcast,
                            false,
                            DISCOVERY_MX,
                        ));

                        // Start polling mechanism and UI spinner
                        let s = self.sender.clone();
                        timeout_handle = Some(app::add_timeout3(0.05, move |_| {
                            s.send(Message::PollDiscovery)
                        }));
                    }

                    // Since we animate the spinner by rotating it at 30fps we might as well
                    // just poll the discovery future mpsc here.
                    // When the future returns None we can move to EndDiscovery.
                    Message::PollDiscovery => {
                        //info!("Message::PollDiscovery");
                        if let Some(d) = disc_mpsc.as_mut() {
                            // Animate spinner
                            animation.rotate(4);
                            let mut img = animation.to_svg_image();
                            img.scale(21, 21, true, true);
                            self.progress_frame.set_image(Some(img));
                            self.progress_frame.hide();
                            self.progress_frame.show();
                            self.progress_frame.redraw();

                            // poll again in 33 ms
                            if let Some(h) = timeout_handle {
                                app::repeat_timeout3(0.033, h);
                            }

                            // Check channel for news or disconnection(discovery finished)
                            match d.try_recv() {
                                Ok(d) => self.sender.send(Message::AddButton(d)),
                                Err(TryRecvError::Disconnected) => {
                                    info!("Ending discovery");
                                    if let Some(h) = timeout_handle {
                                        app::remove_timeout3(h);
                                    }
                                    self.sender.send(Message::EndDiscovery);
                                }
                                Err(TryRecvError::Empty) => (),
                            };
                        }
                    }

                    // Discovery phase ended, update UI accordingly.
                    Message::EndDiscovery => {
                        info!("Message::EndDiscovery");
                        self.discovering = false;
                        self.progress_frame.hide();
                        self.reloading_frame.hide();
                    }

                    // A switch have changed state, update the UI accordingly.
                    Message::Notification(n) => {
                        info!("Message::Notification: {:?} {:?}", n.unique_id, n.state);

                        if let Some(btn) = self.buttons.get_mut(&n.unique_id) {
                            if n.state == State::On {
                                btn.set_color(BUTTON_ON_COLOR);
                            } else {
                                btn.set_color(BUTTON_OFF_COLOR);
                            }

                            self.app.redraw();
                        }
                    }
                }
            }
        }
    }
}

struct AnimatedSvg {
    // Svg file with a single rotate(000,x,y) instruction
    svg: String,
    // Current degrees of rotation
    position: u32,
    // Location of the hardcoded "000" in "rotate(000" to modify
    range: core::ops::Range<usize>,
}

impl AnimatedSvg {
    pub fn new(data: &str) -> Self {
        let location = data.find("rotate(000").unwrap() + 7;
        AnimatedSvg {
            svg: data.to_string(),
            position: 0,
            range: location..location + 3,
        }
    }

    // Rotate by current position + degrees delta.
    // Rotates the svg by modifying the svg data in place
    // much faster than string replace but unsafe
    // the constructor would have panicked at find(..).unwrap() if it wasn't possible
    pub fn rotate(&mut self, degrees: u32) {
        self.position += degrees;
        if self.position > 360 {
            self.position = 0
        }
        let do_rotate = format!("{:03}", self.position);
        let range = self.range.clone();
        unsafe {
            core::ptr::copy_nonoverlapping(do_rotate.as_ptr(), self.svg[range].as_mut_ptr(), 3);
        }
    }

    fn to_svg_image(&self) -> SvgImage {
        SvgImage::from_data(&self.svg).unwrap()
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Settings {
    x: i32,
    y: i32,
    w: i32,
    h: i32,
}

pub struct Storage {
    cache_file: Option<std::path::PathBuf>,
}

impl Default for Storage {
    fn default() -> Self {
        Self::new()
    }
}

impl Storage {
    #[must_use]
    pub fn new() -> Self {
        let mut file_path: Option<std::path::PathBuf> = None;

        if let Some(proj_dirs) = ProjectDirs::from("", "", "WeeApp") {
            let mut path = proj_dirs.config_dir().to_path_buf();
            path.push(SETTINGS_FILE);
            file_path = Some(path);
            info!("Settings file: {:#?}", file_path);
        }
        Self {
            cache_file: file_path,
        }
    }

    /// Write data to cache, errors ignored
    pub fn write(&self, settings: Settings) {
        info!("Saving settings: {:#?}", settings);

        if let Some(ref fpath) = self.cache_file {
            let data = settings;
            if let Some(prefix) = fpath.parent() {
                let _ignore = std::fs::create_dir_all(prefix);

                if let Ok(serialized) = serde_json::to_string(&data) {
                    if let Ok(mut buffer) = File::create(fpath) {
                        let _ignore = buffer.write_all(&serialized.into_bytes());
                    }
                }
            }
        }
    }

    pub fn read(&self) -> Option<Settings> {
        if let Some(ref fpath) = self.cache_file {
            if let Ok(mut file) = File::open(fpath) {
                let mut s = String::new();
                let _ignore = file.read_to_string(&mut s);
                let data: Option<Settings> = serde_json::from_str(&s).ok();
                info!("read settings: {:#?}", data);
                return data;
            }
        }
        None
    }

    pub fn clear(&self) {
        if let Some(ref fpath) = self.cache_file {
            let _ignore = std::fs::remove_file(fpath);
        }
    }
}

fn main() {
    use tracing_subscriber::fmt::time;

    tracing_subscriber::fmt()
        .with_timer(time::LocalTime::rfc_3339())
        .init();

    WeeApp::new().run();
}
