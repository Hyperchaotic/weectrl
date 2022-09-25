// On Windows don't create terminal window when opening app in release mode GUI
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

extern crate weectrl;

use tracing::info;

use weectrl::{DeviceInfo, DiscoveryMode, State, StateNotification, WeeController};

use fltk::{enums::Color, enums::Event, image::PngImage, prelude::*, *};
use fltk_theme::widget_schemes::fluent::colors::*;
use fltk_theme::{SchemeType, WidgetScheme};
use std::collections::HashMap;

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
    sender: app::Sender<Message>,
    receiver: app::Receiver<Message>,
    controller: WeeController,
    discovering: bool,                        // Is discover currently in progress?
    buttons: HashMap<String, button::Button>, // List of deviceID's and indexes of the associated buttons
}

const SETTINGS_FILE: &str = "Settings.json";

const SUBSCRIPTION_DURATION: u32 = 180;
const SUBSCRIPTION_AUTO_RENEW: bool = true;

const UNIT_SPACING: i32 = 40;
const BUTTON_HEIGHT: i32 = UNIT_SPACING;
const WINDOW_WIDTH: i32 = 350;
const TOP_BAR_HEIGHT: i32 = UNIT_SPACING + 10;
const LIST_HEIGHT: i32 = 170;
const WINDOW_HEIGHT: i32 = LIST_HEIGHT + TOP_BAR_HEIGHT + UNIT_SPACING;
const SCROLL_WIDTH: i32 = 15;

const BUTTON_ON_COLOR: Color = Color::from_rgb(114, 159, 207);
const BUTTON_OFF_COLOR: Color = Color::from_rgb(13, 25, 38);

const WINDOW_ICON: &[u8] = include_bytes!("images/icon.png");

const RL_BTN1: &[u8] = include_bytes!("images/refresh_b.png");
const RL_BTN2: &[u8] = include_bytes!("images/refresh_press_b.png");
const RL_BTN3: &[u8] = include_bytes!("images/refresh_hover_b.png");

const CL_BTN1: &[u8] = include_bytes!("images/clear.png");
const CL_BTN2: &[u8] = include_bytes!("images/clear_press.png");
const CL_BTN3: &[u8] = include_bytes!("images/clear_hover.png");

const CLEAR_TOOLTIP: &str = "Forget all devices and clear the on-disk list of known devices.";
const RELOAD_TOOLTIP: &str =
    "Reload list of devices from on-disk list (if any) and then by network query.";

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
        if let Some(settings) = storage.read() {
            main_win.set_size(settings.w, settings.h);
            main_win.set_pos(settings.x, settings.y);
        } else {
            main_win.set_size(WINDOW_WIDTH, WINDOW_HEIGHT);
        }

        main_win.set_color(Color::Gray0);

        let window_icon = PngImage::from_data(WINDOW_ICON).unwrap();
        main_win.set_icon(Some(window_icon));

        // Load all the images for clear/reload buttons
        let mut image_clear = PngImage::from_data(CL_BTN1).unwrap();
        image_clear.scale(50, 50, true, true);

        let mut image_clear_click = PngImage::from_data(CL_BTN2).unwrap();
        image_clear_click.scale(50, 50, true, true);

        let mut image_clear_hover = PngImage::from_data(CL_BTN3).unwrap();
        image_clear_hover.scale(50, 50, true, true);

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

        let mut image_reload = PngImage::from_data(RL_BTN1).unwrap();
        image_reload.scale(50, 50, true, true);

        let mut image_reload_click = PngImage::from_data(RL_BTN2).unwrap();
        image_reload_click.scale(50, 50, true, true);

        let mut image_reload_hover = PngImage::from_data(RL_BTN3).unwrap();
        image_reload_hover.scale(50, 50, true, true);

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
        scroll.set_color(Color::BackGround | Color::from_hex(0x2e3436));
        scroll.set_scrollbar_size(SCROLL_WIDTH);

        let mut pack = group::Pack::default()
            .with_size(scroll.w() - SCROLL_WIDTH, scroll.h())
            .center_of(&scroll);

        pack.set_type(group::PackType::Vertical);
        pack.set_spacing(2);
        pack.set_color(Color::BackGround | Color::Red);

        pack.end();

        main_win.end();

        // The part that says "Searching..." when looking for new switches on the LAN
        let mut reloading_frame = frame::Frame::default().with_size(100, UNIT_SPACING);
        reloading_frame.set_pos(10, WINDOW_HEIGHT - UNIT_SPACING);
        reloading_frame.set_color(Color::Black);
        reloading_frame.set_label_color(Color::Yellow);

        main_win.add(&reloading_frame);

        let mut sc = scroll.clone();
        let mut pa = pack.clone();
        let mut reload = btn_reload.clone();
        let mut clear = btn_clear.clone();
        let mut rlfr = reloading_frame.clone();

        let mut controller = WeeController::new();

        main_win.handle(move |w, ev| match ev {
            // When quitting the App save Windows size/position
            Event::Hide => {
                let settings = Settings {
                    x: w.x(),
                    y: w.y(),
                    w: w.w(),
                    h: w.h(),
                };

                info!("Quitting. Saving Window position/size: {:#?}", settings);

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
                false
            }

            // When resizing the App window, reposition internal elements accordingly
            Event::Resize => {
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

                rlfr.set_size(100, UNIT_SPACING);
                rlfr.set_pos(10, w.h() - UNIT_SPACING);

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
        let rx = controller.start_subscription_service().unwrap();
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
            sender,
            receiver,
            controller,
            discovering: true,
            buttons: HashMap::new(),
        }
    }

    pub fn run(mut self) {
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
                        );

                        let mut but = button::Button::default()
                            .with_label(&format!("{:?}", device.friendly_name));

                        but.set_size(self.pack.w(), BUTTON_HEIGHT);

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
                            "Message::Clicked {:?} {:?}",
                            device.unique_id, device.friendly_name
                        );
                        if let Some(btn) = self.buttons.get_mut(&device.unique_id) {
                            let state = if btn.color() == BUTTON_ON_COLOR {
                                State::Off
                            } else {
                                State::On
                            };

                            if let Ok(ret_state) =
                                self.controller.set_binary_state(&device.unique_id, state)
                            {
                                if ret_state == State::On {
                                    btn.set_color(BUTTON_ON_COLOR);
                                } else {
                                    btn.set_color(BUTTON_OFF_COLOR);
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
                        self.reloading_frame.set_label("Searching...");
                        let s = self.sender.clone();

                        let rx = self.controller.discover_async(
                            DiscoveryMode::CacheAndBroadcast,
                            false,
                            5,
                        );
                        let _ignore = std::thread::Builder::new()
                            .name("APP_discovery".to_string())
                            .spawn(move || {
                                loop {
                                    let msg = rx.recv();
                                    info!("Discover thread forwarding");

                                    if let Ok(dev) = msg {
                                        s.send(Message::AddButton(dev));
                                    } else {
                                        s.send(Message::EndDiscovery);
                                        break; // End thread
                                    }
                                }
                            })
                            .unwrap();
                    }

                    // Discovery phase ended, update UI accordingly.
                    Message::EndDiscovery => {
                        info!("Message::EndDiscovery");
                        self.discovering = false;
                        self.reloading_frame.set_label("");
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

fn main() {
    use tracing_subscriber::fmt::time;

    tracing_subscriber::fmt()
        .with_timer(time::LocalTime::rfc_3339())
        .init();

    let a = WeeApp::new();
    a.run();
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
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
