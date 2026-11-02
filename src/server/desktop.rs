//! Opening a page of the web UI, or a folder, on the desktop of the user who runs the server.

use super::Logged;
use crate::log;

/// Whether the desktop opened `target`: a page in the browser, or a folder in the file manager.
pub(super) fn open_on_desktop(target: String) -> bool {
    open::that_detached(target).map_err(|e| log::event(None, &Logged::NotOpened { error: e.to_string() })).is_ok()
}
