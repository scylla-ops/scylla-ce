//! The job stats of the agents: the ends of their jobs on each day of a period, and their
//! durations.

use time::{Duration, OffsetDateTime, Time, UtcOffset};

use crate::identity::apps::AppId;
use crate::DomainError;

/// The number of days of the stats when a request gives no first day, the last day included.
pub const STATS_DAYS: u8 = 30;

/// The largest number of days of one period, the last day included.
pub const MAX_STATS_DAYS: u16 = 366;

/// The number of jobs for each end.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Outcomes {
    /// The jobs that completed.
    pub completed: u32,
    /// The jobs that failed.
    pub failed: u32,
    /// The jobs that a user cancelled.
    pub cancelled: u32,
    /// The jobs that ended because their agent went away or was disabled.
    pub orphaned: u32,
}

/// The jobs of one day, by their end.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DayOutcomes {
    /// The start of the day, at 00:00 UTC.
    pub day: OffsetDateTime,
    /// The jobs created on that day.
    pub outcomes: Outcomes,
}

/// The job stats of an agent for the days of a `StatsPeriod`. A job counts on the day of its
/// creation, and only when it ended.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentStats {
    /// The agent.
    pub agent_id: AppId,
    /// The jobs of all the days.
    pub outcomes: Outcomes,
    /// The median time from the start to the end of the jobs that started and ended.
    pub median_duration: Option<Duration>,
    /// The 95th percentile of the same times.
    pub p95_duration: Option<Duration>,
    /// Each day, oldest first, also the days with no job.
    pub days: Vec<DayOutcomes>,
}

/// The days of the stats in UTC, from the start of `first` to the end of `last`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StatsPeriod {
    /// The start of the first day, at 00:00 UTC.
    pub first: OffsetDateTime,
    /// The start of the last day, at 00:00 UTC.
    pub last: OffsetDateTime,
}

impl StatsPeriod {
    /// Returns the last `STATS_DAYS` days that end on the day of `now`.
    pub fn recent(now: OffsetDateTime) -> Self {
        let last = day_of(now);
        Self {
            first: last - Duration::days(i64::from(STATS_DAYS) - 1),
            last,
        }
    }

    /// Returns the period of a request on an agent created at `joined`. The last day is today
    /// when it is not set or later. The first day is `STATS_DAYS - 1` days before the last day
    /// when it is not set. A first day that is set moves to the day of `joined` if it is earlier.
    /// The period keeps at most `MAX_STATS_DAYS` days. Refuses a period that ends before it
    /// starts.
    pub fn of_request(
        first: Option<OffsetDateTime>,
        last: Option<OffsetDateTime>,
        joined: OffsetDateTime,
        now: OffsetDateTime,
    ) -> Result<Self, DomainError> {
        // Find the last day.
        let today = day_of(now);
        let last = last.map_or(today, |last| day_of(last).min(today));

        // Find the first day.
        let oldest = last - Duration::days(i64::from(MAX_STATS_DAYS) - 1);
        let first = match first {
            Some(first) => day_of(first).max(day_of(joined)),
            None => last - Duration::days(i64::from(STATS_DAYS) - 1),
        };

        // Refuse an empty period.
        let first = first.max(oldest);
        if first > last {
            return Err(DomainError::BeforeStart);
        }
        Ok(Self { first, last })
    }

    /// Returns the start of the day after the last day: the end of the period.
    pub fn end(&self) -> OffsetDateTime {
        self.last + Duration::DAY
    }

    /// Returns each day of the period, with the outcomes of `busy` on its days and none on the
    /// other days.
    pub fn days(&self, busy: &[DayOutcomes]) -> Vec<DayOutcomes> {
        let count = (self.last - self.first).whole_days() + 1;
        (0..count)
            .map(|offset| {
                let day = self.first + Duration::days(offset);
                let found = busy.iter().find(|busy| busy.day == day);
                let outcomes = found.map(|busy| busy.outcomes).unwrap_or_default();
                DayOutcomes { day, outcomes }
            })
            .collect()
    }
}

/// Returns the start of the day of `at` in UTC.
fn day_of(at: OffsetDateTime) -> OffsetDateTime {
    at.to_offset(UtcOffset::UTC).replace_time(Time::MIDNIGHT)
}

#[cfg(test)]
mod tests {
    use time::{Date, Month, PrimitiveDateTime};

    use super::*;

    /// Returns a time on a day of 2025 or 2026, at `hour` in the offset `offset` hours.
    fn at(year: i32, month: Month, day: u8, hour: u8, offset: i8) -> OffsetDateTime {
        let date = Date::from_calendar_date(year, month, day).unwrap();
        let time = Time::from_hms(hour, 0, 0).unwrap();
        let offset = UtcOffset::from_hms(offset, 0, 0).unwrap();
        PrimitiveDateTime::new(date, time).assume_offset(offset)
    }

    /// Returns the start of a day of November or October 2026 in UTC.
    fn day(month: Month, day: u8) -> OffsetDateTime {
        at(2026, month, day, 0, 0)
    }

    #[test]
    fn the_recent_stats_start_29_days_before_today_at_midnight_utc() {
        let period = StatsPeriod::recent(at(2026, Month::November, 4, 23, -2));
        assert_eq!(period.first, day(Month::October, 7));
        assert_eq!(period.last, day(Month::November, 5));
    }

    #[test]
    fn a_request_with_no_period_gets_the_recent_days() {
        let now = at(2026, Month::November, 9, 10, 0);
        let joined = day(Month::November, 8);

        let period = StatsPeriod::of_request(None, None, joined, now).unwrap();

        assert_eq!(period, StatsPeriod::recent(now));
    }

    #[test]
    fn a_request_takes_the_utc_days_of_its_times() {
        let now = at(2026, Month::November, 9, 10, 0);
        let joined = day(Month::January, 1);
        let first = at(2026, Month::October, 11, 23, -3);
        let last = at(2026, Month::October, 20, 1, 2);

        let period = StatsPeriod::of_request(Some(first), Some(last), joined, now).unwrap();

        assert_eq!(period.first, day(Month::October, 12));
        assert_eq!(period.last, day(Month::October, 19));
        assert_eq!(period.end(), day(Month::October, 20));
    }

    #[test]
    fn a_request_starts_at_the_creation_of_the_agent_and_ends_today() {
        let now = at(2026, Month::November, 9, 10, 0);
        let joined = at(2026, Month::July, 14, 15, 0);
        let first = day(Month::January, 1);
        let last = day(Month::December, 31);

        let period = StatsPeriod::of_request(Some(first), Some(last), joined, now).unwrap();

        assert_eq!(period.first, day(Month::July, 14));
        assert_eq!(period.last, day(Month::November, 9));
    }

    #[test]
    fn a_request_keeps_at_most_a_year_of_days() {
        let now = at(2026, Month::November, 9, 10, 0);
        let joined = at(2020, Month::January, 1, 0, 0);
        let first = at(2024, Month::January, 1, 0, 0);

        let period = StatsPeriod::of_request(Some(first), None, joined, now).unwrap();

        assert_eq!(period.first, at(2025, Month::November, 9, 0, 0));
        assert_eq!(period.days(&[]).len(), usize::from(MAX_STATS_DAYS));
    }

    #[test]
    fn a_request_with_no_first_day_gets_30_days_before_its_last_day() {
        let now = at(2026, Month::November, 9, 10, 0);
        let joined = day(Month::November, 1);
        let last = day(Month::October, 30);

        let period = StatsPeriod::of_request(None, Some(last), joined, now).unwrap();

        assert_eq!(period.first, day(Month::October, 1));
        assert_eq!(period.days(&[]).len(), usize::from(STATS_DAYS));
    }

    #[test]
    fn a_request_that_ends_before_it_starts_is_refused() {
        let now = at(2026, Month::November, 9, 10, 0);
        let joined = day(Month::October, 1);
        let first = day(Month::October, 20);
        let last = day(Month::October, 10);
        let early = day(Month::September, 1);

        let reversed = StatsPeriod::of_request(Some(first), Some(last), joined, now);
        let before_joined = StatsPeriod::of_request(Some(early), Some(early), joined, now);

        assert_eq!(reversed, Err(DomainError::BeforeStart));
        assert_eq!(before_joined, Err(DomainError::BeforeStart));
    }

    #[test]
    fn each_day_gets_an_entry_and_keeps_its_outcomes() {
        let period = StatsPeriod {
            first: day(Month::October, 6),
            last: day(Month::November, 4),
        };
        let outcomes = Outcomes {
            completed: 2,
            failed: 1,
            ..Outcomes::default()
        };
        let busy = DayOutcomes {
            day: day(Month::October, 8),
            outcomes,
        };

        let days = period.days(&[busy]);

        assert_eq!(days.len(), usize::from(STATS_DAYS));
        assert_eq!(days[0].day, period.first);
        assert_eq!(days[2], busy);
        assert_eq!(days[29].day, day(Month::November, 4));
        let empty = days
            .iter()
            .filter(|day| day.outcomes == Outcomes::default());
        assert_eq!(empty.count(), 29);
    }
}
