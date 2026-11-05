//! What opens on the desktop, over the fake bus.

use super::*;

/// The profiles' folder opens in the file manager, for a browser on this computer. Without a file manager on the
/// desktop, nor `xdg-open`, the page says that it could not.
#[cfg(target_os = "linux")]
#[test]
fn the_profile_folder_opens_in_the_file_manager() {
    let sb = docs_with_remote("");
    let log = sb.path("desktop.log");
    let bus = crate::bus::FakeBus::start(&sb, &log, None);
    let open = |server: &Server| {
        let form = "Content-Type: application/x-www-form-urlencoded";
        let login = server.request("POST", "/login", &[form], &format!("token={}", server.token));
        let session = login.head.lines().find_map(|l| l.strip_prefix("set-cookie: ")).unwrap();
        let cookie = format!("Cookie: {}", session.split(';').next().unwrap());
        server.request("POST", "/setup/folder", &[form, &cookie], "").code
    };
    assert_eq!(open(&sb.server(&bus.env())), 204);
    let folder = format!("ShowFolders file://{}", sb.profile_dir().display());
    assert!(fs::read_to_string(&log).unwrap().contains(&folder), "{}", fs::read_to_string(&log).unwrap());
    drop(bus);
    let path = std::env::var_os("PATH").unwrap();
    let without = std::env::split_paths(&path).filter(|d| !d.join("xdg-open").exists());
    let without = std::env::join_paths(without).unwrap().into_string().unwrap();
    assert_eq!(open(&sb.server(&[("PATH", &without)])), 422);
}

/// `serve --open` beside a server that runs opens that one in the browser, logged in, and starts no other.
#[cfg(target_os = "linux")]
#[test]
fn serve_open_opens_the_running_server() {
    let sb = docs_with_remote("");
    let log = sb.path("desktop.log");
    let bus = crate::bus::FakeBus::start(&sb, &log, None);
    let server = sb.server(&[]);
    let listen = var("LISTEN");
    let env = [bus.env().as_slice(), &[(listen.as_str(), server.addr.as_str())]].concat();
    sb.run_with(&["serve", "--open"], &env).assert_exit(0);
    let opened = fs::read_to_string(&log).unwrap();
    assert!(opened.contains(&format!("OpenURI http://{}/", server.addr)), "{opened}");
}

/// A desktop notification shows the app's own icon, written where the notification server reads it.
#[cfg(target_os = "linux")]
#[test]
fn a_desktop_notification_shows_the_app_icon() {
    let sb = Sandbox::new();
    let log = sb.path("desktop.log");
    let bus = crate::bus::FakeBus::start(&sb, &log, None);
    sb.backup_profile("Docs", &[&["docs"]], &[], &global_env(&[("NOTIFY_URLS", "dbus://")]));
    sb.write("docs/a.txt", "a");
    sb.run(&["backup", "--all"]).assert_exit(0);
    sb.run_with(&["notify", "--all"], &bus.env()).assert_exit(0);
    let notified = fs::read_to_string(&log).unwrap();
    let icon = notified.lines().find_map(|l| l.strip_prefix("Notify ")?.rsplit(' ').next()).unwrap_or_default();
    // The app's PNG, not the bundled SVG that mercurio falls back to.
    let icon = std::path::Path::new(icon);
    let named = icon.file_name().is_some_and(|n| n.to_string_lossy().starts_with("icon-"));
    assert!(named && icon.extension().is_some_and(|e| e.eq_ignore_ascii_case("png")), "{notified}");
}
