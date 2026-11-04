//! The job stats of the agents: the ends of their jobs on each of the last days, and their
//! durations.

use time::{Duration, OffsetDateTime, Time, UtcOffset};

use crate::identity::apps::AppId;

/// The number of days of the stats, today included.
pub const STATS_DAYS: u8 = 30;

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

/// The job stats of an agent for the last `STATS_DAYS` days in UTC, today included. A job
/// counts on the day of its creation, and only when it ended.
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

impl AgentStats {
    /// Returns the start of the first day of the stats that end on the day of `now`.
    pub fn since(now: OffsetDateTime) -> OffsetDateTime {
        let today = now.to_offset(UtcOffset::UTC).replace_time(Time::MIDNIGHT);
        today - Duration::days(i64::from(STATS_DAYS) - 1)
    }

    /// Returns each day from `since`, with the outcomes of `busy` on its days and none on the
    /// other days.
    pub fn days(since: OffsetDateTime, busy: &[DayOutcomes]) -> Vec<DayOutcomes> {
        (0..i64::from(STATS_DAYS))
            .map(|offset| {
                let day = since + Duration::days(offset);
                let found = busy.iter().find(|busy| busy.day == day);
                let outcomes = found.map(|busy| busy.outcomes).unwrap_or_default();
                DayOutcomes { day, outcomes }
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use time::{Date, Month, PrimitiveDateTime};

    use super::*;

    /// Returns a time on a day of October or November 2026, at `hour` in the offset `offset` hours.
    fn at(month: Month, day: u8, hour: u8, offset: i8) -> OffsetDateTime {
        let date = Date::from_calendar_date(2026, month, day).unwrap();
        let time = Time::from_hms(hour, 0, 0).unwrap();
        let offset = UtcOffset::from_hms(offset, 0, 0).unwrap();
        PrimitiveDateTime::new(date, time).assume_offset(offset)
    }

    #[test]
    fn the_stats_start_29_days_before_today_at_midnight_utc() {
        let since = AgentStats::since(at(Month::November, 4, 23, -2));
        assert_eq!(since, at(Month::October, 7, 0, 0));
    }

    #[test]
    fn each_day_gets_an_entry_and_keeps_its_outcomes() {
        let since = at(Month::October, 6, 0, 0);
        let outcomes = Outcomes {
            completed: 2,
            failed: 1,
            ..Outcomes::default()
        };
        let busy = DayOutcomes {
            day: at(Month::October, 8, 0, 0),
            outcomes,
        };

        let days = AgentStats::days(since, &[busy]);

        assert_eq!(days.len(), usize::from(STATS_DAYS));
        assert_eq!(days[0].day, since);
        assert_eq!(days[2], busy);
        assert_eq!(days[29].day, at(Month::November, 4, 0, 0));
        let empty = days
            .iter()
            .filter(|day| day.outcomes == Outcomes::default());
        assert_eq!(empty.count(), 29);
    }
}
