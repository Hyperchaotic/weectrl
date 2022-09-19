extern crate directories;
use directories::ProjectDirs;

use tracing::info;

use serde::{Deserialize, Serialize};

use std::fs::File;
use std::io::prelude::*;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceAddress {
    pub location: String,
    pub mac_address: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
struct CacheData {
    pub cache: Vec<DeviceAddress>,
}

const CACHE_FILE: &str = "IPCache.dat";

/// Disk cache of IP addresses of WeMo devices
/// WeMo doesn't always answer broadcast upnp, this cache allows
/// for the persistent storage of known devices for unicast discovery
pub struct DiskCache {
    cache_file: Option<std::path::PathBuf>,
}

impl DiskCache {
    pub fn new() -> Self {
        let mut file_path: Option<std::path::PathBuf> = None;

        if let Some(proj_dirs) = ProjectDirs::from("", "", "WeeCtrl") {
            let mut path = proj_dirs.config_dir().to_path_buf();
            path.push(CACHE_FILE);
            file_path = Some(path);
            info!("Cache file: {:#?}", file_path);
        }
        Self {
            cache_file: file_path,
        }
    }

    /// Write data to cache, errors ignored as everything will still work
    /// without a cache, just slower.
    pub fn write(&self, addresses: Vec<DeviceAddress>) {
        if let Some(ref fpath) = self.cache_file {
            let data = CacheData { cache: addresses };
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

    pub fn read(&self) -> Option<Vec<DeviceAddress>> {
        if let Some(ref fpath) = self.cache_file {
            if let Ok(mut file) = File::open(fpath) {
                let mut s = String::new();
                let _ignore = file.read_to_string(&mut s);
                let cachedata: Option<CacheData> = serde_json::from_str(&s).ok();
                if let Some(data) = cachedata {
                    return Some(data.cache);
                }
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
