//! Opening a page of the web UI, or a folder, on the desktop of the user who runs the server.
//!
//! Each system's way to open them is in a file of its own, used as `imp`.

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
