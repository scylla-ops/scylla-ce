//! Unit tests of the agents: the protocol of the host, and the figures of the busy slots and of
//! the samples of the machines, in their buckets.

mod labels;

use time::{Duration, OffsetDateTime};

use super::*;
use crate::identity::apps::AppId;
use crate::tenancy::workspaces::WorkspaceId;
use crate::{Span, TimeZone};

/// 2026-11-02 00:00 UTC.
fn monday() -> OffsetDateTime {
    OffsetDateTime::from_unix_timestamp(1_793_577_600).unwrap()
}

/// Returns a query of the slots of a period of `hours` hours from `monday`, by hour, at `now`.
fn query(hours: i64, now: OffsetDateTime) -> SlotQuery {
    let from = monday();
    SlotQuery {
        workspace: WorkspaceId::generate(),
        agent: None,
        period: FiguresPeriod::new(from, Some(from + Duration::hours(hours)), now).unwrap(),
        bucket: Some(Span::Hour),
        now,
    }
}

/// Returns a busy time of `minutes` minutes with a largest number.
fn busy(minutes: i64, max: u32) -> Busy {
    Busy {
        time: Duration::minutes(minutes),
        max,
    }
}

/// The mean of a bucket is its busy time divided by its length, and the totals add the parts.
#[test]
fn the_mean_of_a_bucket_is_its_busy_time_divided_by_its_length() {
    // given
    let query = query(3, monday() + Duration::hours(10));
    let starts = query.period.starts(TimeZone::UTC, Span::Hour).unwrap();
    let bounds = bounds(query.period, &starts);

    // when
    let usage = SlotUsage::of_parts(
        &query,
        &starts,
        &bounds,
        &[(1, busy(90, 2)), (3, busy(30, 1))],
        TimeZone::UTC,
    );

    // then
    let means: Vec<f64> = usage.buckets.iter().map(|b| b.figures.busy_avg).collect();
    assert_eq!(means, vec![1.5, 0.0, 0.5]);
    assert_eq!(usage.buckets[0].figures.busy_max, 2);
    assert_eq!(usage.totals.busy_avg, 2.0 / 3.0);
    assert_eq!(usage.totals.busy_max, 2);
}

/// A bucket that holds now counts its length up to now.
#[test]
fn the_slots_count_up_to_now() {
    // given
    let query = query(2, monday() + Duration::minutes(90));
    let starts = query.period.starts(TimeZone::UTC, Span::Hour).unwrap();
    let bounds = bounds(query.period, &starts);

    // when
    let usage = SlotUsage::of_parts(&query, &starts, &bounds, &[(2, busy(30, 1))], TimeZone::UTC);

    // then
    assert_eq!(usage.buckets[1].figures.busy_avg, 1.0);
    assert_eq!(usage.totals.busy_avg, 1.0 / 3.0);
}

/// With no bucket, the period is one part and gives the totals.
#[test]
fn a_period_with_no_bucket_is_one_part() {
    // given
    let mut query = query(4, monday() + Duration::days(1));
    query.bucket = None;
    let bounds = bounds(query.period, &[]);

    // when
    let usage = SlotUsage::of_parts(&query, &[], &bounds, &[(1, busy(120, 3))], TimeZone::UTC);

    // then
    assert!(usage.buckets.is_empty());
    assert_eq!(usage.totals.busy_avg, 0.5);
    assert_eq!(usage.totals.busy_max, 3);
}

/// A period after now has its buckets, each at 0.
#[test]
fn a_period_in_the_future_has_no_busy_slot() {
    // given
    let query = query(2, monday() - Duration::hours(1));
    let starts = query.period.starts(TimeZone::UTC, Span::Hour).unwrap();
    let bounds = bounds(query.period, &starts);

    // when
    let usage = SlotUsage::of_parts(&query, &starts, &bounds, &[], TimeZone::UTC);

    // then
    assert_eq!(usage.totals, SlotFigures::default());
    assert_eq!(usage.buckets.len(), 2);
}

/// Returns the sums of samples.
fn sums(samples: u32, cpu_sum: f64, cpu_max: f32, memory_sum: f64, memory_max: u64) -> SampleSums {
    SampleSums {
        samples,
        cpu_sum,
        cpu_max,
        memory_sum,
        memory_max,
    }
}

/// The means of a bucket are its sums divided by its number of samples.
#[test]
fn the_metrics_give_the_means_and_the_largest_values() {
    // given
    let starts = [monday(), monday() + Duration::hours(1)];
    let found = [(2, sums(4, 100.0, 40.0, 4000.0, 1500))];

    // when
    let metrics = AgentMetrics::of_parts(&starts, &found, TimeZone::UTC);

    // then
    assert_eq!(metrics.buckets[0].figures, MetricsFigures::default());
    let second = metrics.buckets[1].figures;
    assert_eq!(
        (second.samples, second.cpu_avg, second.cpu_max),
        (4, 25.0, 40.0)
    );
    assert_eq!((second.memory_avg, second.memory_max), (1000, 1500));
    assert_eq!(metrics.totals, second);
}

/// The totals weigh each sample, not each bucket.
#[test]
fn the_totals_of_the_metrics_weigh_each_sample() {
    // given
    let starts = [monday(), monday() + Duration::hours(1)];
    let found = [
        (1, sums(1, 90.0, 90.0, 300.0, 300)),
        (2, sums(3, 30.0, 20.0, 300.0, 200)),
    ];

    // when
    let metrics = AgentMetrics::of_parts(&starts, &found, TimeZone::UTC);

    // then
    let totals = metrics.totals;
    assert_eq!(
        (totals.samples, totals.cpu_avg, totals.cpu_max),
        (4, 30.0, 90.0)
    );
    assert_eq!((totals.memory_avg, totals.memory_max), (150, 300));
}

/// A CPU use that is not a number is refused, and the others stay from 0 to 100.
#[test]
fn a_sample_keeps_its_cpu_use_from_0_to_100() {
    // given
    let values = [f32::NAN, f32::INFINITY, -3.0, 104.5, 42.0];

    // when
    let parsed: Vec<Option<f32>> = values
        .iter()
        .map(|cpu| Sample::parse(*cpu, 1).ok().map(|sample| sample.cpu_percent))
        .collect();

    // then
    assert_eq!(parsed, vec![None, None, Some(0.0), Some(100.0), Some(42.0)]);
}

/// The slots count to the end of the period, or to now when it is earlier.
#[test]
fn the_slots_of_an_agent_count_to_the_end_of_the_period_or_to_now() {
    // given
    let mut past = query(1, monday() + Duration::hours(2));
    past.agent = Some(AppId::generate());
    let running = query(3, monday() + Duration::hours(2));

    // when
    let ends = (past.end(), running.end());

    // then
    assert_eq!(
        ends,
        (monday() + Duration::hours(1), monday() + Duration::hours(2))
    );
}

/// Returns a host with a protocol and features.
fn host(protocol: u32, features: &[&str]) -> Host {
    Host {
        version: "0.7.0".to_owned(),
        os: "linux".to_owned(),
        arch: "x86_64".to_owned(),
        hostname: "build-1".to_owned(),
        cpu_count: 4,
        memory_bytes: 0,
        capacity: 1,
        container_runtime: None,
        features: features.iter().map(|one| (*one).to_owned()).collect(),
        protocol,
        build: None,
        started_at: None,
    }
}

/// An agent older than the current protocol is outdated, and the current one is compatible.
#[test]
fn the_protocol_gives_the_compatibility() {
    // given
    let old = host(0, &[]);
    let current = host(CURRENT_AGENT_PROTOCOL, &[Host::CHECKOUT]);

    // when
    let fits = [old.compatibility(), current.compatibility()];

    // then
    assert_eq!(fits, [Compatibility::Outdated, Compatibility::Compatible]);
}

/// An old agent with no checkout must be updated, and a new one needs git.
#[test]
fn the_protocol_tells_why_the_checkout_is_missing() {
    // given
    let old = host(0, &[]);
    let no_git = host(CHECKOUT_PROTOCOL, &[]);
    let ready = host(CHECKOUT_PROTOCOL, &[Host::CHECKOUT]);

    // when
    let gaps = [
        old.checkout_gap(),
        no_git.checkout_gap(),
        ready.checkout_gap(),
    ];

    // then
    assert_eq!(
        gaps,
        [Some(Unfit::UpdateAgent), Some(Unfit::InstallGit), None]
    );
    assert_eq!(old.missing_features(), ["checkout"]);
    assert!(ready.missing_features().is_empty());
}

/// An empty build gives none, and a build of two lines is refused.
#[test]
fn a_build_is_one_line() {
    // given
    let mut empty = host(2, &[]);
    empty.build = Some("  ".to_owned());
    let mut two_lines = host(2, &[]);
    two_lines.build = Some("a\nb".to_owned());

    // when
    let parsed = Host::parse(empty);
    let refused = Host::parse(two_lines);

    // then
    assert_eq!(parsed.map(|host| host.build), Ok(None));
    assert!(matches!(refused, Err(("build", DomainError::NotOneLine))));
}
