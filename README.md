# weectrl   [![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT) [![License](https://img.shields.io/badge/License-Apache%202.0-blue.svg)](https://opensource.org/licenses/Apache-2.0)

A cross platform library and application for controlling Belkin WeMo switches and Sockets. Written in Rust.

## WeeApp, the application

### Screenshots
![Screenshot-macOS](https://user-images.githubusercontent.com/19599562/193374117-9890c33d-c4f9-49ca-a1b2-9010daaecfa3.png)
![info](https://user-images.githubusercontent.com/19599562/194636095-28ebccc5-7ab9-49e7-a1a8-acd7b77c67cf.png)
![Screenshot-Windows](https://user-images.githubusercontent.com/19599562/193374122-f3d494d4-3872-44c6-9ef6-7cf67923f16b.png)
![Screenshot-Linux](https://user-images.githubusercontent.com/19599562/193374125-3b5d8d51-763a-40b0-b8f8-bb21c9a5f4fc.png)

### Functionality
The "WeeApp" application will scan the local network for Belkin WeMo devices and list what's found. They can be switched on/off from the app and if a device changes state due to external activity (e.g. physical toggle or schedule) this will be reflected in the app UI due to the notification feature.

Searching for new devices can take 5-6 seconds but the app benefits from the caching feature of the library so previously known devices will be displayed very quickly upon restart.   

* The "paper basket" will forget all devices and erase the disk cache.
* The reload button will load known devices from cache and rescan the network.
* The application remembers size and position of its window. 

### Platforms
Current version tested on Windows 11, Linux and maOS x84_64.

### Building

The (example) App has certain FLTK_rs [dependencies](https://fltk-rs.github.io/fltk-book/Setup.html).

```
cargo build --release --example weeapp
```
#### Windows addendum
An application icon can be found in `examples/images/icon.ico`. To insert it into the Windows binary use [rcedit][56bbd8db]:
```
rcedit target\release\examples\weeapp.exe --set-icon examples\images\icon.ico
```
  [56bbd8db]: https://github.com/electron/rcedit/releases "rcedit"

## weectrl, the library
### Functionality
* Discover devices synchronously or asynchronously (threaded).
* Retrieve detailed device information.
* Switch devices on or off.
* Cache known devices on disk for quick reload.
* Subscription to state change notifications from devices.
* Uses the Tracing crate for logging.

### API examples

#### Initialization and discovery
Create new instance of controller:
``` rust

use weectrl::*;

let controller = WeeController::new();
```

To discover devices on network or/and in cache asynchronously:
``` rust

let rx: std::sync::mpsc::Receiver<DeviceInfo> = controller.discover(
    DiscoveryMode::CacheAndBroadcast, 
    true, 
    5, 
    );
```
Scans both disk cache file and network, will "forget" in-memory list first. Give network devices maximum 5 seconds to respond.
When discovery ends the channel will be closed.

Futures version of the discover function.
``` rust

let notifications: futures::channel::mpsc::UnboundedReceiver<DeviceInfo> = controller.discover_future(
    DiscoveryMode::CacheAndBroadcast, 
    true, 
    5, 
    );
```


#### Switching and notifications

Let `unique_id` be a `DeviceInfo::unique_id` returned by previous discovery.

Starting listening for notifications from subscribed devices. If called several times only the most recent `rx` channel will receive notifications:
``` rust

let rx: std::sync::mpsc::Receiver<StateNotification> = controller.notify();
```
Whenever a switch is toggled a message will appear on the Receiver.

Futures version.
``` rust

let notifications_fut: futures::channel::mpsc::UnboundedReceiver<StateNotification> = controller.notify_future();
```

Subscribe to notifications for a device and instruct WeeController to automatically refresh subscriptions every 120 seconds before they expire.
As this causes a network request to be made to the switch a 5 second network timeout is set for the request in case the switch doesn't respond:
``` rust

let result = controller.subscribe(&unique_id, Duration::from_secs(120), true, Duration::from_secs(5));
```

Unsubscribe from notifications from one device:
``` rust

let result = controller.unsubscribe(&unique_id, Duration::from_secs(5));
```

Unsubscribe from all notifications:
``` rust

controller.unsubscribe_all();
```

To toggle a switch (on or off):
``` rust

let result = controller.set_binary_state(unique_id, State::On, Duration::from_secs(5));
 match result {
    Ok(state) => println!("Ok({:?})", state),
    Err(err) => println!("Err({:?})", err),
}
```

To get a switch state:
``` rust

let result = controller.get_binary_state(unique_id, Duration::from_secs(5));
 match result {
    Ok(state) => println!("Ok({:?})", state),
    Err(err) => println!("Err({:?})", err),
}
```

Retrive icons stored in a switch:
``` rust

let list: Result<Vec<Icon>, Error> = controller.get_icons(unique_id, Duration::from_secs(5));
```

Retrieve structure with information stored about a switch, from the controller. This is the same structure returned earlier during Discovery:
``` rust

let result = controller.get_info(unique_id);
```


## Compatibility
Theoretically supports and device advertising the `urn:Belkin:service:basicevent:1` service. Tested with Lightswitch and Socket.
Currently only tested with Belkin WeMo LightSwitch and Socket.

## License

Licensed under either of

 * Apache License, Version 2.0, ([LICENSE-APACHE](LICENSE-APACHE) or http://www.apache.org/licenses/LICENSE-2.0)
 * MIT license ([LICENSE-MIT](LICENSE-MIT) or http://opensource.org/licenses/MIT)
