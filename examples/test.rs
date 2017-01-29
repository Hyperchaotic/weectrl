#[macro_use(slog_info, slog_log, o)]
extern crate slog;

#[macro_use]
extern crate slog_scope;
extern crate slog_stream;
extern crate chrono;
extern crate app_dirs;
use app_dirs::*;

use std::io;
use std::fs::OpenOptions;

use slog::DrainExt;

extern crate weectrl;
extern crate hyper;

use hyper::*;
use weectrl::weectrl::*;
use weectrl::device::State;
use std::net::{IpAddr, Ipv4Addr};

use std::net;
use std::time::Duration;
use std::{thread, time};

use std::io::prelude::*;
use std::net::TcpStream;

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

const LOG_FILE: &'static str = "WeeController.log";

const APP_INFO: AppInfo = AppInfo {
    name: "WeeApp",
    author: "hyperchaotic",
};

use std::prelude::v1::Result::Ok;

fn main() {
    // get path for logfile, like so: %USERPROFILE%\AppData\Local\hyperchaotic\WeeApp\WeeController.log
    //                            or: $HOME/.cache/WeeApp/WeeController.log
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

    let mut wee = WeeController::new(Some(logger));
    //wee.discover(3);
    //let rx = wee.start_subscription_service().unwrap();

    let device = wee.add_device(IpAddr::V4(Ipv4Addr::new(192,168,0,16)));
    match device {
        Ok(o) => println!("Found: {:?} {:?} {:?} {:?}.", o.friendly_name, o.hostname, o.unique_id, o.state),
        Err(e) => println!("Error: {:?}", e),
    };


    //let res = wee.subscribe("94103E4830D0", 30, true);
    //let res = wee.subscribe("94103E450AAC", 300);

//    println!("====> a {:?}", res);

/*    let millis = time::Duration::from_secs(20);
    thread::sleep(millis);

    let res = wee.resubscribe("94103E4830D0", 30);

    println!("====> b {:?}", res);
*/

    // let state = wee.get_binary_state("94103E4830D0");
    // println!("State {:?}", state);
    // wee.set_binary_state("94103E4830D0", WeeState::Off);
    // let state = wee.get_binary_state("94103E4830D0");
    // println!("State {:?}", state);
    // wee.set_binary_state("94103E4830D0", WeeState::On);
    //

/*
    fn hello(req: mut Request, res: Response) {
        let mut data: Vec<u8> = vec![];
        io::copy(&mut req, &mut data).unwrap();
        println!(" REQUEST {:?}", data);

    }

    Server::http("0.0.0.0:0").unwrap().handle(hello).unwrap();*/

    use std::io;
    use hyper::server::{Server, Request, Response};
    use hyper::status::StatusCode;
    use std::io::Read;
/*
    println!("====================================");

    Server::http("0.0.0.0:8787").unwrap().handle(|mut req: Request, mut res: Response| {
        println!(" Got request");

        let mut data = String::new();
        if let Some(size) = req.read_to_string(&mut data).ok() {
            println!("{:?}", data);
        }

        match req.method {
            hyper::Post => {
                io::copy(&mut req, &mut res.start().unwrap()).unwrap();
            },
            hyper::Get => {
                println!("yay");
                let mut reader: &[u8] = b"<html><body><h1>200 OK</h1></body></html>";
                res.headers_mut().set_raw("Content-Type", vec!("text/html".to_string().into_bytes()));
                res.headers_mut().set_raw("Content-Length", vec!(reader.len().to_string().into_bytes()));
                res.headers_mut().set_raw("Connection", vec!("Close".to_string().into_bytes()));
                io::copy(&mut reader, &mut res.start().unwrap()).unwrap();
            },
            _ => *res.status_mut() = StatusCode::MethodNotAllowed
        }
    }).unwrap();
*/
    use std::result::Result;
    let mut input = String::new();
    match io::stdin().read_line(&mut input) {
        Result::Ok(n) => {
            println!("{} bytes read", n);
            println!("{}", input);
        }
        Result::Err(error) => println!("error: {}", error),
    }
}
