//! A library for finding and managing Belkin WeMo switches and devices, or compatible devices.
//!
//! WeMo devices use uPnP and HTTP/SOAP for network anouncements and control. The `WeeController`
//! encapsulates this and offers a simpler API for discovery and management.
//! Discovery can be synchronous or asynchronous and a list of known devices is cached on disk
//! for quick startup.

#[macro_use] extern crate hyper;
#[macro_use] extern crate serde_derive;
#[macro_use] extern crate slog;
extern crate slog_stdlog;
extern crate slog_scope;

extern crate serde;
extern crate serde_xml;
extern crate url;

pub mod weectrl;
pub mod device;
pub mod xml;
pub mod cache;
pub mod rpc;
pub mod error;
