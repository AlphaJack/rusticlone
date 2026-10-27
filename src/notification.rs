//! Status change notifications, sent with Mercurio.

use mercurio::{Mercurio, Message, NotifyType};

use jiff::Timestamp;

use crate::{
    engine::{Collection, CollectionOverview, Engine},
    fmt,
    i18n::Fluent,
    log::{self, Level, Message as _},
    settings,
    state::Notified,
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

/// A notification's lines.
#[derive(Fluent)]
#[fluent(prefix = "notification")]
enum Line<'a> {
    Title {
        host: &'static str,
        collection: &'a str,
        level: Level,
    },
    /// A test before any collection exists.
    #[cfg(feature = "server")]
    Test {
        name: &'static str,
        emoji: &'static str,
    },
    Lasted {
        emoji: &'static str,
        level: Level,
        age: String,
    },
    Others {
        emoji: &'static str,
        count: u64,
        level: Level,
    },
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

impl Engine {
    /// Sends the collection's status when its level changed since the last one notified. A failed send is
    /// retried with backoff: returns when.
    pub(crate) fn notify(&self, collection: &Collection<'_>) -> Option<Timestamp> {
        let urls = settings::NOTIFY.get(&[collection.primary])?;
        // Nothing changed, so nothing to say.
        if self.store.is_dry_run() {
            return None;
        }
        let (key, title, now) = (collection.name(), collection.title, self.store.now());
        let status = self.status_of(collection);
        let level = status.level;
        // The attempt is recorded before sending, so that no other process sends it too, and a send that
        // never ends counts as failed.
        let update = self.store.update_notified(key, |n| match n {
            n if n.level == level => Notified { level, since: n.since, ..Notified::default() },
            n if n.retry_at().is_some_and(|at| now < at) => n.clone(),
            n => Notified { failures: n.failures + 1, last_attempt: Some(now), ..n.clone() },
        });
        let notified = match update {
            Ok(Some(attempt)) if attempt.level != level => {
                let sent =
                    deliver(Mercurio::new(), Some(title), &urls, level, &self.message(collection, &status, &attempt));
                self.record_sent(key, title, level, &sent).unwrap_or(attempt)
            }
            Ok(_) => self.store.notified(key),
            Err(e) => {
                log::event(Some(title), &Logged::NotRecorded { error: format!("{e:#}") });
                return None;
            }
        };
        notified.retry_at()
    }

    /// Sends the collection's status now, whatever was notified before, and returns its text. A dry run
    /// sends nothing.
    pub(crate) fn notify_now(&self, collection: &Collection<'_>) -> Result<String, Error> {
        let status = self.status_of(collection);
        let urls = settings::NOTIFY.get(&[collection.primary]).ok_or(Error::NoUrls)?;
        let before = self.store.notified(collection.name());
        let text = self.message(collection, &status, &before);
        if self.store.is_dry_run() {
            return Ok(text);
        }
        let sent = deliver(Mercurio::new(), Some(collection.title), &urls, status.level, &text);
        if sent.is_ok() {
            self.record_sent(collection.name(), collection.title, status.level, &sent);
        }
        sent.map(|()| text)
    }

    /// The status as a notification's text: its level, and its issues except about notifications. A recovery
    /// has none: it says how long the level notified `before` lasted.
    pub(crate) fn message(
        &self,
        collection: &Collection<'_>,
        status: &CollectionOverview,
        before: &Notified,
    ) -> String {
        let now = self.store.now();
        let title = Line::Title { host: crate::sys::hostname(), collection: collection.title, level: status.level };
        let mut text = format!("{title}\n");
        text.extend(
            (status.issues.iter().filter(|p| p.counts()))
                .map(|p| format!("{} {}\n", p.level().emoji(), fmt::capitalized(&p.text(now)))),
        );
        if let Some(since) = before.since.filter(|_| status.level == Level::Success && before.level != status.level) {
            let age = fmt::human_age(since, now);
            let lasted = Line::Lasted { emoji: status.level.emoji(), level: before.level, age };
            text.push_str(&format!("{lasted}\n"));
            text.push_str(&self.others(collection));
        }
        text
    }

    /// A line for each level but healthy, with how many other collections are at it. Only those notified
    /// at the same URLs are counted, since these already hear about them.
    fn others(&self, collection: &Collection<'_>) -> String {
        let urls = settings::NOTIFY.get(&[collection.primary]);
        let others: Vec<_> = self
            .every_collection()
            .filter(|c| c.name() != collection.name() && settings::NOTIFY.get(&[c.primary]) == urls)
            .collect();
        let levels = self.levels(&others);
        [Level::Failure, Level::Warning]
            .into_iter()
            .filter_map(|level| {
                let n = levels.iter().filter(|l| **l == level).count() as u64;
                (n > 0).then(|| format!("{}\n", Line::Others { emoji: level.emoji(), count: n, level }))
            })
            .collect()
    }

    /// Records a send of `level` about `key`. Returns the record, if stored.
    fn record_sent(&self, key: &str, title: &str, level: Level, sent: &Result<(), Error>) -> Option<Notified> {
        if let Err(e) = sent {
            log::event(Some(title), &Logged::Undelivered { error: e.to_string() });
        }
        let now = self.store.now();
        let recorded = self.store.update_notified(key, |n| match sent {
            Ok(()) => {
                Notified { level, since: n.since.filter(|_| n.level == level).or(Some(now)), ..Notified::default() }
            }
            Err(e) => Notified { error: Some(e.to_string()), ..n.clone() },
        });
        recorded.unwrap_or_else(|e| {
            log::event(Some(title), &Logged::NotRecorded { error: format!("{e:#}") });
            None
        })
    }
}

/// Sends a test to `urls`, such as the ones a new collection would use.
#[cfg(feature = "server")]
pub(crate) fn test(urls: &str) -> Result<(), Error> {
    check(urls)?;
    deliver(
        Mercurio::new(),
        None,
        urls,
        Level::Success,
        &Line::Test { name: crate::APP_NAME, emoji: Level::Success.emoji() }.to_string(),
    )
}

/// Refuses `urls` if mercurio does not read one of them.
#[cfg(any(feature = "server", test))]
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
fn deliver(client: Mercurio, collection: Option<&str>, urls: &str, level: Level, text: &str) -> Result<(), Error> {
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
    for warning in report.entries.iter().flat_map(|e| &e.warnings) {
        log::event(collection, &Logged::Warned { warning: warning.clone() });
    }
    report.error_summary().map_or(Ok(()), |e| Err(Error::Failed(e)))
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
                None,
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
            assert!(deliver(client, None, "json://localhost/hook", level, &format!("Docs status\n{detail}")).is_ok());
            let request = fake.last_request().unwrap();
            let body: serde_json::Value = serde_json::from_slice(&request.body).unwrap();
            assert_eq!(body["type"], expected);
            assert_eq!(body["title"], "Docs status");
            assert_eq!(body["message"], if detail.is_empty() { "Docs status" } else { detail });
        }
    }

    #[test]
    fn invalid_configuration_sends_nothing() {
        let fake = Arc::new(FakeTransport::new());
        let client = Mercurio::with_transport(fake.clone());
        assert!(deliver(client, None, "json://localhost/one invalid://target", Level::Warning, "warning").is_err());
        assert!(fake.requests().is_empty());
    }
}
