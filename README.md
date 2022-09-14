# weectrl   [![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT) [![License](https://img.shields.io/badge/License-Apache%202.0-blue.svg)](https://opensource.org/licenses/Apache-2.0)

A cross platform library and application for controlling Belkin WeMo switches and Sockets. Written in Rust.

## WeeApp, the application

### Screenshots

![Linux](http://i.imgur.com/4QutZDQ.png "Linux")   ![Windows](http://i.imgur.com/PoNrogW.png "Windows")   ![macOS](http://i.imgur.com/s4XsDnA.png "macOS")

### Functionality
The "WeeApp" application will scan the local network for Belkin WeMo devices and list what's found. They can be switched on/off from the app and if a device changes state due to external activity (e.g. physical toggle or schedule) this will be reflected in the app UI due to the notification feature.

Searching for new devices can take 5-6 seconds but the app benefits from the caching feature of the library so previously known devices will be displayed very quickly upon restart.   

* The "paper basket" will forget all devices and erase the disk cache.
* The reload button will load known devices from cache and rescan the network.
* The application remembers size and position of its window. 

### Platforms
Current version tested on Windows 11 and Linux. It should build on macOS.

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
extern crate weectrl;

use weectrl::*;

let mut controller = WeeController::new();
```

To discover devices on network or/and in cache asynchronously:
``` rust


let rx: mpsc::Receiver<DeviceInfo> = self.controller.discover_async(
    DiscoveryMode::CacheAndBroadcast, 
    true, 
    5, 
    );
```
Scans both disk cache file and network, will "forget" in-memory list first. Give network devices maximum 5 seconds to respond.
The ```Receiver``` will receive device information as they're found and when discovery ends the channel will be closed.

Blocking version of the discover function.
``` rust
let list: Option<Vec<DeviceInfo>> = controller.discover(DiscoveryMode::CacheAndBroadcast, true, 5);
```
Returns when scan is done. Can take many seconds.


#### Switching and notifications

Let `unique_id` be a `DeviceInfo::unique_id` returned by previous discovery.

Starting listening for notifications from subscribed devices:
``` rust
let rx: mpsc::Receiver<StateNotification> = controller.start_subscription_service().unwrap();
```
Whenever a switch is toggled a message will appear on the Receiver. This channel will live for the duration
of the application. 

Subscribe to notifications for a device and instruct WeeController to automatically refresh subscriptions every 120 seconds before they expire:
``` rust
controller.subscribe(&unique_id, 120, true).unwrap();
```

Unsubscribe from notifications from one device:
``` rust
controller.unsubscribe(&unique_id).unwrap();
```

Unsubscribe from all notifications.
``` rust
controller.unsubscribe_all();
```

To toggle a switch:
``` rust
let res = controller.set_binary_state(unique_id, State::On);
 match res {
    Ok(state) => println!("Ok({:?})", state),
    Err(err) => println!("Err({:?})", err),
}
```

To get a switch state:
``` rust
let res = controller.get_binary_state(unique_id);
 match res {
    Ok(state) => println!("Ok({:?})", state),
    Err(err) => println!("Err({:?})", err),
}
```
## Compatibility
Theoretically supports and device advertising the `urn:Belkin:service:basicevent:1` service. Does not support dimmers, motion sensors, yet.
Currently only tested with Belkin WeMo LightSwitch and Socket.

## License

Licensed under either of

 * Apache License, Version 2.0, ([LICENSE-APACHE](LICENSE-APACHE) or http://www.apache.org/licenses/LICENSE-2.0)
 * MIT license ([LICENSE-MIT](LICENSE-MIT) or http://opensource.org/licenses/MIT)
