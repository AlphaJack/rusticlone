//! The panel icon, as a `StatusNotifierItem` over the session's D-Bus: KDE, Xfce, Cinnamon and Ubuntu
//! show it; plain GNOME only with the `AppIndicator` extension.

use anyhow::Result;
use futures_util::StreamExt;
use ksni::{
    Handle, Icon, MenuItem, ToolTip, TrayMethods,
    menu::{CheckmarkItem, StandardItem},
};
use zbus::{Connection, MatchRule, MessageStream, message::Type, zvariant::OwnedValue};

use super::{Row, SIDE, Switch, ink, level, open_label, pixels, quit, quit_label, tooltip, version_label};
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
pub(in crate::server) fn beside(open: bool, serve: impl FnOnce(Option<Tray>) -> Result<()>) -> Result<()> {
    serve(super::attended(open).then_some(Tray(None)))
}

/// What the icon shows: the web UI logged in, the collections and the switches, on a dark panel or not.
struct Shown {
    login: String,
    rows: Vec<Row>,
    switches: Vec<(Switch, Option<bool>)>,
    dark: bool,
}

impl Tray {
    /// Without a session bus, the icon waits for the next change to try again.
    pub(super) async fn show(&mut self, login: &str, rows: &[Row], switches: &[(Switch, Option<bool>)]) {
        let bus = Connection::session().await.ok();
        let (login, rows, switches) = (login.to_owned(), rows.to_vec(), switches.to_vec());
        let shown = Shown { login, rows, switches, dark: dark(bus.as_ref()).await };
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

/// Whether the panel is dark: GNOME's bar is whatever the color scheme, and most panels are when it says nothing.
async fn dark(bus: Option<&Connection>) -> bool {
    let Some(bus) = bus else { return true };
    let scheme = async {
        let reply =
            bus.call_method(Some(PORTAL), PORTAL_PATH, Some(SETTINGS), "ReadOne", &(APPEARANCE, COLOR_SCHEME)).await?;
        Ok::<_, zbus::Error>(u32::try_from(reply.body().deserialize::<OwnedValue>()?)?)
    };
    scheme.await.ok() != Some(LIGHT)
}

/// Has the icon drawn again whenever the color scheme changes.
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
        _ = crate::server::open_on_desktop(self.login.clone());
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
        let open = |url: String| Box::new(move |_: &mut Self| _ = crate::server::open_on_desktop(url.clone()));
        let item = |label: String, url: Option<&String>| {
            let mut item = StandardItem { label, enabled: url.is_some(), ..StandardItem::default() };
            if let Some(url) = url {
                item.activate = open(url.clone());
            }
            item.into()
        };
        let quit =
            StandardItem { label: quit_label(), activate: Box::new(|_: &mut Self| quit()), ..StandardItem::default() };
        let switches = self.switches.iter().map(|&(switch, state)| {
            let activate = Box::new(move |_: &mut Self| switch.toggle());
            let (enabled, checked) = (state.is_some(), state == Some(true));
            CheckmarkItem { label: switch.label(), enabled, checked, activate, ..CheckmarkItem::default() }.into()
        });
        [item(version_label(), None), item(open_label(), Some(&self.login)), MenuItem::Separator]
            .into_iter()
            .chain(self.rows.iter().map(|r| item(r.label.clone(), r.url.as_ref())))
            .chain([MenuItem::Separator])
            .chain(switches)
            .chain([MenuItem::Separator, quit.into()])
            .collect()
    }
}
