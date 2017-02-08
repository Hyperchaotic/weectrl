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

extern crate futures;
extern crate tokio_core;

use futures::stream::Stream;
use tokio_core::reactor::Core;

use slog::DrainExt;
use app_dirs::*;

use std::io;
use std::fs::OpenOptions;

use weectrl::weectrl::*;

#[macro_use]
extern crate conrod;
extern crate image;

use conrod::backend::glium::glium;
use conrod::backend::glium::glium::{DisplayBuild, Surface};

use conrod::{Borderable, color};

use std::sync::{Arc, Mutex, mpsc};

use std::thread;

widget_ids! {
    struct Ids { canvas, top_bar, list_canvas, list,
        button_clear, button_refresh, refresh_image, clear_image, wait_label,
    notify_canvas, notify_button, notify_label, scanning_label }
}

const SUBSCRIPTION_DURATION: u32 = 180;
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

#[derive(Debug)]
enum Message {
    Event(conrod::event::Input),
    DiscoveryItem(DeviceInfo),
    DiscoveryEnded,
    NotificationsStarted,
    Notification(StateNotification),
}

struct ImageIds {
    refresh_normal: conrod::image::Id,
    refresh_press: conrod::image::Id,
    refresh_hover: conrod::image::Id,
    clear_normal: conrod::image::Id,
    clear_press: conrod::image::Id,
    clear_hover: conrod::image::Id,
}

fn start_discovery_async(tx: mpsc::Sender<Message>,
                         ctrl: Arc<Mutex<WeeController>>) {

    thread::spawn(move || {
        info!("Discovery Thread: Start discovery");

        let mut core = Core::new().unwrap();
        let discovery =
            ctrl.lock().unwrap().discover_future(DiscoveryMode::CacheAndBroadcast, true, 3);

        let processor = discovery.for_each(|o| {
            info!(" Got device {:?}", o.unique_id);
            let _ = tx.send(Message::DiscoveryItem(o));
            Ok(())
        });

        core.run(processor).unwrap();
        let _ = tx.send(Message::DiscoveryEnded);
        info!("Discovery Thread: Ended.");
    });
}

fn start_notification_listener(tx: mpsc::Sender<Message>,
                               ctrl: Arc<Mutex<WeeController>>) {

    thread::spawn(move || {
        let notifications;
        {
            let mut controller = ctrl.lock().unwrap();
            notifications = controller.subscription_future().ok();
        }
        // This message causes the UI to draw first time.
        let _ = tx.send(Message::NotificationsStarted);
        if let Some(notifications) = notifications {
            let mut core = Core::new().unwrap();
            let processor = notifications.for_each(|n| {
                let _ = tx.send(Message::Notification(n));
                Ok(())
            });
            core.run(processor).unwrap();
        }
    });
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
    let refresh_image_bytes = include_bytes!("../assets/images/refresh.png");
    let clear_image_bytes = include_bytes!("../assets/images/clear.png");
    let refresh_press_image_bytes = include_bytes!("../assets/images/refresh_press.png");
    let clear_press_image_bytes = include_bytes!("../assets/images/clear_press.png");
    let refresh_hover_image_bytes = include_bytes!("../assets/images/refresh_hover.png");
    let clear_hover_image_bytes = include_bytes!("../assets/images/clear_hover.png");

    // Build the window.
    let display = glium::glutin::WindowBuilder::new()
        .with_vsync()
        .with_dimensions(WINDOW_WIDTH as u32, WINDOW_HEIGHT as u32)
        .with_title("Wee Controller")
        .build_glium()
        .unwrap();

    let mut image_map = conrod::image::Map::new();

    let image_ids = ImageIds {
        refresh_normal: image_map.insert(load_image(refresh_image_bytes, &display)),
        refresh_press: image_map.insert(load_image(refresh_press_image_bytes, &display)),
        refresh_hover: image_map.insert(load_image(refresh_hover_image_bytes, &display)),
        clear_normal: image_map.insert(load_image(clear_image_bytes, &display)),
        clear_press: image_map.insert(load_image(clear_press_image_bytes, &display)),
        clear_hover: image_map.insert(load_image(clear_hover_image_bytes, &display)),
    };

    // Construct our `Ui`.
    let mut renderer = conrod::backend::glium::Renderer::new(&display).unwrap();

    // A channel to send events from the main `winit` thread to the conrod thread.
    let (event_tx, event_rx): (std::sync::mpsc::Sender<Message>, std::sync::mpsc::Receiver<Message>) = std::sync::mpsc::channel();
    // A channel to send `render::Primitive`s from the conrod thread to the `winit thread.
    let (render_tx, render_rx) = std::sync::mpsc::channel();
    // This window proxy will allow conrod to wake up the `winit::Window` for rendering.
    let window_proxy = display.get_window().unwrap().create_window_proxy();

    // A function that runs the conrod loop.
    fn run_conrod(weecontrol: Arc<Mutex<WeeController>>,
                  image_ids: &ImageIds,
                  event_emit: std::sync::mpsc::Sender<Message>,
                  event_rx: std::sync::mpsc::Receiver<Message>,
                  render_tx: std::sync::mpsc::Sender<conrod::render::OwnedPrimitives>,
                  window_proxy: glium::glutin::WindowProxy) {

        // Initial state
        let mut app_state = AppState::Initializing;

        let mut notification_diag = NotificationDialog { is_open: false, message: String::new()};

        let mut ui = conrod::UiBuilder::new([WINDOW_WIDTH as f64, WINDOW_HEIGHT as f64]).build();

        // Unique identifier for each widget.
        let ids = Ids::new(ui.widget_id_generator());

        use conrod::text::FontCollection;
        let font_bytes = include_bytes!("../assets/fonts/NotoSans/NotoSans-Regular.ttf");
        let font =
            FontCollection::from_bytes(&font_bytes[0..font_bytes.len()]).into_font().unwrap();
        ui.fonts.insert(font);

        let mut list: Vec<(DeviceInfo, bool)> = Vec::new();
        //let mut weecontrol = Arc::new(Mutex::new(WeeController::new(Some(logger))));
        start_notification_listener(event_emit.clone(), weecontrol.clone());
        // Many widgets require another frame to finish drawing after clicks or hovers, so we
        // insert an update into the conrod loop using this `bool` after each event.
        let mut needs_update = true;
        'conrod: loop {

            if app_state == AppState::StartDiscovery {
                list.clear();
                app_state = AppState::Discovering;
                start_discovery_async(event_emit.clone(), weecontrol.clone());
            }

            // Collect any pending events.
            let mut events = Vec::new();
            while let Ok(event) = event_rx.try_recv() {
                events.push(event);
            }

            // If there are no events pending, wait for them.
            if events.is_empty() || !needs_update {
                match event_rx.recv() {
                    Ok(event) => events.push(event),
                    Err(_) => break 'conrod,
                };
            }

            needs_update = false;

            // Input each event into the `Ui`.
            for event in events {
                match event {
                    Message::Event(e) => {
                        ui.handle_event(e);
                    },
                    Message::DiscoveryEnded => {
                        if list.is_empty() {
                            app_state = AppState::NoDevices;
                        } else {
                            app_state = AppState::Ready;
                        }
                    },
                    Message::DiscoveryItem(i) => {
                        info!("UI Got device {:?}", i.unique_id);
                        let i_state: bool = if i.state == State::On { true } else { false };
                        {
                            let mut controller = weecontrol.lock().unwrap();
                            let _ = controller.subscribe(&i.unique_id, SUBSCRIPTION_DURATION, true);
                        }
                        list.push((i, i_state));
                    },
                    Message::NotificationsStarted => (),
                    Message::Notification(n) => update_list(&mut list, &n),
                }
                needs_update = true;
            }

            set_ui(ui.set_widgets(),
                   &mut list,
                   &ids,
                   &mut app_state,
                   weecontrol.clone(),
                   &mut notification_diag,
                   image_ids);

            // Render the `Ui` to a list of primitives that we can send to the main thread for
            // display.
            if let Some(primitives) = ui.draw_if_changed() {
                if render_tx.send(primitives.owned()).is_err() {
                    break 'conrod;
                }
                // Wakeup `winit` for rendering.
                window_proxy.wakeup_event_loop();
            }
        }
    } // fn run_conrod

    // Spawn the conrod loop on its own thread.
    let weecontrol = Arc::new(Mutex::new(WeeController::new(Some(logger.clone()))));
    let control_clone = weecontrol.clone();
    let event_emitter = event_tx.clone();
    std::thread::spawn(move || {
        run_conrod(control_clone,
                   &image_ids,
                   event_emitter,
                   event_rx,
                   render_tx,
                   window_proxy)
    });

    // Run the `winit` loop.
    let mut last_update = std::time::Instant::now();
    'main: loop {

        // We don't want to loop any faster than 60 FPS, so wait until it has been at least
        // 16ms since the last yield.
        let sixteen_ms = std::time::Duration::from_millis(16);
        let now = std::time::Instant::now();
        let duration_since_last_update = now.duration_since(last_update);
        if duration_since_last_update < sixteen_ms {
            std::thread::sleep(sixteen_ms - duration_since_last_update);
        }

        // Collect all pending events.
        let mut events: Vec<_> = display.poll_events().collect();

        // If there are no events, wait for the next event.
        if events.is_empty() {
            events.extend(display.wait_events().next());
        }

        // Send any relevant events to the conrod thread.
        for event in events {

            // Use the `winit` backend feature to convert the winit event to a conrod one.
            if let Some(event) = conrod::backend::winit::convert(event.clone(), &display) {
                event_tx.send(Message::Event(event)).unwrap();
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

        // Draw the most recently received `conrod::render::Primitives` sent from the `Ui`.
        if let Ok(mut primitives) = render_rx.try_recv() {
            while let Ok(newest) = render_rx.try_recv() {
                primitives = newest;
            }

            renderer.fill(&display, primitives.walk(), &image_map);
            let mut target = display.draw();
            target.clear_color(0.0, 0.0, 0.0, 1.0);
            renderer.draw(&display, &mut target, &image_map).unwrap();
            target.finish().unwrap();
        }

        last_update = std::time::Instant::now();
    }
    let mut controller = weecontrol.lock().unwrap();
    controller.unsubscribe_all();
}

// Declare the `WidgetId`s and instantiate the widgets.
fn set_ui(ref mut ui: conrod::UiCell,
          list: &mut Vec<(DeviceInfo, bool)>,
          ids: &Ids,
          app_state: &mut AppState,
          weecontrol: Arc<Mutex<WeeController>>,
          notification_diag: &mut NotificationDialog,
          image_ids: &ImageIds) {

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

    if widget::Button::image(image_ids.refresh_normal)
        .press_image(image_ids.refresh_press)
        .hover_image(image_ids.refresh_hover)
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
    if widget::Button::image(image_ids.clear_normal)
        .press_image(image_ids.clear_press)
        .hover_image(image_ids.clear_hover)
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

    if *app_state == AppState::Ready || (*app_state == AppState::Discovering && !list.is_empty()) {
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
        if *app_state == AppState::Discovering {
            widget::Text::new("Searching...")
                .color(color::YELLOW)
                .w_h(180.0, 20.0)
                .bottom_left_with_margins_on(ids.canvas, 15.0, 15.0)
                .set(ids.scanning_label, ui);
        }
    } else {
        match *app_state {
            AppState::Ready => (),
            AppState::StartDiscovery => {}
            AppState::Initializing => {
                widget::Text::new("Scanning for devices...")
                    .color(color::LIGHT_YELLOW)
                    .middle_of(ids.list_canvas)
                    .set(ids.wait_label, ui);
                *app_state = AppState::StartDiscovery;
            }
            AppState::Discovering => {
                widget::Text::new("Scanning for devices...")
                    .color(color::LIGHT_YELLOW)
                    .middle_of(ids.list_canvas)
                    .set(ids.wait_label, ui);
            }
            AppState::NoDevices => {
                widget::Text::new("No devices found.")
                    .color(color::LIGHT_YELLOW)
                    .middle_of(ids.list_canvas)
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
