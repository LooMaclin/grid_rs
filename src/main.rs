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
use std::io::Read;
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
                println!("url: {:?}", url);
                println!("response: {:?}", response);
                match response {
                    Ok(success_response) => {
                        success_response
                            .body()
                            .fold::<_, String, _>(String::new(), move |mut acc, chunk| {
                                let mut z: &[u8] = &*chunk;
                                z.read_to_string(&mut acc);
                                let result: Result<String, hyper::Error> = Ok(acc);
                                result
                            })
                            .then(move |response_payload| {
                                let response_payload =
                                    response_payload.unwrap_or(String::from("Ошибка"));
                                if response_payload == "Ошибка" {
                                    element.disabled = true;
                                    Ok::<(), ()>(())
                                } else {
                                    element.disabled = false;
                                    let value: Value = serde_json::from_str(&response_payload.as_str())
                                        .unwrap();
                                    element.current_browsers_count =
                                        value["value"].as_array().unwrap().len() as u32;
                                    Ok::<(), ()>(())
                                }
                            });
                        Ok::<(), ()>(())
                    },
                    Err(error_response) => {
                        Ok::<(), ()>(())
                    }
                }

            })
        })
        .collect::<Vec<_>>();
    let work = join_all(work);
    core.run(work).unwrap();
}

// fn kill_all(drivers_session: &Vec<Result<Value, hyper::Error>>, drivers: &Vec<ChromeDriver>) {
//    let mut core = tokio_core::reactor::Core::new().unwrap();
//    let handle = core.handle();
//    let client = &Client::new(&handle);
//    let work =
//        drivers.iter().flat_map(move |driver| {
//            println!("driver : {:?}", driver);
//            drivers_session.iter().flat_map(move |ref session| {
//                let session = session.unwrap();
//                session["value"].as_array().unwrap().iter().map(move |browser| {
//                    let url = format!("http://{}/session/{}", driver.ip, browser["id"].as_str()\
// .unwrap()).as_str().parse::<hyper::Uri>().unwrap();
//                    println!("url: {:?}", url);
//                    let request = ClientRequest::new(Method::Delete, url);
//                    client.request(request).and_then(|res| {
//                        println!("response: {:?}", res);
//                        let mut body = Vec::new();
//                        res.body().fold(body, |mut body, chunk| {
//                            body.extend_from_slice(&chunk);
//                            Ok::<Vec<u8>, hyper::Error>(body)
//                        }).map(|full_body| {
//                            println!("response recevied");
//                            let value : Value = serde_json::from_slice(&full_body).unwrap();
//                            value
//                        })
//                    })
//                })
//        })
//    }).collect::<Vec<_>>();
//    let work = join_all(work);
//    let result = core.run(work).unwrap();
// }

impl Service for Test {
    type Request = Request;
    type Response = Response;
    type Error = hyper::Error;
    type Future = FutureResult<Response, hyper::Error>;

    fn call(&self, req: Request) -> Self::Future {
        let mut drivers = self.drivers.lock().unwrap();
        futures::future::ok(match (req.method(), req.path()) {
            (&Get, "/url") => {
                let mut values = update_drivers_information(&mut drivers);
                Response::new()
                    .with_header(ContentLength(INDEX.len() as u64))
                    .with_body(INDEX)
            }
            //            (&Get, "/killall") => {
            //                let drivers_session = update_drivers_information(&mut drivers);
            //                println!("Drivers session: {:?}", drivers_session);
            // /                kill_all(&drivers_session, &drivers);
            //                Response::new()
            //                    .with_header(ContentLength(INDEX.len() as u64))
            //                    .with_body(INDEX)
            //            },
            _ => Response::new().with_status(StatusCode::NotFound),
        })
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
        .get_matches();
    let file = std::fs::File::open(matches.value_of("config").unwrap()).unwrap();
    let mut buf_reader = BufReader::new(file);
    let chrome_drivers: Arc<Mutex<Vec<ChromeDriver>>> =
        Arc::new(Mutex::new(serde_json::from_reader(buf_reader).unwrap()));
    let addr = "127.0.0.1:1337".parse().unwrap();
    let server = Http::new()
        .bind(&addr, move || Ok(Test { drivers: chrome_drivers.clone() }))
        .unwrap();
    println!("Listening on http://{} with 1 thread.",
             server.local_addr().unwrap());
    server.run().unwrap();
}
