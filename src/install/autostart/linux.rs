use super::{
    Path, PathBuf, Removed, Result,
    unix::{path_var, run},
};
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

fn systemctl(args: &[&str]) -> Result<()> {
    let user = (!sys::is_root()).then_some("--user");
    run("systemctl", &user.into_iter().chain(args.iter().copied()).collect::<Vec<_>>())
}

fn dir() -> Result<PathBuf> {
    Ok(if sys::is_root() {
        "/etc/systemd/system".into()
    } else {
        etcetera::choose_base_strategy().map(|b| etcetera::BaseStrategy::config_dir(&b))?.join("systemd").join("user")
    })
}

/// A user without a graphical session, as on a server over SSH, whose services systemd would stop at logout.
pub(super) fn headless() -> bool {
    !sys::is_root() && ["DISPLAY", "WAYLAND_DISPLAY"].iter().all(|v| std::env::var_os(v).is_none())
}

/// Writes the unit unless a package installed it, and a drop-in with this executable and `PATH`. Restarted,
/// so that a running server takes them. Headless, systemd keeps the user's services running without a login.
pub(super) fn install(_listen: &str, exe: &Path, serve: &[String]) -> Result<PathBuf> {
    if headless() {
        run("loginctl", &["enable-linger"])?;
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
    systemctl(&["daemon-reload"])?;
    systemctl(&["enable", &service])?;
    systemctl(&["restart", &service])?;
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
    systemctl(&["cat", &format!("{NAME}.service")]).is_ok()
}

pub(super) fn at_login() -> Option<bool> {
    known().then(|| systemctl(&["is-enabled", "--quiet", &format!("{NAME}.service")]).is_ok())
}

pub(super) fn set_at_login(on: bool) -> Result<()> {
    systemctl(&[if on { "enable" } else { "disable" }, &format!("{NAME}.service")])
}

/// Asks systemd to start the server unless it runs; false without its unit.
pub(super) fn start() -> Result<bool> {
    if !known() {
        return Ok(false);
    }
    systemctl(&["start", &format!("{NAME}.service")])?;
    Ok(true)
}

/// `None` if not installed.
pub(super) fn uninstall() -> Result<Option<Removed>> {
    let (dir, service) = (dir()?, format!("{NAME}.service"));
    let drop_in = dir.join(format!("{service}.d"));
    if !drop_in.join("install.conf").exists() {
        return Ok(None);
    }
    systemctl(&["disable", "--now", &service])?;
    std::fs::remove_file(drop_in.join("install.conf"))?;
    _ = std::fs::remove_dir(&drop_in);
    // A packaged unit stays.
    if std::fs::read_to_string(dir.join(&service)).is_ok_and(|u| u == super::packaging::unit()) {
        std::fs::remove_file(dir.join(&service))?;
    }
    systemctl(&["daemon-reload"])?;
    for file in desktop_files()?.into_iter().flatten() {
        _ = std::fs::remove_file(file);
    }
    Ok(Some(Removed { file: drop_in.display().to_string(), running: false }))
}

/// macOS's alone.
pub(super) fn app_source(_: &Path) -> Option<PathBuf> {
    None
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
