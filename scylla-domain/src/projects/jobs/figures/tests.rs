//! The tests of the period, the buckets and the order of the groups of the job figures.

use time::{Date, Month, PrimitiveDateTime, Time};

use super::*;

/// Returns a time of a day of November 2026 in UTC.
fn utc(day: u8, hour: u8) -> OffsetDateTime {
    let date = Date::from_calendar_date(2026, Month::November, day).unwrap();
    let time = Time::from_hms(hour, 0, 0).unwrap();
    PrimitiveDateTime::new(date, time).assume_utc()
}

/// Returns figures with a count, failures and a median in seconds.
fn figures(created: u32, failed: u32, p50: Option<i64>) -> Figures {
    Figures {
        created,
        outcomes: Outcomes {
            failed,
            ..Outcomes::default()
        },
        p50_duration: p50.map(Duration::seconds),
        ..Figures::default()
    }
}

/// Returns the key of an origin.
fn origin(origin: JobOrigin) -> GroupKey {
    GroupKey::Origin(origin)
}

#[test]
fn a_period_with_no_end_ends_now_and_the_previous_period_has_its_length() {
    let period = FiguresPeriod::new(utc(2, 0), None, utc(16, 0)).unwrap();

    assert_eq!(period.to, utc(16, 0));
    assert_eq!(
        period.previous(),
        FiguresPeriod::new(utc(2, 0) - Duration::days(14), Some(utc(2, 0)), utc(16, 0)).unwrap()
    );
}

#[test]
fn a_period_that_does_not_end_after_its_start_is_refused() {
    let now = utc(16, 0);

    let empty = FiguresPeriod::new(utc(10, 0), Some(utc(10, 0)), now);
    let reversed = FiguresPeriod::new(utc(10, 0), Some(utc(9, 0)), now);

    assert_eq!(empty, Err(DomainError::NotAfterStart));
    assert_eq!(reversed, Err(DomainError::NotAfterStart));
}

#[test]
fn a_period_keeps_at_most_400_days() {
    let to = utc(16, 0);
    let longest = to - Duration::days(MAX_FIGURES_DAYS);

    let kept = FiguresPeriod::new(longest, Some(to), to);
    let refused = FiguresPeriod::new(longest - Duration::SECOND, Some(to), to);

    assert!(kept.is_ok());
    assert_eq!(refused, Err(DomainError::PeriodTooLong { days: 400 }));
}

#[test]
fn the_buckets_start_at_the_days_of_the_zone() {
    let paris = TimeZone::parse("Europe/Paris".to_owned()).unwrap();
    let period = FiguresPeriod::new(utc(2, 10), Some(utc(4, 23)), utc(16, 0)).unwrap();

    let starts = period.starts(paris, Span::Day).unwrap();

    assert_eq!(starts, vec![utc(1, 23), utc(2, 23), utc(3, 23)]);
}

#[test]
fn more_than_1000_buckets_are_refused() {
    let to = utc(16, 0);
    let period = FiguresPeriod::new(to - Duration::hours(1001), Some(to), to).unwrap();
    let largest = FiguresPeriod::new(to - Duration::hours(1000), Some(to), to).unwrap();

    let refused = period.starts(TimeZone::UTC, Span::Hour);
    let kept = largest.starts(TimeZone::UTC, Span::Hour).unwrap();

    assert_eq!(refused, Err(DomainError::TooMany { max: MAX_BUCKETS }));
    assert_eq!(kept.len(), MAX_BUCKETS);
}

#[test]
fn each_start_gets_a_bucket_and_keeps_its_figures() {
    let starts = [utc(1, 0), utc(2, 0), utc(3, 0)];

    let found = buckets(&starts, &[(2, figures(4, 1, Some(30)))]);

    assert_eq!(found.len(), 3);
    assert_eq!(found[0].figures, Figures::default());
    assert_eq!(found[1].start, utc(2, 0));
    assert_eq!(found[1].figures.created, 4);
    assert_eq!(found[2].figures, Figures::default());
}

#[test]
fn the_groups_with_the_most_failures_come_first_then_the_key() {
    let groups = vec![
        (origin(JobOrigin::Push), figures(9, 1, None)),
        (origin(JobOrigin::Cron), figures(2, 3, None)),
        (origin(JobOrigin::Manual), figures(5, 3, None)),
    ];

    let found = top(groups, None, GroupOrder::Failed, 20);

    let keys: Vec<GroupKey> = found.iter().map(|group| group.key).collect();
    assert_eq!(
        keys,
        vec![
            origin(JobOrigin::Manual),
            origin(JobOrigin::Cron),
            origin(JobOrigin::Push)
        ]
    );
    assert!(found.iter().all(|group| group.previous.is_none()));
}

#[test]
fn the_slowest_groups_come_first_and_a_group_with_no_median_comes_last() {
    let groups = vec![
        (origin(JobOrigin::Api), figures(1, 0, None)),
        (origin(JobOrigin::Push), figures(1, 0, Some(60))),
        (origin(JobOrigin::Cron), figures(1, 0, Some(300))),
    ];

    let found = top(groups, None, GroupOrder::P50Duration, 2);

    let keys: Vec<GroupKey> = found.iter().map(|group| group.key).collect();
    assert_eq!(keys, vec![origin(JobOrigin::Cron), origin(JobOrigin::Push)]);
}

#[test]
fn each_group_gets_its_previous_figures_or_figures_at_0() {
    let groups = vec![
        (origin(JobOrigin::Push), figures(9, 0, None)),
        (origin(JobOrigin::Cron), figures(2, 0, None)),
    ];
    let previous = vec![
        (origin(JobOrigin::Push), figures(7, 0, None)),
        (origin(JobOrigin::Tag), figures(1, 0, None)),
    ];

    let found = top(groups, Some(previous), GroupOrder::Count, 20);

    assert_eq!(found[0].previous, Some(figures(7, 0, None)));
    assert_eq!(found[1].previous, Some(Figures::default()));
}
