//! A D-Bus of a test's own, with systemd's manager, logind and the desktop's services, which only record the calls
//! they get: the install reaches no real service manager, and nothing opens on the desktop.

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

    fn start_unit(&self, name: String, _mode: String) -> zbus::OwnedObjectPath {
        self.job("StartUnit", &name)
    }

    fn restart_unit(&self, name: String, _mode: String) -> zbus::OwnedObjectPath {
        self.job("RestartUnit", &name)
    }

    fn stop_unit(&self, name: String, _mode: String) -> zbus::OwnedObjectPath {
        self.job("StopUnit", &name)
    }

    /// Unknown without a unit, as one that is not installed yet.
    fn get_unit_file_state(&self, file: String) -> zbus::fdo::Result<String> {
        self.unit.clone().ok_or(zbus::fdo::Error::FileNotFound(file))
    }
}

impl Systemd {
    fn job(&self, method: &str, name: &str) -> zbus::OwnedObjectPath {
        record(&self.log, &format!("{method} {name}"));
        zbus::OwnedObjectPath::try_from("/org/freedesktop/systemd1/job/1").unwrap()
    }
}

struct Logind(PathBuf);

#[zbus::interface(name = "org.freedesktop.login1.Manager")]
impl Logind {
    fn set_user_linger(&self, _uid: u32, enable: bool, _interactive: bool) {
        record(&self.0, &format!("SetUserLinger {enable}"));
    }
}

/// The desktop portal, which opens a page in the browser.
struct Portal(PathBuf);

#[zbus::interface(name = "org.freedesktop.portal.OpenURI")]
impl Portal {
    #[zbus(name = "OpenURI")]
    fn open_uri(
        &self,
        _parent: String,
        uri: String,
        _options: std::collections::HashMap<String, zbus::OwnedVariant>,
    ) -> zbus::OwnedObjectPath {
        record(&self.0, &format!("OpenURI {uri}"));
        zbus::OwnedObjectPath::try_from("/org/freedesktop/portal/desktop/request/1").unwrap()
    }
}

/// The file manager, which opens a folder.
struct FileManager(PathBuf);

#[zbus::interface(name = "org.freedesktop.FileManager1")]
impl FileManager {
    fn show_folders(&self, uris: Vec<String>, _startup: String) {
        record(&self.0, &format!("ShowFolders {}", uris.join(" ")));
    }
}

/// The notification server, which records the title and the icon it was given.
struct Notifications(PathBuf);

#[zbus::interface(name = "org.freedesktop.Notifications")]
impl Notifications {
    #[expect(clippy::too_many_arguments, reason = "the arguments of the specification's `Notify`")]
    fn notify(
        &self,
        _app: String,
        _replaces: u32,
        icon: String,
        summary: String,
        _body: String,
        _actions: Vec<String>,
        _hints: std::collections::HashMap<String, zbus::OwnedVariant>,
        _timeout: i32,
    ) -> u32 {
        record(&self.0, &format!("Notify {summary} {icon}"));
        1
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
                    .name("org.freedesktop.systemd1")
                    .name("org.freedesktop.login1")
                    .name("org.freedesktop.portal.Desktop")
                    .name("org.freedesktop.FileManager1")
                    .name("org.freedesktop.Notifications")
                    .serve_at("/org/freedesktop/systemd1", Systemd { log: log.clone(), unit })
                    .serve_at("/org/freedesktop/login1", Logind(log.clone()))
                    .serve_at("/org/freedesktop/portal/desktop", Portal(log.clone()))
                    .serve_at("/org/freedesktop/FileManager1", FileManager(log.clone()))
                    .serve_at("/org/freedesktop/Notifications", Notifications(log))
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
