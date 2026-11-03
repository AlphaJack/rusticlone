//! Status change notifications, sent with Mercurio.

use mercurio::{Mercurio, Message, NotifyType};

use crate::{
    i18n::Fluent,
    log::{self, Level},
    settings,
};

/// What a notification did, as the server and the `notify` command log it. `error` fields hold the
/// error's text.
#[derive(Debug, serde::Serialize, Fluent)]
#[serde(tag = "kind", rename_all = "kebab-case")]
#[fluent(prefix = "notification")]
pub(crate) enum Logged {
    Notified { text: String },
    WouldNotify { text: String },
    NotRecorded { error: String },
    Undelivered { error: String },
    Warned { warning: String },
}

/// A notification that fails is a warning: the collection's level does not count it.
impl log::Message for Logged {
    fn level(&self) -> Level {
        match self {
            Self::Notified { .. } | Self::WouldNotify { .. } => Level::Success,
            Self::NotRecorded { .. } | Self::Undelivered { .. } | Self::Warned { .. } => Level::Warning,
        }
    }
}

/// A test before any collection exists.
#[cfg(feature = "server")]
#[derive(Fluent)]
#[fluent(prefix = "notification")]
enum Line {
    Test { name: &'static str, emoji: &'static str },
}

#[derive(Debug, thiserror::Error, Fluent)]
#[fluent(prefix = "notification")]
pub(crate) enum Error {
    /// Each invalid URL, its secrets masked, and why.
    InvalidUrls { urls: String },
    #[fluent(key = settings::NOTIFY.key)]
    NoUrls,
    /// Each failed target and why.
    #[fluent(transparent)]
    Failed(String),
}

/// Sends a test to `urls`, such as the ones a new collection would use.
#[cfg(feature = "server")]
pub(crate) fn test(urls: &str) -> Result<(), Error> {
    check(urls)?;
    deliver(
        Mercurio::new(),
        &mut logged(None),
        urls,
        Level::Success,
        &Line::Test { name: crate::APP_NAME, emoji: Level::Success.emoji() }.to_string(),
    )
}

/// Refuses `urls` if mercurio does not read one of them.
#[cfg(feature = "server")]
pub(crate) fn check(urls: &str) -> Result<(), Error> {
    add(&mut Mercurio::new(), urls)
}

/// Adds `urls` to `client`, refusing them all if one does not parse.
fn add(client: &mut Mercurio, urls: &str) -> Result<(), Error> {
    let errors = client.add_all(urls);
    if errors.is_empty() {
        return Ok(());
    }
    Err(Error::InvalidUrls { urls: errors.iter().map(ToString::to_string).collect::<Vec<_>>().join("; ") })
}

/// The logo for the desktop notifiers.
const ICON: &[u8] = include_bytes!("../assets/images/logo.png");
/// Where the services that show an image download the logo, rather than mercurio's.
const ICON_URL: &str = env!("LOGO_URL");

/// Sends `text` at `urls`; warnings are logged under `collection`.
/// Logs each warning as the collection's.
pub(crate) fn logged(collection: Option<&str>) -> impl FnMut(String) + '_ {
    move |warning| log::event(collection, &Logged::Warned { warning })
}

/// Sends `text` at `urls`, its first line as the title. `warned`: with each warning of a delivery that went on.
pub(crate) fn send(warned: &mut dyn FnMut(String), urls: &str, level: Level, text: &str) -> Result<(), Error> {
    deliver(Mercurio::new(), warned, urls, level, text)
}

/// `warned`: with each warning of a delivery that went on.
fn deliver(
    client: Mercurio,
    warned: &mut dyn FnMut(String),
    urls: &str,
    level: Level,
    text: &str,
) -> Result<(), Error> {
    let client = client.asset(mercurio::Asset {
        image_bytes: Some(ICON),
        image_url_mask: Some(ICON_URL.into()),
        image_url_logo: Some(ICON_URL.into()),
        ..mercurio::Asset::default()
    });
    let mut client = client.app(crate::APP_NAME);
    add(&mut client, urls)?;
    if client.is_empty() {
        return Err(Error::NoUrls);
    }
    let kind = match level {
        Level::Success => NotifyType::Success,
        Level::Warning => NotifyType::Warning,
        Level::Failure => NotifyType::Failure,
    };
    let (title, body) = text.split_once('\n').unwrap_or((text, ""));
    let body = if body.trim().is_empty() { title } else { body };
    let report = client.notify(&Message::new(body).title(title).kind(kind));
    let error = report.error_summary();
    // A failed delivery's error is among its warnings too: it is reported once, as the error.
    let new = |w: &&String| !error.as_deref().is_some_and(|e| e.contains(w.as_str()));
    report.entries.iter().flat_map(|e| &e.warnings).filter(new).for_each(|w| warned(w.clone()));
    error.map_or(Ok(()), |e| Err(Error::Failed(e)))
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use mercurio::testing::FakeTransport;

    use super::*;

    #[test]
    fn sends_to_every_target_and_reports_failures() {
        let fake = Arc::new(FakeTransport::new());
        fake.respond(204, "");
        fake.respond(500, "failed");
        let client = Mercurio::with_transport(fake.clone());
        assert!(
            deliver(
                client,
                &mut |_| {},
                "json://localhost/one json://localhost/two",
                Level::Failure,
                "Docs failing\nmissing source"
            )
            .is_err()
        );
        let requests = fake.requests();
        assert_eq!(requests.len(), 2);
        let body = String::from_utf8_lossy(&requests[0].body);
        assert!(body.contains("Docs failing") && body.contains("missing source"));
    }

    #[test]
    fn maps_each_status_level() {
        for (level, expected, detail) in
            [(Level::Success, "success", ""), (Level::Warning, "warning", "body"), (Level::Failure, "failure", "body")]
        {
            let fake = Arc::new(FakeTransport::new());
            let client = Mercurio::with_transport(fake.clone());
            assert!(
                deliver(client, &mut |_| {}, "json://localhost/hook", level, &format!("Docs status\n{detail}")).is_ok()
            );
            let request = fake.last_request().unwrap();
            let body: serde_json::Value = serde_json::from_slice(&request.body).unwrap();
            assert_eq!(body["type"], expected);
            assert_eq!(body["title"], "Docs status");
            assert_eq!(body["message"], if detail.is_empty() { "Docs status" } else { detail });
        }
    }

    /// Services that show an icon show the logo.
    #[test]
    fn notifications_show_the_logo() {
        let sent = |url: &str| {
            let fake = Arc::new(FakeTransport::new());
            deliver(Mercurio::with_transport(fake.clone()), &mut |_| {}, url, Level::Warning, "title\nbody").unwrap();
            fake.last_request().unwrap()
        };
        let ntfy = sent("ntfy://localhost/topic");
        assert!(ntfy.headers.iter().any(|(k, v)| k == "X-Icon" && v == ICON_URL), "{:?}", ntfy.headers);
        let discord = sent("discord://1/token?format=markdown&footer=yes");
        let discord: serde_json::Value = serde_json::from_slice(&discord.body).unwrap();
        assert_eq!(
            (&discord["avatar_url"], &discord["embeds"][0]["footer"]["icon_url"]),
            (&ICON_URL.into(), &ICON_URL.into())
        );
    }

    /// A failed delivery reports its error once, not also as a warning. Its level is a warning: the collection's
    /// level does not count it.
    #[test]
    fn a_failure_is_reported_once() {
        let fake = Arc::new(FakeTransport::new());
        fake.respond(500, "failed");
        let mut warnings = Vec::new();
        let client = Mercurio::with_transport(fake);
        assert!(deliver(client, &mut |w| warnings.push(w), "json://localhost/hook", Level::Failure, "title").is_err());
        assert_eq!(warnings, Vec::<String>::new());
        let levels = [Logged::Notified { text: String::new() }, Logged::Undelivered { error: String::new() }];
        assert_eq!(levels.map(|l| log::Message::level(&l)), [Level::Success, Level::Warning]);
    }

    #[test]
    fn invalid_configuration_sends_nothing() {
        let fake = Arc::new(FakeTransport::new());
        let client = Mercurio::with_transport(fake.clone());
        assert!(
            deliver(client, &mut |_| {}, "json://localhost/one invalid://target", Level::Warning, "warning").is_err()
        );
        assert!(fake.requests().is_empty());
    }
}
