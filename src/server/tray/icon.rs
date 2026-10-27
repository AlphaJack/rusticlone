//! The icon and menu that the `tray-icon` crate draws on macOS and Windows, from the thread that runs the
//! system's event loop.

use std::cell::RefCell;

use tray_icon::{
    Icon, TrayIcon, TrayIconBuilder,
    menu::{CheckMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem},
};

use super::{Rgb, Row, SIDE, Switch, level, open_label, pixels, quit, quit_label, tooltip, version_label};
use crate::log::Level;

thread_local! {
    static SHOWN: RefCell<Option<TrayIcon>> = const { RefCell::new(None) };
}

/// The id of the item that stops the server. A switch's id is its label; every other item's id is the page
/// it opens.
const QUIT: &str = "quit";

pub(super) fn open_clicked() {
    MenuEvent::set_event_handler(Some(|event: MenuEvent| match event.id.0.as_str() {
        QUIT => quit(),
        id => match id.parse::<Switch>() {
            Ok(switch) => switch.toggle(),
            Err(_) => _ = crate::server::open_on_desktop(id.to_owned()),
        },
    }));
}

/// `paint` draws the icon for the level, as each system wants it.
pub(super) fn show(
    login: String,
    rows: &[Row],
    switches: &[(Switch, Option<bool>)],
    paint: impl FnOnce(&TrayIcon, Level),
) {
    let level = level(rows);
    let menu = Menu::new();
    _ = menu.append(&MenuItem::new(version_label(), false, None));
    _ = menu.append(&MenuItem::with_id(login, open_label(), true, None));
    _ = menu.append(&PredefinedMenuItem::separator());
    for row in rows {
        let item = match &row.url {
            Some(url) => MenuItem::with_id(url, row.label.clone(), true, None),
            None => MenuItem::new(row.label.clone(), false, None),
        };
        _ = menu.append(&item);
    }
    _ = menu.append(&PredefinedMenuItem::separator());
    for (switch, state) in switches {
        let label = switch.to_string();
        _ = menu.append(&CheckMenuItem::with_id(&label, &label, state.is_some(), *state == Some(true), None));
    }
    _ = menu.append(&PredefinedMenuItem::separator());
    _ = menu.append(&MenuItem::with_id(QUIT, quit_label(), true, None));
    SHOWN.with_borrow_mut(|shown| {
        match shown {
            // Updated in place, so that it keeps its place in the menu bar.
            Some(tray) => {
                _ = tray.set_tooltip(Some(tooltip(level)));
                tray.set_menu(Some(Box::new(menu)));
            }
            None => *shown = TrayIconBuilder::new().with_tooltip(tooltip(level)).with_menu(Box::new(menu)).build().ok(),
        }
        if let Some(tray) = shown {
            paint(tray, level);
        }
    });
}

/// `pixels` as an icon: `None` only for pixels of the wrong size, which `pixels` does not make.
pub(super) fn image(level: Level, ink: Rgb, cut_only: bool) -> Option<Icon> {
    Icon::from_rgba(pixels(level, ink, cut_only), SIDE, SIDE).ok()
}
