//! The web UI: repository views (listing, browsing, downloading, restoring, re-creating), live pages,
//! monitoring and logins.

use std::{
    io::{Read, Write},
    thread,
    time::{Duration, Instant},
};

use rusticlone::APP_NAME;
use serde_json::json;

use crate::{
    common::{Sandbox, as_root, docs_with_remote, global_env, json_has, wait_for},
    server::card,
};

mod browse;
mod live;
mod pages;
mod restore;
mod runs;
mod setup;

/// Whether a flash message of this level is in `html`: `success` or `failure`.
fn flash(html: &str, level: &str) -> bool {
    html.contains(&format!(r#"class="flash {level}""#))
}

/// Answers every request with `204 No Content`, and counts them.
fn receiver() -> (String, std::sync::mpsc::Receiver<()>) {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("json://{}", listener.local_addr().unwrap());
    let (tx, rx) = std::sync::mpsc::channel();
    thread::spawn(move || {
        for mut socket in listener.incoming().flatten() {
            let mut request = [0; 8192];
            _ = socket.set_read_timeout(Some(Duration::from_secs(5)));
            _ = socket.read(&mut request);
            _ = write!(socket, "HTTP/1.1 204 No Content\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
            _ = tx.send(());
        }
    });
    (url, rx)
}
