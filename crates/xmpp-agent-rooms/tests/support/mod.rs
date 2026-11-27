//! An in-process fake rights service on the loopback interface, and helpers to point the client at it.

// Each integration test file uses a different part of this module.
#![allow(dead_code)]

use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use tiny_http::{Header, Response, Server};
use xmpp_agent_rooms::client::HttpTransport;
use xmpp_agent_rooms::config::{Capabilities, Config, Env};
use xmpp_agent_rooms::tools::Rooms;

/// The token every test sends.
pub const TOKEN: &str = "tok-7d3f9a1c5e";

/// One request the fake received.
#[derive(Debug, Clone)]
pub struct Seen {
    pub method: String,
    pub path: String,
    pub authorization: Option<String>,
    pub body: String,
}

/// What the fake answers.
#[derive(Debug, Clone)]
pub struct Reply {
    pub status: u16,
    pub body: String,
    pub location: Option<String>,
    pub delay: Duration,
}

impl Reply {
    pub fn new(status: u16, body: &str) -> Self {
        Self { status, body: body.to_owned(), location: None, delay: Duration::ZERO }
    }
}

/// A running fake. Dropping it stops the server.
pub struct Fake {
    pub url: String,
    seen: Arc<Mutex<Vec<Seen>>>,
    server: Arc<Server>,
    handle: Option<JoinHandle<()>>,
}

impl Fake {
    /// Start a fake that answers every request with `reply(&request)`.
    pub fn start(reply: impl Fn(&Seen) -> Reply + Send + 'static) -> Self {
        let server = Arc::new(Server::http("127.0.0.1:0").expect("bind loopback"));
        let url = format!("http://{}", server.server_addr().to_ip().expect("an IP address"));
        let seen = Arc::new(Mutex::new(Vec::new()));
        let (loop_server, loop_seen) = (Arc::clone(&server), Arc::clone(&seen));
        let handle = thread::spawn(move || {
            for mut request in loop_server.incoming_requests() {
                let mut body = String::new();
                request.as_reader().read_to_string(&mut body).expect("read body");
                let header = request.headers().iter().find(|h| h.field.equiv("Authorization")).map(|h| h.value.to_string());
                let record = Seen { method: request.method().to_string(), path: request.url().to_owned(), authorization: header, body };
                let answer = reply(&record);
                loop_seen.lock().expect("lock").push(record);
                thread::sleep(answer.delay);
                let mut response = Response::from_string(answer.body).with_status_code(answer.status);
                if let Some(location) = answer.location {
                    response = response.with_header(Header::from_bytes("Location", location).expect("header"));
                }
                // The client may have given up already (a timeout test); that is not an error here.
                let _ = request.respond(response);
            }
        });
        Self { url, seen, server, handle: Some(handle) }
    }

    /// A fake that always answers the same way.
    pub fn always(status: u16, body: &str) -> Self {
        let reply = Reply::new(status, body);
        Self::start(move |_| reply.clone())
    }

    /// The requests received so far.
    pub fn seen(&self) -> Vec<Seen> {
        self.seen.lock().expect("lock").clone()
    }

    /// The settings that point a client at this fake.
    pub fn env(&self) -> Env {
        Env { rights_url: Some(self.url.clone()), token: Some(TOKEN.to_owned()), capabilities: Some("rooms".to_owned()) }
    }

    /// Actions bound to this fake, with `rooms` granted.
    pub fn rooms(&self) -> Rooms<HttpTransport> {
        self.rooms_with(Capabilities::parse("rooms"), Duration::from_secs(5))
    }

    /// Actions bound to this fake with chosen capabilities and timeout.
    pub fn rooms_with(&self, capabilities: Capabilities, timeout: Duration) -> Rooms<HttpTransport> {
        let mut config = Config::load(&self.env(), None).expect("config");
        config.timeout = timeout;
        Rooms::new(HttpTransport::new(config), capabilities)
    }
}

impl Drop for Fake {
    fn drop(&mut self) {
        self.server.unblock();
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}
