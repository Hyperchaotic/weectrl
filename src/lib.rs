//! A library for finding and managing Belkin WeMo switches and devices, or compatible devices.
//!
//! WeMo devices use uPnP and HTTP/SOAP for network anouncements and control. The `WeeController`
//! encapsulates this and offers a simpler API for discovery and management.
//! Discovery can be synchronous or asynchronous and a list of known devices is cached on disk
//! for quick startup.

extern crate reqwest;

extern crate serde;
extern crate serde_xml_rs;

extern crate serde_derive;

extern crate url;

pub mod weectrl;
pub use weectrl::{
    DeviceInfo, DiscoveryMode, Icon, Model, State, StateNotification, WeeController,
};

pub mod error;

mod cache;
mod device;
mod rpc;
mod xml;
