//! A D-Bus of a test's own, with systemd's manager and logind, which only record the calls they get: the
//! install reaches no real service manager.

#![expect(
    clippy::needless_pass_by_value,
    clippy::used_underscore_binding,
    clippy::disallowed_types,
    reason = "zbus's interface macro takes owned arguments, every one, and a map of its own"
)]

use std::{
    io::{BufRead, Write},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
};

use crate::common::Sandbox;

/// The bus, stopped once dropped.
pub(crate) struct FakeBus {
    pub address: String,
    daemon: Child,
}

impl Drop for FakeBus {
    fn drop(&mut self) {
        _ = self.daemon.kill();
        _ = self.daemon.wait();
    }
}

/// Appends `call` to `log`, a line per call.
fn record(log: &Path, call: &str) {
    let mut file = std::fs::OpenOptions::new().create(true).append(true).open(log).unwrap();
    writeln!(file, "{call}").unwrap();
}

/// Records to its log, and answers the unit's state as systemd would once it knows the unit.
struct Systemd {
    log: PathBuf,
    unit: Option<String>,
}

#[zbus::interface(name = "org.freedesktop.systemd1.Manager")]
impl Systemd {
    fn reload(&self) {
        record(&self.log, "Reload");
    }

    fn enable_unit_files(
        &self,
        files: Vec<String>,
        _runtime: bool,
        _force: bool,
    ) -> (bool, Vec<(String, String, String)>) {
        record(&self.log, &format!("EnableUnitFiles {}", files.join(" ")));
        (false, Vec::new())
    }

    fn disable_unit_files(&self, files: Vec<String>, _runtime: bool) -> Vec<(String, String, String)> {
        record(&self.log, &format!("DisableUnitFiles {}", files.join(" ")));
        Vec::new()
    }

    fn start_unit(&self, name: String, _mode: String) -> zbus::zvariant::OwnedObjectPath {
        self.job("StartUnit", &name)
    }

    fn restart_unit(&self, name: String, _mode: String) -> zbus::zvariant::OwnedObjectPath {
        self.job("RestartUnit", &name)
    }

    fn stop_unit(&self, name: String, _mode: String) -> zbus::zvariant::OwnedObjectPath {
        self.job("StopUnit", &name)
    }

    /// Unknown without a unit, as one that is not installed yet.
    fn get_unit_file_state(&self, file: String) -> zbus::fdo::Result<String> {
        self.unit.clone().ok_or(zbus::fdo::Error::FileNotFound(file))
    }
}

impl Systemd {
    fn job(&self, method: &str, name: &str) -> zbus::zvariant::OwnedObjectPath {
        record(&self.log, &format!("{method} {name}"));
        zbus::zvariant::OwnedObjectPath::try_from("/org/freedesktop/systemd1/job/1").unwrap()
    }
}

struct Logind(PathBuf);

#[zbus::interface(name = "org.freedesktop.login1.Manager")]
impl Logind {
    fn set_user_linger(&self, _uid: u32, enable: bool, _interactive: bool) {
        record(&self.0, &format!("SetUserLinger {enable}"));
    }
}

impl FakeBus {
    /// Starts the bus in the sandbox, with the fake services recording to `log`. systemd knows a unit in
    /// the `unit` state, as one a package installed, or none.
    pub(crate) fn start(sb: &Sandbox, log: &Path, unit: Option<&str>) -> Self {
        let mut daemon = Command::new("dbus-daemon")
            .args(["--session", "--nofork", "--print-address=1"])
            .arg(format!("--address=unix:path={}", sb.path("bus").display()))
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let mut address = String::new();
        std::io::BufReader::new(daemon.stdout.take().unwrap()).read_line(&mut address).unwrap();
        let address = address.trim().to_owned();
        let (ready, served) = std::sync::mpsc::channel();
        let (at, log, unit) = (address.clone(), log.to_owned(), unit.map(str::to_owned));
        std::thread::spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
            runtime.block_on(async {
                let _connection = zbus::connection::Builder::address(at.as_str())
                    .unwrap()
                    .name("org.freedesktop.systemd1")
                    .unwrap()
                    .name("org.freedesktop.login1")
                    .unwrap()
                    .serve_at("/org/freedesktop/systemd1", Systemd { log: log.clone(), unit })
                    .unwrap()
                    .serve_at("/org/freedesktop/login1", Logind(log))
                    .unwrap()
                    .build()
                    .await
                    .unwrap();
                ready.send(()).unwrap();
                std::future::pending::<()>().await;
            });
        });
        served.recv().unwrap();
        Self { address, daemon }
    }

    /// The variables that point the system's and the session's bus to this one.
    pub(crate) fn env(&self) -> [(&'static str, &str); 2] {
        [("DBUS_SESSION_BUS_ADDRESS", &self.address), ("DBUS_SYSTEM_BUS_ADDRESS", &self.address)]
    }
}
