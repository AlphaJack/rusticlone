//! The desktop, over the session's D-Bus: the desktop portal opens a page, the file manager a folder.
//! `xdg-open` steps in on desktops that offer neither, such as a bare window manager.

/// Opens a page in the default browser, or a folder in the file manager.
pub(super) fn open(target: &str, page: bool) -> std::io::Result<()> {
    over_dbus(target, page).or_else(|_| xdg_open(target))
}

/// Starts `xdg-open`, which finds a browser or a file manager on its own. It may wait for what it opens, so a
/// thread of its own collects it.
#[expect(clippy::disallowed_methods, reason = "the desktop's last resort to open a page, not rustic")]
fn xdg_open(target: &str) -> std::io::Result<()> {
    use std::process::Stdio;
    std::process::Command::new("xdg-open")
        .arg(target)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map(|mut child| drop(std::thread::spawn(move || child.wait())))
}

/// The desktop portal's `OpenURI` for a page, and the `FileManager1` service for a folder.
fn over_dbus(target: &str, page: bool) -> std::io::Result<()> {
    let call = async {
        let bus = zbus::Connection::session().await?;
        if page {
            let options = std::collections::BTreeMap::<&str, zbus::zvariant::Value<'_>>::new();
            let portal = "org.freedesktop.portal.Desktop";
            let args = ("", target, options);
            bus.call_method(
                Some(portal),
                "/org/freedesktop/portal/desktop",
                Some("org.freedesktop.portal.OpenURI"),
                "OpenURI",
                &args,
            )
            .await
        } else {
            let fm = "org.freedesktop.FileManager1";
            let folders = vec![format!("file://{}", encoded(target))];
            bus.call_method(Some(fm), "/org/freedesktop/FileManager1", Some(fm), "ShowFolders", &(folders, "")).await
        }
    };
    let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build()?;
    runtime.block_on(call).map(drop).map_err(std::io::Error::other)
}

/// `path` as a URI's path: each byte but a letter, a digit, `/` and `-._~` percent-encoded.
fn encoded(path: &str) -> String {
    let kept = |b: u8| b.is_ascii_alphanumeric() || b"/-._~".contains(&b);
    path.bytes().map(|b| if kept(b) { char::from(b).to_string() } else { format!("%{b:02X}") }).collect()
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_path_is_percent_encoded() {
        assert_eq!(super::encoded("/home/me/My Docs/ü"), "/home/me/My%20Docs/%C3%BC");
    }
}
