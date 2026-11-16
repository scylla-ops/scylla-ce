//! The time zones: the names of the IANA database, and the hours, days and weeks of a zone.

use std::str::FromStr;

use chrono::{DateTime, Datelike, Days, NaiveDate, NaiveTime, Offset, TimeDelta};
use time::{Duration, OffsetDateTime};

use crate::DomainError;

/// A zone of the IANA database, for example `Europe/Paris`. `chrono-tz` holds the database.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TimeZone(chrono_tz::Tz);

/// A length of calendar time in a zone.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Span {
    /// An hour.
    Hour,
    /// A day, from 00:00.
    Day,
    /// A week, from Monday 00:00.
    Week,
}

impl TimeZone {
    /// The zone of a new workspace.
    pub const UTC: Self = Self(chrono_tz::Tz::UTC);

    /// Trims the text, and checks that it is the name of a zone, in the case of the database.
    pub fn parse(raw: String) -> Result<Self, DomainError> {
        chrono_tz::Tz::from_str(raw.trim())
            .map(Self)
            .map_err(|_| DomainError::NotATimeZone)
    }

    /// Returns the name of the zone.
    pub fn as_str(&self) -> &'static str {
        self.0.name()
    }

    /// Returns the start of the hour, the day or the week of `at` in this zone.
    pub fn start_of(&self, at: OffsetDateTime, span: Span) -> OffsetDateTime {
        let local = self.local(at);
        match span {
            Span::Hour => {
                let offset = i64::from(local.offset().fix().local_minus_utc());
                let seconds = at.unix_timestamp() + offset;
                instant(seconds - seconds.rem_euclid(3600) - offset)
            }
            Span::Day => self.midnight(local.date_naive()),
            Span::Week => {
                let date = local.date_naive();
                let monday = u64::from(date.weekday().num_days_from_monday());
                self.midnight(date - Days::new(monday))
            }
        }
    }

    /// Returns the start of the hour, the day or the week after the one that starts at `start`.
    pub fn next_start(&self, start: OffsetDateTime, span: Span) -> OffsetDateTime {
        let date = self.local(start).date_naive();
        match span {
            Span::Hour => {
                let next = self.start_of(start + Duration::HOUR, Span::Hour);
                next.max(start + Duration::HOUR / 2)
            }
            Span::Day => self.midnight(date + Days::new(1)),
            Span::Week => self.midnight(date + Days::new(7)),
        }
    }

    /// Returns the time of `at` in this zone, to the second.
    fn local(&self, at: OffsetDateTime) -> DateTime<chrono_tz::Tz> {
        DateTime::from_timestamp(at.unix_timestamp(), 0)
            .unwrap_or_default()
            .with_timezone(&self.0)
    }

    /// Returns the start of a day in this zone: 00:00, or the end of a gap of the zone at 00:00.
    fn midnight(&self, date: NaiveDate) -> OffsetDateTime {
        let start = date.and_time(NaiveTime::MIN);
        (0..=96)
            .map(|quarter| start + TimeDelta::minutes(15 * quarter))
            .find_map(|local| local.and_local_timezone(self.0).earliest())
            .map_or_else(
                || instant(start.and_utc().timestamp()),
                |found| instant(found.timestamp()),
            )
    }
}

impl Default for TimeZone {
    fn default() -> Self {
        Self::UTC
    }
}

/// Returns the time of a number of seconds after 1970-01-01 00:00 UTC.
fn instant(seconds: i64) -> OffsetDateTime {
    OffsetDateTime::from_unix_timestamp(seconds).unwrap_or(OffsetDateTime::UNIX_EPOCH)
}

#[cfg(test)]
mod tests {
    use time::{Date, Month, PrimitiveDateTime, Time};

    use super::*;

    /// Returns a time of a day of 2026 in UTC.
    fn utc(month: Month, day: u8, hour: u8, minute: u8) -> OffsetDateTime {
        let date = Date::from_calendar_date(2026, month, day).unwrap();
        let time = Time::from_hms(hour, minute, 0).unwrap();
        PrimitiveDateTime::new(date, time).assume_utc()
    }

    /// Returns a zone of the database.
    fn zone(name: &str) -> TimeZone {
        TimeZone::parse(name.to_owned()).unwrap()
    }

    #[test]
    fn a_day_starts_at_midnight_in_the_zone() {
        let paris = zone("Europe/Paris");

        let start = paris.start_of(utc(Month::November, 10, 8, 0), Span::Day);

        assert_eq!(start, utc(Month::November, 9, 23, 0));
        assert_eq!(
            paris.next_start(start, Span::Day),
            utc(Month::November, 10, 23, 0)
        );
    }

    #[test]
    fn the_day_of_the_change_to_summer_time_has_23_hours() {
        let paris = zone("Europe/Paris");

        let start = paris.start_of(utc(Month::March, 29, 12, 0), Span::Day);
        let next = paris.next_start(start, Span::Day);

        assert_eq!(start, utc(Month::March, 28, 23, 0));
        assert_eq!(next, utc(Month::March, 29, 22, 0));
    }

    #[test]
    fn a_week_starts_on_monday_at_midnight_in_the_zone() {
        let new_york = zone("America/New_York");

        let start = new_york.start_of(utc(Month::November, 12, 3, 0), Span::Week);

        assert_eq!(start, utc(Month::November, 9, 5, 0));
        assert_eq!(
            new_york.next_start(start, Span::Week),
            utc(Month::November, 16, 5, 0)
        );
    }

    #[test]
    fn an_hour_starts_at_the_hour_of_the_zone() {
        let kolkata = zone("Asia/Kolkata");

        let start = kolkata.start_of(utc(Month::November, 10, 10, 10), Span::Hour);

        assert_eq!(start, utc(Month::November, 10, 9, 30));
        assert_eq!(
            kolkata.next_start(start, Span::Hour),
            utc(Month::November, 10, 10, 30)
        );
    }

    #[test]
    fn the_hours_of_the_change_to_winter_time_are_all_there() {
        let paris = zone("Europe/Paris");
        let start = paris.start_of(utc(Month::October, 25, 0, 10), Span::Hour);

        let next = paris.next_start(start, Span::Hour);
        let after = paris.next_start(next, Span::Hour);

        assert_eq!(start, utc(Month::October, 25, 0, 0));
        assert_eq!(next, utc(Month::October, 25, 1, 0));
        assert_eq!(after, utc(Month::October, 25, 2, 0));
    }

    #[test]
    fn a_day_with_no_midnight_starts_at_the_end_of_the_gap() {
        let santiago = zone("America/Santiago");

        let start = santiago.start_of(utc(Month::September, 6, 12, 0), Span::Day);

        assert_eq!(start, utc(Month::September, 6, 4, 0));
    }
}
