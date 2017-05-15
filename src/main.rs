#![feature(plugin)]

extern crate clap;
extern crate serde;
#[macro_use]
extern crate serde_json;
#[macro_use]
extern crate serde_derive;
extern crate futures;
extern crate hyper;
extern crate tokio_core;
extern crate itertools;

use futures::future::FutureResult;
use hyper::{Get, Post, StatusCode};
use hyper::header::ContentLength;
use hyper::server::{Http, Service, Request, Response};
use hyper::client::Request as ClientRequest;
static INDEX: &'static [u8] = b"Try POST /echo";
use clap::{Arg, App, SubCommand};
use std::io::BufReader;
use futures::{Future, Stream};
use futures::future::join_all;
use tokio_core::reactor::Core;
use std::io;
use std::io::Write;
use std::io::stdout;
use std::str::from_utf8;
use hyper::{Client, Chunk};
use serde_json::Value;
use std::sync::Arc;
use itertools::interleave;
use std::str::FromStr;
use hyper::Method;
use hyper::Uri;
use std::sync::Mutex;
use std::cell::RefCell;
use futures::future::{ok};
use std::io::Read;
use std::ops::Deref;

#[derive(Debug, Serialize, Deserialize, Clone)]
struct ChromeDriver {
    name: String,
    pub ip: String,
    #[serde(default)]
    disabled: bool,
    #[serde(default)]
    blocked: bool,
    #[serde(default)]
    current_browsers_count: u32,
}

struct Test {
    drivers: Arc<Mutex<Vec<ChromeDriver>>>,
}

fn update_drivers_information(drivers: &mut Vec<ChromeDriver>) {
    let mut core = tokio_core::reactor::Core::new().unwrap();
    let handle = core.handle();
    let client = &Client::new(&handle);
    let work = drivers.iter_mut()
        .map(move |element| {
            let url =
                format!("http://{}/sessions", element.ip).as_str().parse::<hyper::Uri>().unwrap();
            client.get(url.clone()).then(move |response| {
                match response {
                    Ok(success_response) => {
                        success_response.body()
                            .fold::<_, String, _>(String::new(), move |mut acc, chunk| {
                                let mut z: &[u8] = &*chunk;
                                z.read_to_string(&mut acc);
                                let result: Result<String, hyper::Error> = Ok(acc);
                                result
                            })
                            .map_err(|_| ())
                            .and_then(move |response_payload| {
                                element.disabled = false;
                                let value: Value = serde_json::from_str(&response_payload.as_str())
                                    .unwrap();
                                element.current_browsers_count =
                                    value["value"].as_array().unwrap().len() as u32;
                                Ok::<(), ()>(())
                            })
                            .wait();
                        Ok::<(), ()>(())
                    }
                    Err(error_response) => {
                        element.disabled = true;
                        Ok::<(), ()>(())
                    }
                }

            })
        })
        .collect::<Vec<_>>();
    let work = join_all(work);
    core.run(work).unwrap();
}

fn get_minimal_loaded_driver(mut drivers: &mut Vec<ChromeDriver>) -> hyper::server::Response {
    update_drivers_information(&mut drivers);
    let mut minimal_loaded_driver = drivers.iter_mut()
        .filter(|driver| {
            println!("driver: {:?}", driver);
            !driver.blocked && !driver.disabled
        })
        .min_by_key(|driver| driver.current_browsers_count);
    match minimal_loaded_driver {
        Some(driver_instance) => {
            driver_instance.blocked = true;
            let driver_json_representation = serde_json::to_string(&driver_instance).unwrap();
            Response::new()
                .with_header(ContentLength(driver_json_representation.len() as u64))
                .with_body(driver_json_representation)
        }
        None => {
            Response::new()
                .with_header(ContentLength(r#"{ "error": "Ни один вебдрайвер из конфигурации не доступен." }"#.len() as u64))
                .with_status(StatusCode::NotFound)
                .with_body(r#"{ "error": "Ни один вебдрайвер из конфигурации не доступен." }"#)
        }
    }
}

fn unlock_driver(mut drivers: &mut Vec<ChromeDriver>, driver_ip_to_unlock: &str) -> hyper::server::Response {
    let mut driver_to_unlock_option = drivers
        .iter_mut()
        .find(|driver| driver.ip == driver_ip_to_unlock);
    match driver_to_unlock_option {
        Some(driver_to_unlock) => {
            driver_to_unlock.blocked = false;
            Response::new()
                .with_header(ContentLength(r#"{ "status": "Указанный драйвер разблокирован." }"#.len() as u64))
                .with_body(r#"{ "status": "Указанный драйвер разблокирован." }"#)
        },
        None => {
            Response::new()
                .with_header(ContentLength(r#"{ "error": "Указанный драйвер для разблокировки не найден в списке. Проверьте конфигурацию grid_rs." }"#.len() as u64))
                .with_status(StatusCode::NotFound)
                .with_body(r#"{ "error": "Указанный драйвер для разблокировки не найден в списке. Проверьте конфигурацию grid_rs." }"#)
        }
    }


}

impl Service for Test {
    type Request = Request;
    type Response = Response;
    type Error = hyper::Error;
    type Future = Box<Future<Item = self::Response, Error = hyper::error::Error>>;

    fn call(&self, req: Request) -> Self::Future {
        match (req.method(), req.path()) {
            (&Get, "/url") => Box::new(ok(get_minimal_loaded_driver(&mut self.drivers.lock().unwrap()))),
            (&Post, "/unlock") => {
                let body = Vec::new();
                let mut drivers = self.drivers.clone();
                Box::new(req.body()
                    .fold(body, move |mut body, chunk| {
                        body.extend_from_slice(&chunk);
                        Ok::<Vec<u8>, hyper::Error>(body)
                    })
                    .map(move |payload| {
                        let mut drivers = drivers.lock().unwrap();
                        let driver_ip_to_unlock : Value = serde_json::from_slice(&payload).unwrap();
                        let driver_ip_to_unlock : &str = driver_ip_to_unlock["ip"].as_str().unwrap();
                        unlock_driver(&mut drivers, driver_ip_to_unlock)
                    }))
            },
            (&Get, "/status") => {
                let mut drivers = self.drivers.clone();
                let mut drivers = drivers.lock().unwrap();
                let mut status = serde_json::to_string(&drivers.deref()).unwrap();
                let status_len : u64 = status.len() as u64;
                Box::new(ok(Response::new().with_status(StatusCode::Ok).with_body(status).with_header(ContentLength(status_len))))
            },
            _ => Box::new(ok(Response::new().with_status(StatusCode::NotFound))),
        }
    }
}

fn main() {
    let matches = App::new("Grid rs")
        .version("1.0")
        .author("Arsen Galimov")
        .about("Load balancer for WebDrivers")
        .arg(Arg::with_name("config")
            .short("c")
            .required(true)
            .long("config")
            .value_name("FILE")
            .help("Sets a custom config file")
            .takes_value(true))
        .arg(Arg::with_name("port")
            .short("p")
            .required(true)
            .long("port")
            .value_name("PORT")
            .help("Sets a port")
            .takes_value(true))
        .get_matches();
    let file = std::fs::File::open(matches.value_of("config").unwrap()).unwrap();
    let mut buf_reader = BufReader::new(file);
    let chrome_drivers: Arc<Mutex<Vec<ChromeDriver>>> =
        Arc::new(Mutex::new(serde_json::from_reader(buf_reader).unwrap()));
    let addr = "172.16.124.165:8000".parse().unwrap();
    let server = Http::new()
        .bind(&addr, move || Ok(Test { drivers: chrome_drivers.clone() }))
        .unwrap();
    println!("Listening on http://{} with 1 thread.",
             server.local_addr().unwrap());
    server.run().unwrap();
}
