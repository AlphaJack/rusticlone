use super::{Path, PathBuf, Removed, Result, unix::path_var};
use crate::{NAME, sys};

const ICON: &[u8] = include_bytes!("../../../assets/images/logo.svg");

/// The desktop entry and its icon, for a user: root has no desktop.
fn desktop_files() -> Result<Option<[PathBuf; 2]>> {
    if sys::is_root() {
        return Ok(None);
    }
    let data = etcetera::choose_base_strategy().map(|b| etcetera::BaseStrategy::data_dir(&b))?;
    let entry = data.join("applications").join(format!("{NAME}.desktop"));
    Ok(Some([entry, data.join("icons").join("hicolor").join("scalable").join("apps").join(format!("{NAME}.svg"))]))
}

/// The bus where systemd manages the server: the system's for root, the user's otherwise.
async fn bus() -> zbus::Result<zbus::Connection> {
    if sys::is_root() { zbus::Connection::system().await } else { zbus::Connection::session().await }
}

/// Calls systemd's manager like `systemctl` would, and reads the answer as `T`.
fn manager<T>(method: &str, args: &(impl serde::Serialize + zbus::zvariant::DynamicType)) -> Result<T>
where
    T: for<'d> zbus::zvariant::DynamicDeserialize<'d>,
{
    let call = async {
        let systemd = "org.freedesktop.systemd1";
        let reply = bus()
            .await?
            .call_method(
                Some(systemd),
                "/org/freedesktop/systemd1",
                Some("org.freedesktop.systemd1.Manager"),
                method,
                args,
            )
            .await?;
        reply.body().deserialize::<T>()
    };
    Ok(tokio::runtime::Builder::new_current_thread().enable_all().build()?.block_on(call)?)
}

/// Asks systemd to reread the units.
fn reload() -> Result<()> {
    manager::<()>("Reload", &())
}

/// Starts the service at login, or at boot for root.
fn enable(service: &str) -> Result<()> {
    manager::<(bool, Vec<(String, String, String)>)>("EnableUnitFiles", &(&[service][..], false, true)).map(drop)
}

fn disable(service: &str) -> Result<()> {
    manager::<Vec<(String, String, String)>>("DisableUnitFiles", &(&[service][..], false)).map(drop)
}

/// Asks systemd to start, restart or stop `service`, without waiting for it.
fn job(method: &str, service: &str) -> Result<()> {
    manager::<zbus::zvariant::OwnedObjectPath>(method, &(service, "replace")).map(drop)
}

/// The unit file's state, as `systemctl is-enabled` prints it, or `None` when systemd has no such unit.
fn state(service: &str) -> Option<String> {
    manager::<String>("GetUnitFileState", &(service,)).ok()
}

/// Keeps the user's services running without a login, as `loginctl enable-linger` does.
#[expect(unsafe_code, reason = "getuid has no preconditions")]
fn linger() -> Result<()> {
    // SAFETY: no preconditions.
    let uid = unsafe { libc::getuid() };
    let call = async {
        let bus = zbus::Connection::system().await?;
        let login = "org.freedesktop.login1";
        let args = (uid, true, false);
        bus.call_method(
            Some(login),
            "/org/freedesktop/login1",
            Some("org.freedesktop.login1.Manager"),
            "SetUserLinger",
            &args,
        )
        .await
    };
    tokio::runtime::Builder::new_current_thread().enable_all().build()?.block_on(call)?;
    Ok(())
}

fn dir() -> Result<PathBuf> {
    Ok(if sys::is_root() {
        "/etc/systemd/system".into()
    } else {
        etcetera::choose_base_strategy().map(|b| etcetera::BaseStrategy::config_dir(&b))?.join("systemd").join("user")
    })
}

/// A user without a graphical session, as on a server over SSH, whose services systemd would stop at logout.
#[cfg_attr(test, mutants::skip)] // Reads this session's environment, which every test shares.
pub(super) fn headless() -> bool {
    !sys::is_root() && ["DISPLAY", "WAYLAND_DISPLAY"].iter().all(|v| std::env::var_os(v).is_none())
}

/// Writes the unit unless a package installed it, and a drop-in with this executable and `PATH`. Restarted,
/// so that a running server takes them. Headless, systemd keeps the user's services running without a login.
pub(super) fn install(_listen: &str, exe: &Path, serve: &[String]) -> Result<PathBuf> {
    if headless() {
        linger()?;
    }
    let (dir, service) = (dir()?, format!("{NAME}.service"));
    let unit = dir.join(&service);
    if !unit.exists() && !known() {
        std::fs::create_dir_all(&dir)?;
        std::fs::write(&unit, super::packaging::unit())?;
    }
    let drop_in = dir.join(format!("{service}.d"));
    std::fs::create_dir_all(&drop_in)?;
    // `%` starts a specifier.
    let exe = exe.display().to_string();
    let (escaped, path) = (exe.replace('%', "%%"), path_var().replace('%', "%%"));
    let serve = serve.join(" ");
    let conf = format!("[Service]\nExecStart=\nExecStart=\"{escaped}\" {serve}\nEnvironment=\"PATH={path}\"\n");
    std::fs::write(drop_in.join("install.conf"), conf)?;
    reload()?;
    enable(&service)?;
    job("RestartUnit", &service)?;
    if let Some([entry, icon]) = desktop_files()? {
        for (path, content) in [(&entry, super::packaging::desktop_entry(&exe).into_bytes()), (&icon, ICON.to_vec())] {
            std::fs::create_dir_all(path.parent().unwrap_or(path))?;
            std::fs::write(path, content)?;
        }
    }
    Ok(drop_in)
}

/// Whether systemd has the unit, from `install` or a package: a packaged one serves without `install`.
fn known() -> bool {
    state(&format!("{NAME}.service")).is_some()
}

/// Whether the server starts at login, by the same rule as `systemctl is-enabled`.
#[cfg_attr(test, mutants::skip)] // Asks systemd.
pub(super) fn at_login() -> Option<bool> {
    /// The states in which `systemctl is-enabled` succeeds.
    const ENABLED: [&str; 7] = ["enabled", "enabled-runtime", "static", "alias", "indirect", "generated", "transient"];
    state(&format!("{NAME}.service")).map(|s| ENABLED.contains(&s.as_str()))
}

#[cfg_attr(test, mutants::skip)] // Changes systemd's settings.
pub(super) fn set_at_login(on: bool) -> Result<()> {
    let service = format!("{NAME}.service");
    if on { enable(&service) } else { disable(&service) }
}

/// Asks systemd to start the server unless it runs; false without its unit.
#[cfg_attr(test, mutants::skip)] // Asks systemd to start the server.
pub(super) fn start() -> Result<bool> {
    if !known() {
        return Ok(false);
    }
    job("StartUnit", &format!("{NAME}.service"))?;
    Ok(true)
}

/// `None` if not installed.
pub(super) fn uninstall() -> Result<Option<Removed>> {
    let (dir, service) = (dir()?, format!("{NAME}.service"));
    let drop_in = dir.join(format!("{service}.d"));
    if !drop_in.join("install.conf").exists() {
        return Ok(None);
    }
    disable(&service)?;
    job("StopUnit", &service)?;
    std::fs::remove_file(drop_in.join("install.conf"))?;
    _ = std::fs::remove_dir(&drop_in);
    // A packaged unit stays.
    if std::fs::read_to_string(dir.join(&service)).is_ok_and(|u| u == super::packaging::unit()) {
        std::fs::remove_file(dir.join(&service))?;
    }
    reload()?;
    for file in desktop_files()?.into_iter().flatten() {
        _ = std::fs::remove_file(file);
    }
    Ok(Some(Removed { file: drop_in.display().to_string(), running: false }))
}

/// macOS's alone.
#[cfg(feature = "self-update")]
#[expect(clippy::unnecessary_wraps, reason = "macOS's copies the executable, which may fail")]
pub(super) fn refresh_app() -> Result<Option<String>> {
    Ok(None)
}

/// macOS's alone.
#[cfg(feature = "self-update")]
#[expect(clippy::unnecessary_wraps, reason = "macOS's rewrites the app, which may fail")]
pub(super) fn rewrite_app(_: &str) -> Result<()> {
    Ok(())
}

/// macOS's alone: Login Items.
#[cfg_attr(test, mutants::skip)] // macOS's alone.
pub(super) fn blocked() -> bool {
    false
}

/// macOS's alone: an app opened from Finder.
pub(super) fn opened_as_app() -> bool {
    false
}

/// A service manager runs it.
#[expect(clippy::unnecessary_wraps, reason = "Windows's stops a running server, which may fail")]
pub(super) fn stop_running(_: &Path) -> Result<()> {
    Ok(())
}
