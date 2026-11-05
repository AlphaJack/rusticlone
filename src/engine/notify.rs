//! When a collection's status is notified, and what the notification says.

use jiff::Timestamp;

use super::{Collection, CollectionOverview, Engine};
use crate::{
    fmt,
    i18n::Fluent,
    log::{self, Level, Message as _},
    notification, settings,
    state::Notified,
};

/// A notification's lines.
#[derive(Fluent)]
#[fluent(prefix = "notification")]
enum Line<'a> {
    Title {
        host: &'static str,
        collection: &'a str,
        level: Level,
    },
    /// On a computer whose name is unknown.
    TitleUnnamed {
        collection: &'a str,
        level: Level,
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
        let notified = update.and_then(|attempt| match attempt {
            Some(attempt) if attempt.level != level => {
                let text = self.message(collection, &status, &attempt);
                let sent = notification::send(&mut notification::logged(Some(title)), &urls, level, &text);
                Ok(self.record_sent(key, title, level, &sent).unwrap_or(attempt))
            }
            _ => self.store.notified(key),
        });
        let notified = match notified {
            Ok(notified) => notified,
            Err(e) => {
                log::event(Some(title), &notification::Logged::NotRecorded { error: format!("{e:#}") });
                return None;
            }
        };
        notified.retry_at()
    }

    /// Sends the collection's status now, whatever was notified before, and returns its text. A dry run
    /// sends nothing. `warned`: with each warning of a delivery that went on.
    pub(crate) fn notify_now(
        &self,
        collection: &Collection<'_>,
        warned: &mut dyn FnMut(String),
    ) -> Result<String, notification::Error> {
        let status = self.status_of(collection);
        let urls = settings::NOTIFY.get(&[collection.primary]).ok_or(notification::Error::NoUrls)?;
        // Unread, it says the level without how long the one before lasted.
        let before = self.store.notified(collection.name()).unwrap_or_default();
        let text = self.message(collection, &status, &before);
        if self.store.is_dry_run() {
            return Ok(text);
        }
        let sent = notification::send(warned, &urls, status.level, &text);
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
        let (collection_title, level) = (collection.title, status.level);
        let title = match crate::sys::hostname() {
            Some(host) => Line::Title { host, collection: collection_title, level },
            None => Line::TitleUnnamed { collection: collection_title, level },
        };
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
    fn record_sent(
        &self,
        key: &str,
        title: &str,
        level: Level,
        sent: &Result<(), notification::Error>,
    ) -> Option<Notified> {
        if let Err(e) = sent {
            log::event(Some(title), &notification::Logged::Undelivered { error: e.to_string() });
        }
        let now = self.store.now();
        let recorded = self.store.update_notified(key, |n| match sent {
            Ok(()) => {
                Notified { level, since: n.since.filter(|_| n.level == level).or(Some(now)), ..Notified::default() }
            }
            Err(e) => Notified { error: Some(e.to_string()), ..n.clone() },
        });
        recorded.unwrap_or_else(|e| {
            log::event(Some(title), &notification::Logged::NotRecorded { error: format!("{e:#}") });
            None
        })
    }
}
