//! Unit tests of the delivery counts: the totals, the buckets, the connections and the triggers.

use time::{Duration, OffsetDateTime};

use super::*;

/// Returns the counts of a key in a bucket, with the arrival of its newest delivery `at` seconds
/// after the start of the tests.
fn count<K>(key: K, bucket: usize, delivered: u32, failed: u32, at: i64) -> BucketCount<K> {
    BucketCount {
        key,
        bucket,
        figures: DeliveryFigures { delivered, failed },
        last_received_at: start() + Duration::seconds(at),
    }
}

/// Returns the start of the tests.
fn start() -> OffsetDateTime {
    OffsetDateTime::from_unix_timestamp(1_793_620_800).unwrap()
}

/// The totals and each bucket add the deliveries of the connections and of the triggers, and a
/// bucket with no delivery has counts at 0.
#[test]
fn the_buckets_add_the_connections_and_the_triggers() {
    // given
    let starts = [
        start(),
        start() + Duration::HOUR,
        start() + Duration::HOUR * 2,
    ];
    let (github, hook) = (VcsConnectionId::generate(), TriggerId::generate());
    let connections = [count(github, 1, 4, 1, 10), count(github, 3, 2, 0, 7300)];
    let triggers = [count(hook, 1, 3, 2, 20)];

    // when
    let stats = DeliveryStats::new(&starts, &connections, Vec::new(), &triggers, TimeZone::UTC);

    // then
    let figures = |delivered, failed| DeliveryFigures { delivered, failed };
    assert_eq!(stats.totals, figures(9, 3));
    let buckets: Vec<_> = stats.buckets.iter().map(|(_, figures)| *figures).collect();
    assert_eq!(buckets, [figures(7, 3), figures(0, 0), figures(2, 0)]);
    assert_eq!(stats.connections[0].figures, figures(6, 1));
}

/// The connections keep their order and get their newest delivery. The triggers come with the
/// most failed deliveries first, then the most deliveries, with their newest arrival.
#[test]
fn the_triggers_with_the_most_failed_deliveries_come_first() {
    // given
    let (first, second) = (VcsConnectionId::generate(), VcsConnectionId::generate());
    let connections = [count(second, 1, 1, 0, 5), count(first, 1, 8, 0, 9)];
    let last = LastDelivery {
        connection: first,
        received_at: start() + Duration::seconds(9),
        event: "push".to_owned(),
        repository: Some("octo/app".to_owned()),
        git_ref: Some("refs/heads/main".to_owned()),
    };
    let [busy, failing, quiet] = [(); 3].map(|()| TriggerId::generate());
    let triggers = [
        count(busy, 1, 9, 0, 30),
        count(failing, 1, 2, 2, 40),
        count(quiet, 1, 1, 0, 50),
        count(busy, 2, 1, 0, 3700),
    ];

    // when
    let stats = DeliveryStats::new(
        &[],
        &connections,
        vec![last.clone()],
        &triggers,
        TimeZone::UTC,
    );

    // then
    let order: Vec<_> = stats.connections.iter().map(|one| one.connection).collect();
    assert_eq!(order, [second, first]);
    assert_eq!(stats.connections[0].last, None);
    assert_eq!(stats.connections[1].last, Some(last));
    let order: Vec<_> = stats.triggers.iter().map(|one| one.trigger).collect();
    assert_eq!(order, [failing, busy, quiet]);
    assert_eq!(stats.triggers[1].figures.delivered, 10);
    assert_eq!(
        stats.triggers[1].last_received_at,
        start() + Duration::seconds(3700)
    );
    assert!(stats.buckets.is_empty());
}
