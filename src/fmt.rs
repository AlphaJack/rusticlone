//! Human-readable sizes, durations and ages.

use bytesize::ByteSize;
use jiff::{RoundMode, SignedDuration, Span, SpanRound, Timestamp, Unit, tz::TimeZone};

pub(crate) fn human_size(bytes: u64) -> String {
    ByteSize(bytes).display().iec().to_string()
}

/// Its largest non-zero unit: `2 months`.
fn largest(span: Span) -> String {
    let units = [
        (i64::from(span.get_years()), "year"),
        (i64::from(span.get_months()), "month"),
        (i64::from(span.get_days()), "day"),
        (i64::from(span.get_hours()), "hour"),
        (span.get_minutes(), "minute"),
        (span.get_seconds(), "second"),
    ];
    let (n, unit) = units.iter().find(|(n, _)| *n != 0).unwrap_or(&units[units.len() - 1]);
    crate::i18n::t!("fmt-span", n = n.unsigned_abs(), unit = *unit)
}

/// Rounded to its largest unit, at most days: `3 hours`, `59 seconds`.
pub(crate) fn human_duration(secs: u64) -> String {
    let span = Span::new().seconds(i64::try_from(secs).unwrap_or(i64::MAX));
    largest(span.round(SpanRound::new().largest(Unit::Day).days_are_24_hours()).unwrap_or(span))
}

/// An age without a number.
#[derive(crate::i18n::Fluent)]
#[fluent(prefix = "fmt-age")]
enum Age {
    UnderAMinute,
    Long,
    Ago { age: String },
}

/// Calendar time between `since` and `now` in its largest unit, truncated: `2 days`. Under an hour, minutes,
/// so that ages shown live change at most once a minute.
pub(crate) fn human_age(since: Timestamp, now: Timestamp) -> String {
    if now.duration_since(since) < SignedDuration::from_mins(1) {
        return Age::UnderAMinute.to_string();
    }
    let (since, now) = (since.to_zoned(TimeZone::system()), now.to_zoned(TimeZone::system()));
    let truncate = SpanRound::new().smallest(Unit::Minute).mode(RoundMode::Trunc).relative(&since);
    match since.until((Unit::Year, &now)).and_then(|span| span.round(truncate)) {
        Ok(span) => largest(span),
        Err(_) => Age::Long.to_string(),
    }
}

/// [`human_age`] ago: `3 hours ago`.
pub(crate) fn human_ago(since: Timestamp, now: Timestamp) -> String {
    Age::Ago { age: human_age(since, now) }.to_string()
}

/// With its first letter in upper case, to start a line: `Snapshot to Docs: ...`.
pub(crate) fn capitalized(text: &str) -> String {
    let mut chars = text.chars();
    chars.next().map(|c| c.to_uppercase().chain(chars).collect()).unwrap_or_default()
}

/// `2026-10-22 11:03`, in local time.
pub(crate) fn local(t: Timestamp) -> String {
    t.to_zoned(TimeZone::system()).strftime("%Y-%m-%d %H:%M").to_string()
}

/// `11:03:42`, in local time.
#[cfg(feature = "server")]
pub(crate) fn local_time(t: Timestamp) -> String {
    t.to_zoned(TimeZone::system()).strftime("%H:%M:%S").to_string()
}

/// Hides the password in a repository URL such as `rest:https://user:password@host/`.
pub(crate) fn redact(repository: &str) -> String {
    let Some((scheme, rest)) = repository.split_once("://") else { return repository.to_owned() };
    let host_end = rest.find('/').unwrap_or(rest.len());
    let (authority, path) = rest.split_at(host_end);
    match authority.rsplit_once('@') {
        Some((user, host)) => {
            let user = user.split_once(':').map_or(user, |(user, _)| user);
            format!("{scheme}://{user}:***@{host}{path}")
        }
        None => repository.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn human_units() {
        assert_eq!(human_size(512), "512 B");
        assert_eq!(human_size(3 << 29), "1.5 GiB");
        assert_eq!(human_duration(0), "0 seconds");
        assert_eq!(human_duration(59), "59 seconds");
        assert_eq!(human_duration(61), "1 minute");
        assert_eq!(human_duration(3600 * 3 + 125), "3 hours");
        assert_eq!(human_duration(86_400 + 300), "1 day");
        let now: Timestamp = "2026-10-14T12:00:00Z".parse().unwrap();
        let ago = |secs: i64| human_age(now - SignedDuration::from_secs(secs), now);
        assert_eq!((ago(59), ago(3 * 3600 + 125)), ("less than a minute".into(), "3 hours".into()));
        // Calendar months: July 28 to October 14.
        assert_eq!(ago(86_400 * 78), "2 months");
        assert_eq!(redact("rest:https://me:secret@host:8000/repo"), "rest:https://me:***@host:8000/repo");
        assert_eq!(redact("rclone:remote:path/to/repo"), "rclone:remote:path/to/repo");
    }
}
