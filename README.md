# weectrl   [![Build Status](https://travis-ci.org/Hyperchaotic/weectrl.svg?branch=master)](https://travis-ci.org/Hyperchaotic/weectrl) [![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT) [![License](https://img.shields.io/badge/License-Apache%202.0-blue.svg)](https://opensource.org/licenses/Apache-2.0)

A cross platform library and application for controlling Belkin WeMo switches and Sockets. Written in Rust.

## Wee Controller, the application

### Screenshots

![Linux](http://i.imgur.com/26aRUfE.png "Linux")   ![Windows](http://i.imgur.com/PoNrogW.png "Windows")


### Functionality
The "Wee Controller" application will scan the local network for Belkin WeMo devices and list what's found. They can be switched on/off from the app and if a device changes state due to external activity (e.g. physical toggle or schedule) this will be reflected in the app UI due to the notification feature.

Searching for new devices can take 5-10 seconds but the app benefits from the caching feature of the library so previously known devices will be displayed very quickly upon restart.   

* The "paper basket" will forget all devices erase the disk cache.
* The reload button will load known devices from cache and rescan the network - fast.

### Platforms
Currently tested on Windows (10) and Linux (Mint KDE). It should work on macOS and might work on platforms supported by the Conrod (+Glium/Winit) toolkits.

## weectrl, the library
### Functionality 
* Discover devices synchronously or asynchronously.
* Retrieve detailed device information.
* Switch devices on or off.
* Cache known devices on disk for quick reload.
* Subscription to state change notifications from devices.
* Uses slog for logging.

### API examples
Note: Binding types included for clarity. Full API described in source doc in weectrl.rs
#### Initialization and discovery
Create new instance of controller:
``` rust
extern crate weectrl;

use weectrl::weectrl::*;
use weectrl::device::State;

let mut weecontrol = WeeController::new(None);
```

To discover devices on network or/and in cache asynchrously use:
``` rust
let rx: mpsc::Receiver<DeviceInfo> = controller.discover_async(DiscoveryMode::CacheAndBroadcast, true, 3);
```
Scans both cache and network, will "forget" non-responding devices, give network devices maximum 3 seconds to respond. 
The returned channel will receive device information as they're found and when discovery ends it will be closed.

To discover devices on network or/and in cache synchrously use:
``` rust
let list: Option<Vec<DeviceInfo>> = controller.discover_async(DiscoveryMode::CacheAndBroadcast, true, 3);
```
Returns when scan is done. Can take many seconds.


#### Switching and notifications

Let `unique_id` be a `DeviceInfo::unique_id` returned by previous discovery.

Starting listening for notifications from subscribed devices:
``` rust
let notification_mpsc = mpsc::Receiver<StateNotification>> = controller.start_subscription_service().unwrap();
```
Whenever a switch is toggled a message will appear on `notification_mpsc`. This channel will stay alive for the duration of the process.


Subscribe to notifications for a device and instruct WeeController to automatically refresh subscriptions before they expire:
``` rust
controller.subscribe(&unique_id, 120, true).unwrap();
```

Unsubscribe from notifications from one device:
``` rust
controller.unsubscribe(&unique_id).unwrap();
```

Unsubscribe from all notifications (good practice to do before the process terminates):
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
Currently tested with Belkin WeMo LightSwitch and Socket only.

## License

Licensed under either of

 * Apache License, Version 2.0, ([LICENSE-APACHE](LICENSE-APACHE) or http://www.apache.org/licenses/LICENSE-2.0)
 * MIT license ([LICENSE-MIT](LICENSE-MIT) or http://opensource.org/licenses/MIT)
 
