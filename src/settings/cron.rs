//! Classic cron expressions: minute hour day month weekday, with lists, ranges, steps, names and the
//! `@daily`-style aliases, in a time zone.

use std::ops::RangeInclusive;

use jiff::{
    Timestamp,
    civil::{Date, DateTime},
    tz::{AmbiguousOffset, TimeZone},
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Cron {
    /// Bit `n` is set when `n` matches.
    minutes: u64,
    hours: u64,
    days: u64,
    months: u64,
    weekdays: u64,
    /// Both the day and the weekday are restricted: either matching is enough, as in every cron.
    either_day: bool,
}

const MINUTES: RangeInclusive<u8> = 0..=59;
const HOURS: RangeInclusive<u8> = 0..=23;
const DAYS: RangeInclusive<u8> = 1..=31;
const MONTHS: RangeInclusive<u8> = 1..=12;
/// Sunday is 0 or 7.
const WEEKDAYS: RangeInclusive<u8> = 0..=7;
const MONTH_NAMES: [&str; 12] = ["jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec"];
const WEEKDAY_NAMES: [&str; 7] = ["sun", "mon", "tue", "wed", "thu", "fri", "sat"];
/// Long enough for the next 29 February.
const SEARCHED_DAYS: usize = 366 * 8;

impl Cron {
    pub(crate) fn parse(expr: &str) -> Option<Self> {
        let expr = match expr.trim() {
            "@yearly" | "@annually" => "0 0 1 1 *",
            "@monthly" => "0 0 1 * *",
            "@weekly" => "0 0 * * 0",
            "@daily" => "0 0 * * *",
            "@hourly" => "0 * * * *",
            expr => expr,
        };
        let fields: Vec<&str> = expr.split_whitespace().collect();
        let [minute, hour, day, month, weekday] = fields[..] else { return None };
        let weekdays = field(weekday, WEEKDAYS, &WEEKDAY_NAMES)?;
        Some(Self {
            minutes: field(minute, MINUTES, &[])?,
            hours: field(hour, HOURS, &[])?,
            days: field(day, DAYS, &[])?,
            months: field(month, MONTHS, &MONTH_NAMES)?,
            weekdays: weekdays | weekdays >> WEEKDAY_NAMES.len(),
            either_day: !day.starts_with('*') && !weekday.starts_with('*'),
        })
    }

    /// The first time strictly after `after`. A time the clocks skip fires as they jump past it, and one
    /// they repeat fires the first time only. `None` if it never comes, such as on 30 February.
    pub(crate) fn next_after(&self, after: Timestamp, tz: &TimeZone) -> Option<Timestamp> {
        let mut date = tz.to_datetime(after).date();
        for _ in 0..SEARCHED_DAYS {
            if self.on(date) {
                let times =
                    set(self.hours, HOURS).flat_map(|h| set(self.minutes, MINUTES).map(move |m| date.at(h, m, 0, 0)));
                if let Some(next) = times.filter_map(|dt| instant(dt, tz)).find(|t| *t > after) {
                    return Some(next);
                }
            }
            date = date.tomorrow().ok()?;
        }
        None
    }

    fn on(&self, date: Date) -> bool {
        let day = self.days >> date.day() & 1 == 1;
        let weekday = self.weekdays >> date.weekday().to_sunday_zero_offset() & 1 == 1;
        self.months >> date.month() & 1 == 1 && if self.either_day { day || weekday } else { day && weekday }
    }
}

/// The `values` that a field lists, as bits.
fn field(field: &str, values: RangeInclusive<u8>, names: &[&str]) -> Option<u64> {
    let (min, max) = (*values.start(), *values.end());
    let value = |v: &str| {
        let named = names.iter().position(|n| n.eq_ignore_ascii_case(v));
        let n = named.map_or_else(|| v.parse().ok(), |i| u8::try_from(i).ok().map(|i| min + i))?;
        values.contains(&n).then_some(n)
    };
    field.split(',').try_fold(0, |bits, part| {
        let (range, step) = part.split_once('/').map_or(Some((part, 1)), |(r, s)| Some((r, s.parse().ok()?)))?;
        let (from, to) = if range == "*" {
            (min, max)
        } else if let Some((from, to)) = range.split_once('-') {
            (value(from)?, value(to)?)
        } else {
            // `5/15` is `5-59/15`.
            let from = value(range)?;
            (from, if step > 1 { max } else { from })
        };
        (step > 0 && from <= to).then(|| (from..=to).step_by(step).fold(bits, |bits, n| bits | 1 << n))
    })
}

/// The `values` whose bits are set, as the hours or minutes of a day.
fn set(bits: u64, values: RangeInclusive<u8>) -> impl Iterator<Item = i8> {
    values.filter(move |n| bits >> n & 1 == 1).filter_map(|n| i8::try_from(n).ok())
}

fn instant(dt: DateTime, tz: &TimeZone) -> Option<Timestamp> {
    match tz.to_ambiguous_timestamp(dt).offset() {
        AmbiguousOffset::Unambiguous { offset } | AmbiguousOffset::Fold { before: offset, .. } => {
            offset.to_timestamp(dt).ok()
        }
        AmbiguousOffset::Gap { after, .. } => Some(tz.following(after.to_timestamp(dt).ok()?).next()?.timestamp()),
    }
}
