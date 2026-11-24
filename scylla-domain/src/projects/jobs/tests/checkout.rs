//! Unit tests of the checkout of a job: its events, its end, the jobs that get one, and the life
//! of its token.

use time::{Duration, OffsetDateTime};

use super::{job, source};
use crate::identity::actors::ActorId;
use crate::integrations::checkout::token_life;
use crate::projects::jobs::{CheckoutRun, JobEvent, JobStatus, NodeStatus, RerunOrigin, StartJob};
use crate::projects::pipeline_files::CheckoutSettings;
use crate::DomainError;

/// Returns a running job with a checkout that waits.
fn running() -> crate::projects::jobs::Job {
    let mut job = job();
    job.checkout = Some(CheckoutRun::pending(CheckoutSettings::default()));
    job.apply(&JobEvent::JobStarted, OffsetDateTime::now_utc())
        .unwrap();
    job
}

/// The checkout of a running job starts, then completes, with its times.
#[test]
fn a_checkout_runs_to_completion() {
    // given
    let mut job = running();
    let now = OffsetDateTime::now_utc();

    // when
    job.apply(&JobEvent::CheckoutStarted, now).unwrap();
    job.apply(&JobEvent::CheckoutCompleted, now).unwrap();

    // then
    let checkout = job.checkout.unwrap();
    assert_eq!(checkout.status, NodeStatus::Completed);
    assert_eq!(
        (checkout.started_at, checkout.finished_at),
        (Some(now), Some(now))
    );
    assert_eq!(job.status, JobStatus::Running);
}

/// A failed checkout keeps its error, and the end of the job does not change it.
#[test]
fn a_failed_checkout_keeps_its_error() {
    // given
    let mut job = running();
    let now = OffsetDateTime::now_utc();
    job.apply(&JobEvent::CheckoutStarted, now).unwrap();

    // when
    let failed = JobEvent::CheckoutFailed("no such commit".to_owned());
    job.apply(&failed, now).unwrap();
    job.apply(&JobEvent::JobFailed("the checkout failed".to_owned()), now)
        .unwrap();

    // then
    let checkout = job.checkout.unwrap();
    assert_eq!(checkout.status, NodeStatus::Failed);
    assert_eq!(checkout.error.as_deref(), Some("no such commit"));
    assert_eq!(job.status, JobStatus::Failed);
}

/// An event of the checkout must fit its state, and a job with no checkout refuses each one.
#[test]
fn a_checkout_event_must_fit_the_state() {
    // given
    let now = OffsetDateTime::now_utc();
    let mut pending = job();
    pending.checkout = Some(CheckoutRun::pending(CheckoutSettings::default()));
    let mut none = job();
    none.apply(&JobEvent::JobStarted, now).unwrap();
    let mut running = running();

    // when
    let before_start = pending.apply(&JobEvent::CheckoutStarted, now);
    let no_checkout = none.apply(&JobEvent::CheckoutStarted, now);
    let early_end = running.apply(&JobEvent::CheckoutCompleted, now);

    // then
    assert_eq!(before_start, Err(DomainError::NotAllowed));
    assert_eq!(no_checkout, Err(DomainError::NotAllowed));
    assert_eq!(early_end, Err(DomainError::NotAllowed));
    assert_eq!(running.checkout.unwrap().status, NodeStatus::Pending);
}

/// The end of a job cancels a checkout that did not end.
#[test]
fn an_end_cancels_an_open_checkout() {
    // given
    let mut job = running();
    let now = OffsetDateTime::now_utc();
    job.apply(&JobEvent::CheckoutStarted, now).unwrap();

    // when
    job.end(JobStatus::Cancelled, None, now);

    // then
    let checkout = job.checkout.unwrap();
    assert_eq!(checkout.status, NodeStatus::Cancelled);
    assert_eq!(checkout.finished_at, Some(now));
}

/// Only a job with a source gets a checkout, and only when the checkout of its pipeline is on. A
/// new run keeps the checkout of the job.
#[test]
fn a_job_with_a_source_gets_the_checkout_of_its_pipeline() {
    // given
    let actor = ActorId::generate();
    let deep = CheckoutSettings {
        depth: 0,
        lfs: true,
        ..CheckoutSettings::default()
    };
    let off = CheckoutSettings {
        enabled: false,
        ..CheckoutSettings::default()
    };
    let start = StartJob {
        checkout: deep.clone(),
        ..StartJob::rerun(job(), RerunOrigin::Person)
    };
    let with_source = StartJob {
        source: Some(source(None)),
        ..start.clone()
    };

    // when
    let plain = start.clone().into_new(actor);
    let checked = with_source.clone().into_new(actor);
    let disabled = StartJob {
        checkout: off,
        ..with_source
    }
    .into_new(actor);
    let mut old = job();
    old.source = Some(source(None));
    old.checkout = Some(CheckoutRun::pending(deep.clone()));
    let again = StartJob::rerun(old.clone(), RerunOrigin::Person).into_new(actor);
    old.checkout = None;
    let again_off = StartJob::rerun(old, RerunOrigin::Person).into_new(actor);

    // then
    assert_eq!(plain.checkout, None);
    assert_eq!(checked.checkout, Some(deep.clone()));
    assert_eq!(disabled.checkout, None);
    assert_eq!(again.checkout, Some(deep));
    assert_eq!(again_off.checkout, None);
}

/// A token of a checkout lives for the checkout and for each attempt of each node, at most 30
/// days.
#[test]
fn a_checkout_token_lives_for_the_longest_run_of_the_job() {
    // given
    let mut long = super::node("long");
    long.timeout_seconds = 604_800;
    long.retries = 10;

    // when
    let none = token_life(&[]);
    let two = token_life(&[super::node("build"), super::node("test")]);
    let capped = token_life(&[long]);

    // then
    assert_eq!(none, Duration::hours(1));
    assert_eq!(two, Duration::hours(1) + Duration::seconds(240));
    assert_eq!(capped, Duration::days(30));
}
