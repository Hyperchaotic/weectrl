// On Windows don't create terminal window when opening app in GUI
#![feature(windows_subsystem)]
#![windows_subsystem = "windows"]

extern crate weectrl;

extern crate chrono;
extern crate app_dirs;

#[macro_use(slog_info, slog_log, o)]
extern crate slog;

#[macro_use]
extern crate slog_scope;
extern crate slog_stream;

use slog::DrainExt;
use app_dirs::*;

use std::io;
use std::fs::OpenOptions;

use weectrl::weectrl::*;
use weectrl::device::State;

#[macro_use]
extern crate conrod;
extern crate image;

use conrod::backend::glium::glium;
use conrod::backend::glium::glium::{DisplayBuild, Surface};

use conrod::{Borderable, color};

use std::sync::{Arc, Mutex, Condvar};
use std::time::Duration;
use std::thread;
use std::sync::mpsc;

widget_ids! {
    struct Ids { canvas, top_bar, list_canvas, list,
        button_clear, button_refresh, refresh_image, clear_image, wait_label,
    notify_canvas, notify_button, notify_label, scanning_label }
}

const SUBSCRIPTION_DURATION: u32 = 120;
const WINDOW_WIDTH: u32 = 400;
const TOP_BAR_HEIGHT: u32 = 40;
const LIST_WIDTH: u32 = WINDOW_WIDTH - 80;
const LIST_HEIGHT: u32 = 220;
const WINDOW_HEIGHT: u32 = LIST_HEIGHT + TOP_BAR_HEIGHT + 40;

#[derive(Debug, Clone, Copy, PartialEq)]
enum AppState {
    Initializing,
    StartDiscovery,
    Discovering,
    Ready,
    NoDevices,
}

struct NotificationDialog {
    pub is_open: bool,
    pub message: String,
}

const LOG_FILE: &'static str = "WeeController.log";

const APP_INFO: AppInfo = AppInfo {
    name: "WeeApp",
    author: "hyperchaotic",
};

fn start_discovery_async(event_loop: Arc<EventLoop>,
                         ctrl: Arc<Mutex<WeeController>>)
                         -> mpsc::Receiver<DeviceInfo> {

    let (discovery, discoveries) = mpsc::sync_channel(1);
    thread::spawn(move || {
        info!("Discovery Thread: Start discovery");
        let rx: mpsc::Receiver<DeviceInfo> =
            ctrl.lock().unwrap().discover_async(DiscoveryMode::CacheAndBroadcast, true, 3);
        loop {
            if let Some(d) = rx.recv().ok() {
                info!(" >>>>>>>>>>>>>> Got device {:?}", d.unique_id);
                let _ = discovery.send(d);
                event_loop.needs_update();
                info!(" <<<<<<<<<<<<<<< ");
            } else {
                break;
            }
        }
        event_loop.needs_update();
        info!("Discovery Thread: Ended.");
    });

    discoveries
}

fn start_notification_listener(event_loop: Arc<EventLoop>,
                               ctrl: Arc<Mutex<WeeController>>)
                               -> mpsc::Receiver<StateNotification> {

    let (notify, notifications) = mpsc::sync_channel(1);
    thread::spawn(move || {
        let rx: Option<mpsc::Receiver<StateNotification>>;
        {
            let mut controller = ctrl.lock().unwrap();
            rx = controller.start_subscription_service().ok();
        }
        if let Some(rx) = rx {
            loop {
                match rx.recv() {
                    Ok(o) => {
                        let _ = notify.send(o);
                        event_loop.needs_update();
                    }
                    Err(_) => (),
                };
            }
        }
    });
    notifications
}

fn main() {
    // get path for logfile: %USERPROFILE%\AppData\Local\hyperchaotic\WeeApp\WeeController.log
    //                   or: $HOME/.cache/WeeApp/WeeController.log
    let mut log_path = String::from(LOG_FILE);
    if let Some(mut path) = app_root(AppDataType::UserCache, &APP_INFO).ok() {
        path.push(LOG_FILE);
        if let Some(st) = path.to_str() {
            log_path = st.to_owned();
        }
    }
    // Create logger
    let file = OpenOptions::new().create(true).write(true).truncate(true).open(log_path).unwrap();
    let drain = slog_stream::stream(file, MyFormat).fuse();
    let logger = slog::Logger::root(drain, o!());
    slog_scope::set_global_logger(logger.clone());

    info!("Starting up");

    // Include resources in binary to avoid problems with missing files
    let font_bytes = include_bytes!("../assets/fonts/NotoSans/NotoSans-Regular.ttf");
    let refresh_image_bytes = include_bytes!("../assets/images/refresh.png");
    let clear_image_bytes = include_bytes!("../assets/images/clear.png");

    let mut notification_diag = NotificationDialog {
        is_open: false,
        message: String::new(),
    };
    // Initial state
    let mut app_state = AppState::Initializing;

    // Build the window.
    let display = glium::glutin::WindowBuilder::new()
        .with_vsync()
        .with_dimensions(WINDOW_WIDTH as u32, WINDOW_HEIGHT as u32)
        .with_title("Wee Controller")
        .build_glium()
        .unwrap();

    // Construct our `Ui`.
    let mut ui = conrod::UiBuilder::new([WINDOW_WIDTH as f64, WINDOW_HEIGHT as f64]).build();
    let mut renderer = conrod::backend::glium::Renderer::new(&display).unwrap();

    // Unique identifier for each widget.
    let ids = Ids::new(ui.widget_id_generator());

    use conrod::text::FontCollection;
    let font = FontCollection::from_bytes(&font_bytes[0..font_bytes.len()]).into_font().unwrap();
    ui.fonts.insert(font);

    let mut image_map = conrod::image::Map::new();
    let refresh_image = image_map.insert(load_image(refresh_image_bytes, &display));
    let clear_image = image_map.insert(load_image(clear_image_bytes, &display));

    let mut list: Vec<(DeviceInfo, bool)> = Vec::new();
    let mut weecontrol = Arc::new(Mutex::new(WeeController::new(Some(logger))));

    // Poll events from the window.
    let event_loop = Arc::new(EventLoop::new());

    // Notification listener, runs perpetually.
    let notification = start_notification_listener(event_loop.clone(), weecontrol.clone());
    let mut discovery: Option<mpsc::Receiver<DeviceInfo>> = None;

    event_loop.needs_update();
    'main: loop {

        if let Some(notification) = notification.try_recv().ok() {
            update_list(&mut list, &notification);
            event_loop.needs_update();
        }

        if app_state == AppState::StartDiscovery {
            list.clear();
            app_state = AppState::Discovering;
            discovery = Some(start_discovery_async(event_loop.clone(), weecontrol.clone()));
        }

        if app_state == AppState::Discovering {
            if let Some(ref rx) = discovery {
                match rx.try_recv() {
                    Ok(dev) => {
                        info!("UI Got device {:?}", dev.unique_id);
                        let dev_state: bool = if dev.state == State::On { true } else { false };
                        {
                            let mut controller = weecontrol.lock().unwrap();
                            let _ =
                                controller.subscribe(&dev.unique_id, SUBSCRIPTION_DURATION, true);
                        }
                        list.push((dev, dev_state));
                    }
                    Err(mpsc::TryRecvError::Empty) => (),
                    Err(_) => {
                        if list.is_empty() {
                            app_state = AppState::NoDevices;
                        } else {
                            app_state = AppState::Ready;
                        }
                    }
                };
            } else {
                if list.is_empty() {
                    app_state = AppState::NoDevices;
                } else {
                    app_state = AppState::Ready;
                }
            }
            event_loop.needs_update();
        }

        set_ui(ui.set_widgets(),
               &mut list,
               &ids,
               &mut app_state,
               &mut weecontrol,
               &mut notification_diag,
               refresh_image,
               clear_image);

        // Render the `Ui` and then display it on the screen.
        if let Some(primitives) = ui.draw_if_changed() {
            renderer.fill(&display, primitives, &image_map);
            let mut target = display.draw();
            target.clear_color(0.0, 0.0, 0.0, 1.0);
            renderer.draw(&display, &mut target, &image_map).unwrap();
            target.finish().unwrap();
        }


        for event in event_loop.next(&display) {
            // Use the `winit` backend feature to convert the winit event to a conrod one.
            if let Some(event) = conrod::backend::winit::convert(event.clone(), &display) {
                ui.handle_event(event);
                event_loop.needs_update();
            }
            match event {
                // Break from the loop upon `Escape`.
                glium::glutin::Event::KeyboardInput(_, _,
                    Some(glium::glutin::VirtualKeyCode::Escape)) |
                glium::glutin::Event::Closed =>
                    break 'main,
                _ => {}
            }
        }
    }

    let mut controller = weecontrol.lock().unwrap();
    controller.unsubscribe_all();
}

// Declare the `WidgetId`s and instantiate the widgets.
fn set_ui(ref mut ui: conrod::UiCell,
          list: &mut Vec<(DeviceInfo, bool)>,
          ids: &Ids,
          app_state: &mut AppState,
          weecontrol: &mut Arc<Mutex<WeeController>>,
          notification_diag: &mut NotificationDialog,
          refresh_image: conrod::image::Id,
          clear_image: conrod::image::Id) {

    use conrod::{widget, Colorable, Labelable, Positionable, Sizeable, Widget};

    widget::Canvas::new().color(conrod::color::BLACK).set(ids.canvas, ui);

    widget::Canvas::new()
        .color(conrod::color::BLACK)
        .top_left_with_margins_on(ids.canvas, 0.0, 00.0)
        .border(0.0)
        .w_h(WINDOW_WIDTH as f64, TOP_BAR_HEIGHT as f64)
        .set(ids.top_bar, ui);

    const ITEM_HEIGHT: conrod::Scalar = 40.0;

    widget::Canvas::new()
        .color(conrod::color::DARK_CHARCOAL)
        .w_h(LIST_WIDTH as f64, LIST_HEIGHT as f64)
        .top_left_with_margins_on(ids.canvas, (TOP_BAR_HEIGHT) as f64, 40.0)
        .set(ids.list_canvas, ui);

    if widget::Button::image(refresh_image)
        .w_h(40.0, 40.0)
        .top_right_with_margins_on(ids.top_bar, 0.0, 0.0)
        .color(color::TRANSPARENT)
        .set(ids.button_refresh, ui)
        .was_clicked() {
        if !notification_diag.is_open && *app_state != AppState::Discovering {
            info!("WeeApp REFRESH", );
            *app_state = AppState::StartDiscovery;
        }
    }

    if widget::Button::image(clear_image)
        .w_h(40.0, 40.0)
        .top_right_with_margins_on(ids.top_bar, 0.0, 40.0)
        .color(color::TRANSPARENT)
        .set(ids.button_clear, ui)
        .was_clicked() {
        if !notification_diag.is_open && *app_state != AppState::Discovering {
            *app_state = AppState::NoDevices;
            info!("WeeApp CLEAR", );
            {
                let mut controller = weecontrol.lock().unwrap();
                controller.clear(true);
                list.clear();
            }
        }
    }

    if *app_state==AppState::Ready || (*app_state==AppState::Discovering && !list.is_empty()) {
        let (mut items, scrollbar) = widget::List::new(list.len(), ITEM_HEIGHT)
            .scrollbar_on_top()
            .scrollbar_color(conrod::color::BLUE)
            .scrollbar_width(15.0)
            .middle_of(ids.list_canvas)
            .wh_of(ids.list_canvas)
            .set(ids.list, ui);

        while let Some(item) = items.next(ui) {
            let i = item.i;
            let label = format!("{}", list[i].0.friendly_name);
            let label_color = conrod::color::WHITE;
            let color = conrod::color::LIGHT_BLUE;
            let toggle = widget::Toggle::new(list[i].1)
                .label(&label)
                .label_color(label_color)
                .color(color);
            for v in item.set(toggle, ui) {
                if !notification_diag.is_open {
                    if list[i].1 != v {
                        let mut controller = weecontrol.lock().unwrap();
                        let new_state = if v { State::On } else { State::Off };

                        info!("list[{}]={:?}. v={:?}", i, list[i].1, v);

                        let res = controller.set_binary_state(&list[i].0.unique_id, new_state);
                        match res {
                            Ok(_) => list[i].1 = v,
                            Err(weectrl::error::Error::DeviceError) => {
                                notification_diag.message = String::from("ERROR\nDevice \
                                                                          returned 'Error'.");
                                notification_diag.is_open = true;
                            }
                            Err(e) => {
                                notification_diag.message = format!("ERROR\n{:?}.", e);
                                notification_diag.is_open = true;
                            }
                        }
                        info!("WeeApp TOGGLE {:?} {:?}", i, v);
                    }
                }
            }
        }
        if let Some(s) = scrollbar {
            s.set(ui)
        }

        if notification_diag.is_open {
            widget::Canvas::new()
                .color(conrod::color::BLUE)
                .middle_of(ids.list_canvas)
                .w_h(200.0, 160.0)
                .set(ids.notify_canvas, ui);

            widget::Text::new(&notification_diag.message)
                .color(color::YELLOW)
                .w_h(180.0, 100.0)
                .top_left_with_margins_on(ids.notify_canvas, 10.0, 10.0)
                .align_text_middle()
                .set(ids.notify_label, ui);

            if widget::Button::new()
                .color(conrod::color::PURPLE)
                .mid_top_with_margin_on(ids.notify_canvas, 100.0)
                .label("Dismiss")
                .w_h(80.0, 40.0)
                .set(ids.notify_button, ui)
                .was_clicked() {
                notification_diag.is_open = false;
            }
        }
        if *app_state==AppState::Discovering {
            widget::Text::new("Searching...")
                .color(color::YELLOW)
                .w_h(180.0, 20.0)
                .bottom_left_with_margins_on(ids.canvas, 15.0, 5.0)
                .align_text_middle()
                .set(ids.scanning_label, ui);
        }
    } else {
        match *app_state {
            AppState::Ready => (),
            AppState::StartDiscovery => {},
            AppState::Initializing => {
                widget::Text::new("Scanning for devices...")
                    .color(color::LIGHT_YELLOW)
                    .middle_of(ids.list_canvas)
                    .align_text_middle()
                    .set(ids.wait_label, ui);
                *app_state = AppState::StartDiscovery;
            }
            AppState::Discovering => {
                widget::Text::new("Scanning for devices...")
                    .color(color::LIGHT_YELLOW)
                    .middle_of(ids.list_canvas)
                    .align_text_middle()
                    .set(ids.wait_label, ui);
            }
            AppState::NoDevices => {
                widget::Text::new("No devices found.")
                    .color(color::LIGHT_YELLOW)
                    .middle_of(ids.list_canvas)
                    .align_text_middle()
                    .set(ids.wait_label, ui);
            }
        }
    }
}

fn update_list(list: &mut Vec<(DeviceInfo, bool)>, notification: &StateNotification) {
    for entry in list.iter_mut() {
        if *entry.0.unique_id == notification.unique_id {
            match notification.state {
                State::On => entry.1 = true,
                State::Off => entry.1 = false,
                State::Unknown => (),
            };
            break;
        }
    }
}

// Convert a PNG image to a texture
fn load_image(image_bytes: &[u8], display: &glium::Display) -> glium::texture::Texture2d {

    use image::ImageDecoder;
    let mut decoder = image::png::PNGDecoder::new(&image_bytes[0..image_bytes.len()]);
    let dimensions = decoder.dimensions().unwrap();
    let buf = decoder.read_image().unwrap();

    if let image::DecodingResult::U8(buf) = buf {
        let raw_image = glium::texture::RawImage2d::from_raw_rgba_reversed(buf, dimensions);
        let texture = glium::texture::Texture2d::new(display, raw_image).unwrap();
        return texture;
    }
    glium::texture::Texture2d::empty(display, 0, 0).unwrap()
}

struct EventLoop {
    pair: Arc<(Mutex<bool>, Condvar)>,
}

impl EventLoop {
    pub fn new() -> Self {
        EventLoop { pair: Arc::new((Mutex::new(false), Condvar::new())) }
    }

    /// Produce an iterator yielding all available events.
    pub fn next(&self, display: &glium::Display) -> Vec<glium::glutin::Event> {
        let mut events = Vec::new();

        loop {
            events.extend(display.poll_events());
            if !events.is_empty() {
                break;
            }

            let &(ref lock, ref cvar) = &*self.pair;
            let guard = lock.lock().unwrap();
            let mut needs_update = cvar.wait_timeout(guard, Duration::from_millis(16)).unwrap().0;

            if *needs_update {
                *needs_update = false;
                break;
            }
        }

        events
    }

    /// Notifies the event loop that the `Ui` requires another update whether or not there are any
    /// pending events.
    pub fn needs_update(&self) {
        let &(ref lock, ref cvar) = &*self.pair;
        let mut guard = lock.lock().unwrap();
        *guard = true;
        cvar.notify_one();
    }
}

struct MyFormat;

impl slog_stream::Format for MyFormat {
    fn format(&self,
              io: &mut io::Write,
              rinfo: &slog::Record,
              _logger_values: &slog::OwnedKeyValueList)
              -> io::Result<()> {
        let ts = chrono::Local::now();
        let msg = format!("{} {} {}: {}\n",
                          ts.format("%H:%M:%S%.3f"),
                          rinfo.level(),
                          rinfo.module(),
                          rinfo.msg());
        let _ = try!(io.write_all(msg.as_bytes()));
        Ok(())
    }
}
