//! The icon and menu that the `tray-icon` crate draws on macOS and Windows, from the thread that runs the
//! system's event loop.

use std::cell::RefCell;

use tray_icon::{
    Icon, TrayIcon, TrayIconBuilder,
    menu::{CheckMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem},
};

use super::{
    Row, Switch,
    logo::{Rgb, SIDE, pixels},
    menu::{Click, Item, items, level, tooltip},
};
use crate::log::Level;

thread_local! {
    static SHOWN: RefCell<Option<TrayIcon>> = const { RefCell::new(None) };
}

/// The id of the item that stops the server. A switch's id is its name; every other item's id is the page
/// it opens.
const QUIT: &str = "quit";

pub(super) fn open_clicked() {
    MenuEvent::set_event_handler(Some(|event: MenuEvent| {
        match event.id.0.as_str() {
            QUIT => Click::Quit,
            id => id.parse::<Switch>().map_or_else(|_| Click::Open(id.to_owned()), Click::Toggle),
        }
        .run();
    }));
}

fn id(click: &Click) -> &str {
    match click {
        Click::Open(url) => url,
        Click::Toggle(switch) => (*switch).into(),
        Click::Quit => QUIT,
    }
}

/// `paint` draws the icon for the level, as each system wants it.
pub(super) fn show(
    login: &str,
    rows: &[Row],
    switches: &[(Switch, Option<bool>)],
    paint: impl FnOnce(&TrayIcon, Level),
) {
    let level = level(rows);
    let menu = Menu::new();
    for item in items(login, rows, switches) {
        _ = match item {
            Item::Entry { label, click: Some(click) } => menu.append(&MenuItem::with_id(id(&click), label, true, None)),
            Item::Entry { label, click: None } => menu.append(&MenuItem::new(label, false, None)),
            Item::Check { label, enabled, checked, click } => {
                menu.append(&CheckMenuItem::with_id(id(&click), label, enabled, checked, None))
            }
            Item::Separator => menu.append(&PredefinedMenuItem::separator()),
        };
    }
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
