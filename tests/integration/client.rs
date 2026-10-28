//! The server tested from outside: started in a sandbox, and queried over raw HTTP.

use std::{
    io::{BufRead, BufReader, Read, Write},
    net::TcpStream,
    process::Child,
    sync::Arc,
    thread,
};

use parking_lot::Mutex;
use rusticlone::Status;

use crate::common::{Sandbox, var};

/// A running server, stopped when dropped.
pub(crate) struct Server {
    child: Child,
    pub addr: String,
    pub token: String,
    /// Stdout and stderr so far.
    output: Arc<Mutex<String>>,
}

pub(crate) struct Response {
    pub code: u16,
    pub head: String,
    pub body: String,
}

impl Sandbox {
    /// Snapshot ids in a repository, oldest first. Empty while it cannot be listed, for example while a
    /// run changes it.
    pub(crate) fn snapshot_ids(&self, profile: &str) -> Vec<String> {
        let out = self.rustic(&["-P", profile, "snapshots", "--json"]);
        let Ok(groups) = serde_json::from_slice::<serde_json::Value>(&out.stdout) else { return Vec::new() };
        let mut snapshots: Vec<(String, String)> = groups
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|g| g["snapshots"].as_array().unwrap())
            .map(|s| (s["time"].as_str().unwrap().to_owned(), s["id"].as_str().unwrap().to_owned()))
            .collect();
        snapshots.sort();
        snapshots.into_iter().map(|(_, id)| id).collect()
    }

    /// The collection's runs waiting for their turn, in any process, including those left by an ended one.
    pub(crate) fn queued(&self, profile: &str) -> usize {
        self.store().runs(profile, usize::MAX).iter().filter(|r| r.status == Status::Queued).count()
    }

    pub(crate) fn server(&self, envs: &[(&str, &str)]) -> Server {
        self.server_with(&[], envs)
    }

    /// With these extra arguments to `server`.
    pub(crate) fn server_with(&self, args: &[&str], envs: &[(&str, &str)]) -> Server {
        let args: Vec<&str> = ["serve"].into_iter().chain(args.iter().copied()).collect();
        let listen = var("LISTEN");
        let mut child = self.spawn(&args, &[&[(listen.as_str(), "127.0.0.1:0")], envs].concat());
        let mut stdout = BufReader::new(child.stdout.take().unwrap());
        // The line that says where the web UI is, and the file of its token if generated.
        let (addr, printed) = loop {
            let mut line = String::new();
            assert!(stdout.read_line(&mut line).unwrap() > 0, "server exited");
            let event = serde_json::from_str::<serde_json::Value>(&line).unwrap_or_default()["event"].take();
            if let Some(url) = event["url"].as_str() {
                let addr = url.trim_start_matches("http://").trim_end_matches('/').to_owned();
                break (addr, event["file"].as_str().map(|f| std::fs::read_to_string(f).unwrap()));
            }
        };
        let stderr = BufReader::new(child.stderr.take().unwrap());
        let output = Arc::new(Mutex::new(String::new()));
        let collect = |lines: Box<dyn Iterator<Item = std::io::Result<String>> + Send>| {
            let output = output.clone();
            thread::spawn(move || lines.map_while(Result::ok).for_each(|l| *output.lock() += &format!("{l}\n")));
        };
        collect(Box::new(stdout.lines()));
        collect(Box::new(stderr.lines()));
        let given = envs.iter().find(|(k, _)| *k == var("ADMIN_TOKEN")).map(|(_, v)| (*v).to_owned());
        Server { child, addr, token: printed.or(given).unwrap_or_default(), output }
    }
}

impl Server {
    /// A raw HTTP/1.1 request with these extra header lines.
    pub(crate) fn request(&self, method: &str, path: &str, headers: &[&str], body: &str) -> Response {
        let mut stream = TcpStream::connect(&self.addr).unwrap();
        let headers: String = headers.iter().map(|h| format!("{h}\r\n")).collect();
        // In one write: a server that rejects the headers closes before reading a body sent apart, and the
        // unread body makes the system reset the connection, losing the response.
        let request = format!(
            "{method} {path} HTTP/1.1\r\nHost: {}\r\nConnection: close\r\nContent-Length: {}\r\n{headers}\r\n{body}",
            self.addr,
            body.len()
        );
        stream.write_all(request.as_bytes()).unwrap();
        let mut bytes = Vec::new();
        stream.read_to_end(&mut bytes).unwrap();
        // Downloads may be binary.
        let response = String::from_utf8_lossy(&bytes);
        let (head, body) = response.split_once("\r\n\r\n").unwrap_or((&response, ""));
        let code = head.split_whitespace().nth(1).unwrap().parse().unwrap();
        Response { code, head: head.to_owned(), body: body.to_owned() }
    }

    /// With the token, as a script would.
    pub(crate) fn get(&self, path: &str) -> Response {
        self.get_with(path, &[])
    }

    pub(crate) fn get_with(&self, path: &str, headers: &[&str]) -> Response {
        let auth = format!("Authorization: Bearer {}", self.token);
        self.request("GET", path, &[&[auth.as_str()], headers].concat(), "")
    }

    pub(crate) fn post(&self, path: &str) -> Response {
        self.post_form(path, "")
    }

    /// `form` is `application/x-www-form-urlencoded`, already encoded.
    pub(crate) fn post_form(&self, path: &str, form: &str) -> Response {
        let auth = format!("Authorization: Bearer {}", self.token);
        let headers = [auth.as_str(), "Content-Type: application/x-www-form-urlencoded"];
        self.request("POST", path, &headers, form)
    }

    pub(crate) fn delete(&self, path: &str) -> Response {
        self.request("DELETE", path, &[&format!("Authorization: Bearer {}", self.token)], "")
    }

    /// A page's event stream, opened as htmx does: `query` is `collection=Docs`, `v=...`.
    pub(crate) fn events(&self, query: &str, headers: &[&str]) -> Events {
        let mut stream = TcpStream::connect(&self.addr).unwrap();
        let headers: String = headers.iter().map(|h| format!("{h}\r\n")).collect();
        let auth = format!("Authorization: Bearer {}", self.token);
        write!(stream, "GET /events?{query} HTTP/1.1\r\nHost: x\r\n{auth}\r\n{headers}\r\n").unwrap();
        stream.set_read_timeout(Some(std::time::Duration::from_millis(200))).unwrap();
        Events { stream, seen: String::new() }
    }

    pub(crate) fn pid(&self) -> u32 {
        self.child.id()
    }

    pub(crate) fn running(&mut self) -> bool {
        self.child.try_wait().unwrap().is_none()
    }

    pub(crate) fn output(&self) -> String {
        self.output.lock().clone()
    }

    /// Whether the server logged an event with these fields.
    pub(crate) fn logged(&self, fields: &serde_json::Value) -> bool {
        crate::common::json_has(&self.output(), fields)
    }

    pub(crate) fn collection(&self, name: &str) -> serde_json::Value {
        let status: serde_json::Value = serde_json::from_str(&self.get("/status").body).unwrap();
        status["collections"].as_array().unwrap().iter().find(|j| j["primary"]["name"] == name).cloned().unwrap()
    }
}

/// An open event stream.
pub(crate) struct Events {
    stream: TcpStream,
    /// Everything received so far, including HTTP framing.
    pub seen: String,
}

impl Events {
    /// Reads until `done` returns true for what arrives from now on, for at most `secs`.
    #[track_caller]
    pub(crate) fn wait_for(&mut self, secs: u64, what: &str, done: impl Fn(&str) -> bool) -> String {
        let (from, deadline) = (self.seen.len(), std::time::Instant::now() + std::time::Duration::from_secs(secs));
        while !done(self.seen.get(from..).unwrap()) {
            assert!(std::time::Instant::now() < deadline, "no event with {what}: {}", self.seen.get(from..).unwrap());
            let mut buf = vec![0; 1 << 16];
            if let Ok(n) = self.stream.read(&mut buf) {
                self.seen += &String::from_utf8_lossy(&buf[..n]);
            }
        }
        self.seen.get(from..).unwrap().to_owned()
    }

    /// What arrives within `secs`.
    pub(crate) fn read_for(&mut self, secs: u64) -> String {
        let (from, deadline) = (self.seen.len(), std::time::Instant::now() + std::time::Duration::from_secs(secs));
        while std::time::Instant::now() < deadline {
            let mut buf = vec![0; 1 << 16];
            if let Ok(n) = self.stream.read(&mut buf) {
                self.seen += &String::from_utf8_lossy(&buf[..n]);
            }
        }
        self.seen.get(from..).unwrap().to_owned()
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        terminate(&mut self.child);
    }
}

/// Sends SIGTERM, as systemd does. The server stops its rustic calls before it exits.
pub(crate) fn terminate(child: &mut Child) {
    if !crate::common::signal(child.id(), "TERM") {
        _ = child.kill();
    }
    _ = child.wait();
}
