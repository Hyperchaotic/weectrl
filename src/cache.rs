

extern crate app_dirs;
extern crate serde_json;

use std;

use std::io::prelude::*;
use std::fs::File;

#[derive(Debug, Serialize, Deserialize)]
pub struct DeviceAddress {
    pub location: String,
    pub mac_address: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct CacheData {
    pub cache: Vec<DeviceAddress>,
}

use self::app_dirs::*;

const CACHE_FILE: &'static str = "IPCache.dat";

const APP_INFO: AppInfo = AppInfo {
    name: "WeeApp",
    author: "hyperchaotic",
};

/// Disk cache of IP addresses of WeMo devices
/// WeMo doesn't always answer broadcast upnp, this cache allows
/// for the persistent storage of known devices for unicast discovery
pub struct DiskCache {
    cache_file: Option<std::path::PathBuf>,
}

impl DiskCache {
    pub fn new() -> DiskCache {
        let mut file_path: Option<std::path::PathBuf> = None;
        if let Some(mut path) = app_root(AppDataType::UserCache, &APP_INFO).ok() {
            path.push(CACHE_FILE);
            file_path = Some(path);
        }
        DiskCache { cache_file: file_path }
    }

    /// Write data to cache, errors ignored
    pub fn write(&self, addresses: Vec<DeviceAddress>) {

        if let Some(ref fpath) = self.cache_file {
            let data = CacheData { cache: addresses };

            if let Some(serialized) = serde_json::to_string(&data).ok() {
                if let Some(mut buffer) = File::create(fpath).ok() {
                    let _ = buffer.write_all(&serialized.into_bytes());
                }
            }
        }
    }

    pub fn read(&self) -> Option<Vec<DeviceAddress>> {

        if let Some(ref fpath) = self.cache_file {
            if let Some(mut file) = File::open(fpath).ok() {
                let mut s = String::new();
                let _ = file.read_to_string(&mut s);
                let cachedata: Option<CacheData> = serde_json::de::from_str(&s).ok();
                if let Some(data) = cachedata {
                    return Some(data.cache);
                }
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
