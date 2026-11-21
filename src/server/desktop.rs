//! Opening a page of the web UI, or a folder, on the desktop of the user who runs the server.
//!
//! Each system's way to open them, and to select a file in the file manager, is in a file of its own, used as
//! `imp`.

#[cfg(not(feature = "desktop"))]
mod headless;
#[cfg(not(feature = "desktop"))]
use headless as imp;

#[cfg(all(feature = "desktop", target_os = "linux"))]
mod linux;
#[cfg(all(feature = "desktop", target_os = "linux"))]
use linux as imp;

#[cfg(all(feature = "desktop", target_os = "macos"))]
mod macos;
#[cfg(all(feature = "desktop", target_os = "macos"))]
use macos as imp;

#[cfg(all(feature = "desktop", windows))]
mod windows;
#[cfg(all(feature = "desktop", windows))]
use windows as imp;

use super::Logged;
use crate::log;

/// Whether the desktop opened `target`: a page in the browser, or a folder in the file manager.
pub(super) fn open_on_desktop(target: &str) -> bool {
    imp::open(target, is_page(target))
        .map_err(|e| log::event(None, &Logged::NotOpened { error: e.to_string() }))
        .is_ok()
}

/// Whether `target` is a page rather than a folder.
fn is_page(target: &str) -> bool {
    target.starts_with("http://") || target.starts_with("https://")
}

/// Whether the file manager showed `file` selected in its folder. Where it does not, as on a Linux desktop without
/// the `FileManager1` service, it opens the folder.
pub(crate) async fn reveal_on_desktop(file: &std::path::Path) -> bool {
    imp::reveal(file).await || file.parent().is_some_and(|dir| open_on_desktop(&dir.display().to_string()))
}

#[cfg(test)]
mod tests {
    /// A page is a web address. Anything else is a folder.
    #[test]
    fn a_page_is_a_web_address() {
        assert!(super::is_page("http://127.0.0.1:32110/") && super::is_page("https://nas.local/"));
        assert!(!super::is_page("/home/me/docs") && !super::is_page("file:///home/me"));
    }
}
