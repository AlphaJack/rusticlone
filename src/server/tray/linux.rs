//! The panel icon, as a `StatusNotifierItem` over the session's D-Bus: KDE, Xfce, Cinnamon and Ubuntu
//! show it; plain GNOME only with the `AppIndicator` extension.

use std::sync::Arc;

use anyhow::Result;
use futures_util::StreamExt;
use ksni::{
    Handle, Icon, MenuItem, ToolTip, TrayMethods,
    menu::{CheckmarkItem, StandardItem},
};
use zbus::{Connection, MatchRule, MessageStream, message::Type, zvariant::OwnedValue};

use super::{
    Row, Switch,
    logo::{SIDE, ink, pixels},
    menu::{Click, Item, items, level, tooltip},
};
use crate::{APP_NAME, NAME};

/// The desktop portal's settings, where the color scheme is: 1 prefers dark, 2 light, 0 says nothing.
const PORTAL: &str = "org.freedesktop.portal.Desktop";
const PORTAL_PATH: &str = "/org/freedesktop/portal/desktop";
const SETTINGS: &str = "org.freedesktop.portal.Settings";
const APPEARANCE: &str = "org.freedesktop.appearance";
const COLOR_SCHEME: &str = "color-scheme";
const LIGHT: u32 = 2;

/// The icon, once the session's D-Bus accepts it.
pub(in crate::server) struct Tray(Option<Handle<Shown>>);

/// Runs `serve` with the icon when its user runs it; as a system service or under the tests, alone.
pub(super) fn beside(attended: bool, serve: impl FnOnce(Option<Tray>) -> Result<()>) -> Result<()> {
    serve(attended.then_some(Tray(None)))
}

/// What the icon shows: the web UI logged in, the collections and the switches, on a dark panel or not.
struct Shown {
    login: String,
    rows: Vec<Row>,
    switches: Vec<(Switch, Option<bool>)>,
    dark: bool,
    /// Does what an item says, when it is clicked.
    run: Arc<dyn Fn(Click) + Send + Sync>,
}

impl Tray {
    /// Without a session bus, the icon waits for the next change to try again.
    #[cfg_attr(test, mutants::skip)] // Puts the icon on the session's D-Bus.
    pub(super) async fn show(&mut self, login: &str, rows: &[Row], switches: &[(Switch, Option<bool>)]) {
        let bus = Connection::session().await.ok();
        let (login, rows, switches) = (login.to_owned(), rows.to_vec(), switches.to_vec());
        let dark = dark(color_scheme(bus.as_ref()).await);
        let shown = Shown { login, rows, switches, dark, run: Arc::new(Click::run) };
        match &self.0 {
            Some(handle) => _ = handle.update(|s| *s = shown).await,
            // The panel may start after the server, at login: the icon shows once it does.
            None => {
                self.0 = shown.assume_sni_available(true).spawn().await.ok();
                if let (Some(_), Some(bus)) = (&self.0, bus) {
                    tokio::spawn(watch_theme(bus));
                }
            }
        }
    }
}

/// The desktop portal's color scheme, if the session's D-Bus has one.
#[cfg_attr(test, mutants::skip)] // Asks the session's D-Bus.
async fn color_scheme(bus: Option<&Connection>) -> Option<u32> {
    let reply = bus?
        .call_method(Some(PORTAL), PORTAL_PATH, Some(SETTINGS), "ReadOne", &(APPEARANCE, COLOR_SCHEME))
        .await
        .ok()?;
    u32::try_from(reply.body().deserialize::<OwnedValue>().ok()?).ok()
}

/// Whether the panel is dark: GNOME's bar is whatever the color scheme, and most panels are when it says nothing.
fn dark(scheme: Option<u32>) -> bool {
    scheme != Some(LIGHT)
}

/// Has the icon drawn again whenever the color scheme changes.
#[cfg_attr(test, mutants::skip)] // Waits for the session's D-Bus.
async fn watch_theme(bus: Connection) {
    let changes = async {
        let rule = MatchRule::builder()
            .msg_type(Type::Signal)
            .interface(SETTINGS)?
            .member("SettingChanged")?
            .arg(0, APPEARANCE)?
            .build();
        MessageStream::for_match_rule(rule, &bus, None).await
    };
    let Ok(mut changes) = changes.await else { return };
    while changes.next().await.is_some() {
        super::REDRAW.notify_one();
    }
}

impl ksni::Tray for Shown {
    fn id(&self) -> String {
        NAME.into()
    }

    fn title(&self) -> String {
        APP_NAME.into()
    }

    /// A click opens the web UI; the menu is on the other button.
    fn activate(&mut self, _: i32, _: i32) {
        (self.run)(Click::Open(self.login.clone()));
    }

    fn icon_pixmap(&self) -> Vec<Icon> {
        let mut data = pixels(level(&self.rows), ink(self.dark), false);
        // ARGB, as D-Bus carries it.
        for pixel in data.as_chunks_mut::<{ size_of::<u32>() }>().0 {
            pixel.rotate_right(1);
        }
        let side = |n: u32| i32::try_from(n).unwrap_or_default();
        vec![Icon { width: side(SIDE), height: side(SIDE), data }]
    }

    fn tool_tip(&self) -> ToolTip {
        ToolTip { title: tooltip(level(&self.rows)), ..ToolTip::default() }
    }

    fn menu(&self) -> Vec<MenuItem<Self>> {
        let on = |click: Click| Box::new(move |shown: &mut Self| (shown.run)(click.clone()));
        let item = |item| match item {
            Item::Entry { label, click: Some(click) } => {
                StandardItem { label, activate: on(click), ..StandardItem::default() }.into()
            }
            Item::Entry { label, click: None } => {
                StandardItem { label, enabled: false, ..StandardItem::default() }.into()
            }
            Item::Check { label, enabled, checked, click } => {
                CheckmarkItem { label, enabled, checked, activate: on(click), ..CheckmarkItem::default() }.into()
            }
            Item::Separator => MenuItem::Separator,
        };
        items(&self.login, &self.rows, &self.switches).into_iter().map(item).collect()
    }
}

/// systemd tells its services where to report readiness.
#[expect(clippy::disallowed_methods, reason = "systemd's variable, not a setting")]
#[cfg_attr(test, mutants::skip)] // Reads the process's environment, which every test shares.
pub(in crate::server) fn managed() -> bool {
    std::env::var_os("NOTIFY_SOCKET").is_some()
}
