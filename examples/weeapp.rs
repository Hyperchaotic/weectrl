// On Windows don't create terminal window when opening app in GUI

#![windows_subsystem = "windows"]

extern crate weectrl;

use tracing::info;
use tracing_subscriber;

use weectrl::*;

use fltk::{enums::Event, image::PngImage, prelude::*, *};
use fltk_theme::widget_schemes::fluent::colors::*;
use fltk_theme::{SchemeType, WidgetScheme};
use std::collections::HashMap;

extern crate directories;
use directories::ProjectDirs;

use serde::{Deserialize, Serialize};
use serde_json;

use std;

use std::fs::File;
use std::io::prelude::*;

#[derive(Debug, Clone)]
enum Message {
    Resize(Settings),
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
    main_win: window::Window,
    scroll: group::Scroll,
    pack: group::Pack,
    reloading_frame: frame::Frame,
    sender: app::Sender<Message>,
    receiver: app::Receiver<Message>,
    controller: WeeController,
    discovering: bool,                        // Is dicover currently in progress?
    buttons: HashMap<String, button::Button>, // List of deviceID's and indexes of the associated buttons
}

const SETTINGS_FILE: &'static str = "Settings.dat";

const SUBSCRIPTION_DURATION: u32 = 180;
const SUBSCRIPTION_AUTO_RENEW: bool = true;

const UNIT_SPACING: i32 = 40;
const WINDOW_WIDTH: i32 = 400;
const TOP_BAR_HEIGHT: i32 = UNIT_SPACING + 10;
const LIST_WIDTH: i32 = WINDOW_WIDTH - 80;
const LIST_HEIGHT: i32 = 220;
const WINDOW_HEIGHT: i32 = LIST_HEIGHT + TOP_BAR_HEIGHT + UNIT_SPACING;
const SCROLL_WIDTH: i32 = 15;

const BUTTON_ON_COLOR: enums::Color = enums::Color::from_rgb(114, 159, 207);
const BUTTON_OFF_COLOR: enums::Color = enums::Color::from_rgb(13, 25, 38);

const WINDOW_ICON: &[u8] = include_bytes!("images/icon.png");

const RL_BTN1: &[u8] = include_bytes!("images/refresh_b.png");
const RL_BTN2: &[u8] = include_bytes!("images/refresh_press_b.png");
const RL_BTN3: &[u8] = include_bytes!("images/refresh_hover_b.png");

const CL_BTN1: &[u8] = include_bytes!("images/clear.png");
const CL_BTN2: &[u8] = include_bytes!("images/clear_press.png");
const CL_BTN3: &[u8] = include_bytes!("images/clear_hover.png");

const WEEAPP_TITLE: &str = "WeeApp 0.9.2 (beta)";
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
            enums::Color::Selection,
            SELECTION_COLOR.0,
            SELECTION_COLOR.1,
            SELECTION_COLOR.2,
        );

        let theme = WidgetScheme::new(SchemeType::Fluent);
        theme.apply();

        app::set_font_size(18);

        let (s, receiver) = app::channel();

        let mut main_win = window::Window::default()
            .with_size(WINDOW_WIDTH, WINDOW_HEIGHT)
            .with_label(WEEAPP_TITLE);
        main_win.set_color(enums::Color::Gray0);

        let window_icon = PngImage::from_data(WINDOW_ICON).unwrap();
        main_win.set_icon(Some(window_icon));

        let mut image_clear = PngImage::from_data(CL_BTN1).unwrap();
        image_clear.scale(50, 50, true, true);

        let mut image_clear_click = PngImage::from_data(CL_BTN2).unwrap();
        image_clear_click.scale(50, 50, true, true);

        let mut image_clear_hover = PngImage::from_data(CL_BTN3).unwrap();
        image_clear_hover.scale(50, 50, true, true);

        let mut btn_clear = button::Button::default().with_size(50, 50);
        btn_clear.set_frame(enums::FrameType::FlatBox);
        btn_clear.set_pos(WINDOW_WIDTH - UNIT_SPACING - 10 - UNIT_SPACING - 10, 0);
        btn_clear.set_image(Some(image_clear.clone()));

        let ic = image_clear.clone();
        let icc = image_clear_click.clone();
        let ich = image_clear_hover.clone();
        btn_clear.emit(s.clone(), Message::Clear);
        btn_clear.set_tooltip(CLEAR_TOOLTIP);

        btn_clear.handle(move |b, e| match e {
            Event::Enter => {
                b.set_image(Some(ich.clone()));
                b.redraw();
                true
            }
            Event::Released => {
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
        btn_reload.set_pos(WINDOW_WIDTH - UNIT_SPACING - 10, 0);
        btn_reload.set_image(Some(image_reload.clone()));

        let ir = image_reload.clone();
        let irc = image_reload_click.clone();
        let irh = image_reload_hover.clone();
        btn_reload.set_tooltip(RELOAD_TOOLTIP);
        btn_reload.emit(s.clone(), Message::Reload);

        btn_reload.handle(move |b, e| match e {
            Event::Enter => {
                b.set_image(Some(irh.clone()));
                b.redraw();
                true
            }
            Event::Released => {
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

        let mut inner_win = window::Window::default()
            .with_label("TEST RESIZE")
            .with_size(
                main_win.w() - 2 * UNIT_SPACING,
                main_win.h() - 2 * UNIT_SPACING,
            );

        inner_win.set_pos(UNIT_SPACING, TOP_BAR_HEIGHT);
        inner_win.set_frame(enums::FrameType::BorderBox);
        inner_win.set_color(enums::Color::BackGround | enums::Color::from_hex(0x2e3436));

        let mut scroll = group::Scroll::default().size_of_parent();
        scroll.set_frame(enums::FrameType::BorderBox);
        scroll.set_type(group::ScrollType::Vertical);
        scroll.make_resizable(false);
        scroll.set_color(enums::Color::BackGround | enums::Color::from_hex(0x2e3436));
        scroll.set_scrollbar_size(SCROLL_WIDTH);

        let mut pack = group::Pack::new(0, 0, LIST_WIDTH - 15, LIST_HEIGHT, "");
        pack.set_type(group::PackType::Vertical);
        pack.set_spacing(2);
        pack.set_color(enums::Color::BackGround | enums::Color::Red);

        pack.end();

        inner_win.end();
        main_win.end();

        let mut reloading_frame = frame::Frame::default().with_size(100, UNIT_SPACING);
        reloading_frame.set_pos(10, WINDOW_HEIGHT - UNIT_SPACING);
        reloading_frame.set_color(enums::Color::Black);
        reloading_frame.set_label_color(enums::Color::Yellow);

        main_win.add(&reloading_frame);

        let mut sc = scroll.clone();
        let mut inner_win_c = inner_win.clone();
        let mut pa = pack.clone();
        let mut reload = btn_reload.clone();
        let mut clear = btn_clear.clone();
        let mut rlfr = reloading_frame.clone();

        let mut controller = WeeController::new();

        main_win.handle(move |w, ev| match ev {
            Event::Hide => {
                //controller.unsubscribe_all();
                let settings = Settings {
                    x: w.x(),
                    y: w.y(),
                    w: w.w(),
                    h: w.h(),
                };

                info!("Quitting. Saving Window: {:#?}", settings);

                let storage = Storage::new();
                storage.write(settings);
                true
            }

            Event::Resize => {
                inner_win_c.resize(
                    UNIT_SPACING,
                    TOP_BAR_HEIGHT,
                    w.w() - 2 * UNIT_SPACING,
                    w.h() - 2 * UNIT_SPACING - 5,
                );

                pa.resize(0, 0, inner_win_c.w() - SCROLL_WIDTH, inner_win_c.h());

                sc.resize(0, 0, inner_win_c.w(), inner_win_c.h());

                reload.set_size(50, 50);
                reload.set_pos(w.w() - UNIT_SPACING - 10, 0);

                clear.set_size(50, 50);
                clear.set_pos(w.w() - UNIT_SPACING - 10 - UNIT_SPACING - 10, 0);

                rlfr.set_size(100, UNIT_SPACING);
                rlfr.set_pos(10, w.h() - UNIT_SPACING);

                true
            }
            _ => false,
        });

        main_win.make_resizable(true);

        let rx = controller.start_subscription_service().unwrap();
        let sc = s.clone();

        let _ = std::thread::Builder::new()
            .name("APP_notifiy".to_string())
            .spawn(move || {
                while let Ok(n) = rx.recv() {
                    let _ = sc.send(Message::Notification(n.clone()));
                }
            });

        main_win.show();

        s.send(Message::StartDiscovery);

        let storage = Storage::new();
        if let Some(settings) = storage.read() {
            s.send(Message::Resize(settings));
        }

        Self {
            app: app,
            main_win: main_win,
            pack: pack,
            scroll: scroll,
            reloading_frame: reloading_frame,
            sender: s,
            receiver: receiver,
            controller: controller,
            discovering: true,
            buttons: HashMap::new(),
        }
    }

    // When adding buttons, they would not show, even with app.redraw().
    // Resizing window is the only thing that works.
    fn force_refresh(&mut self) {
        //Sometimes the buttons would mysteriously be double height
        let cnt = self.pack.children();
        for i in 0..cnt {
            let mut btn = self.pack.child(i).unwrap();
            btn.set_size(0, UNIT_SPACING);
        }

        self.pack.redraw();
        self.scroll.redraw();

        // It only shows the buttons if I do this, TODO figure out why...
        self.main_win
            .set_size(self.main_win.w(), self.main_win.h() + 5);
        self.main_win.redraw();
        self.main_win
            .set_size(self.main_win.w(), self.main_win.h() - 5);
        self.main_win.redraw();
    }

    pub fn run(mut self) {
        while self.app.wait() {
            if let Some(msg) = self.receiver.recv() {
                match msg {
                    Message::Resize(settings) => {
                        info!("Resizing application window: {:#?}", settings);
                        self.main_win
                            .resize(settings.x, settings.y, settings.w, settings.h);
                        self.app.redraw();
                    }
                    Message::Clear => {
                        if !self.discovering {
                            info!("Message::Clear");
                            self.controller.clear(true);
                            self.buttons.clear();

                            for _i in 0..self.pack.children() {
                                let wgt = self.pack.child(0).unwrap();
                                self.pack.remove_by_index(0);
                                fltk::app::delete_widget(wgt);
                            }
                            self.scroll.redraw();
                        }
                    }
                    Message::Reload => {
                        if !self.discovering {
                            info!("Message::Reload");
                            self.controller.clear(false);
                            self.buttons.clear();

                            for _i in 0..self.pack.children() {
                                let wgt = self.pack.child(0).unwrap();
                                self.pack.remove_by_index(0);
                                fltk::app::delete_widget(wgt);
                            }

                            self.sender.send(Message::StartDiscovery);
                        }
                    }

                    Message::AddButton(device) => {
                        info!(
                            "Message::AddButton {:?} {:?}",
                            device.unique_id, device.friendly_name
                        );
                        let _ = self.controller.subscribe(
                            &device.unique_id,
                            SUBSCRIPTION_DURATION,
                            SUBSCRIPTION_AUTO_RENEW,
                        );

                        let mut but = button::Button::default()
                            .with_size(0, UNIT_SPACING)
                            .with_label(&format!("{:?}", device.friendly_name));
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

                        but.emit(self.sender.clone(), Message::Clicked(device.clone()));

                        self.buttons.insert(device.unique_id, but.clone());
                        self.pack.add(&but);

                        self.force_refresh();
                    }

                    Message::Clicked(device) => {
                        info!(
                            "Message::Clicked {:?} {:?}",
                            device.unique_id, device.friendly_name
                        );
                        if let Some(btn) = self.buttons.get(&device.unique_id) {
                            let state = if btn.color() == BUTTON_ON_COLOR {
                                State::Off
                            } else {
                                State::On
                            };

                            if let Ok(ret_state) =
                                self.controller.set_binary_state(&device.unique_id, state)
                            {
                                if ret_state == State::On {
                                    btn.clone().set_color(BUTTON_ON_COLOR);
                                } else {
                                    btn.clone().set_color(BUTTON_OFF_COLOR);
                                }
                            }
                        }
                    }

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
                        let _ = std::thread::Builder::new()
                            .name("APP_discovery".to_string())
                            .spawn(move || {
                                loop {
                                    let msg = rx.recv();
                                    info!("Discover thread forwarding");
                                    match msg {
                                        Ok(dev) => s.send(Message::AddButton(dev)),
                                        Err(_) => {
                                            s.send(Message::EndDiscovery);
                                            break; // End thread
                                        }
                                    }
                                }
                            });
                    }

                    Message::EndDiscovery => {
                        info!("Message::EndDiscovery");
                        self.discovering = false;
                        self.reloading_frame.set_label("");
                    }
                    Message::Notification(n) => {
                        info!("Message::Notification: {:?} {:?}", n.unique_id, n.state);
                        if let Some(btn) = self.buttons.get(&n.unique_id) {
                            if n.state == State::On {
                                btn.clone().set_color(BUTTON_ON_COLOR);
                            } else {
                                btn.clone().set_color(BUTTON_OFF_COLOR);
                            }

                            self.main_win.redraw();
                            self.app.redraw()
                        }
                    }
                }
            }
        }
        self.app.redraw()
    }
}

fn main() {
    // install global collector configured based on RUST_LOG env var.
    tracing_subscriber::fmt::init();

    let a = WeeApp::new();
    a.run();
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Settings {
    x: i32,
    y: i32,
    w: i32,
    h: i32,
}

pub struct Storage {
    cache_file: Option<std::path::PathBuf>,
}

impl Storage {
    pub fn new() -> Storage {
        let mut file_path: Option<std::path::PathBuf> = None;

        if let Some(proj_dirs) = ProjectDirs::from("", "", "WeeApp") {
            let mut path = proj_dirs.config_dir().to_path_buf();
            path.push(SETTINGS_FILE);
            file_path = Some(path);
            info!("Settings file: {:#?}", file_path);
        }
        Storage {
            cache_file: file_path,
        }
    }

    /// Write data to cache, errors ignored
    pub fn write(&self, settings: Settings) {
        if let Some(ref fpath) = self.cache_file {
            let data = settings;
            if let Some(prefix) = fpath.parent() {
                let _ = std::fs::create_dir_all(prefix);

                if let Ok(serialized) = serde_json::to_string(&data) {
                    if let Ok(mut buffer) = File::create(fpath) {
                        let _ = buffer.write_all(&serialized.into_bytes());
                    }
                }
            }
        }
    }

    pub fn read(&self) -> Option<Settings> {
        if let Some(ref fpath) = self.cache_file {
            if let Ok(mut file) = File::open(fpath) {
                let mut s = String::new();
                let _ = file.read_to_string(&mut s);
                let data: Option<Settings> = serde_json::from_str(&s).ok();
                return data;
            }
        }
        None
    }

    pub fn clear(&self) {
        if let Some(ref fpath) = self.cache_file {
            let _ = std::fs::remove_file(fpath);
        }
    }
}
