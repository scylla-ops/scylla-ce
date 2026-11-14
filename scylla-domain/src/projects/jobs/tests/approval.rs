//! Unit tests of the approval of a job from a fork: an approval, a rejection, and a job that does
//! not wait.

use time::OffsetDateTime;

use super::job;
use crate::identity::actors::ActorId;
use crate::integrations::sources::ApprovalState;
use crate::projects::jobs::{JobApproval, JobStatus, NodeStatus};
use crate::DomainError;

/// Returns a job from a fork that waits for an approval.
fn waiting() -> crate::projects::jobs::Job {
    let mut job = job();
    job.status = JobStatus::AwaitingApproval;
    job.approval = Some(JobApproval::waiting());
    job
}

/// An approval makes the job pending, with the actor and the time of the approval.
#[test]
fn an_approval_makes_the_job_pending() {
    // given
    let mut job = waiting();
    let (editor, now) = (ActorId::generate(), OffsetDateTime::now_utc());

    // when
    job.approve(editor, now).unwrap();

    // then
    assert_eq!(job.status, JobStatus::Pending);
    let approval = job.approval.unwrap();
    assert_eq!(approval.state, ApprovalState::Approved);
    assert_eq!(
        (approval.decided_by, approval.decided_at),
        (Some(editor), Some(now))
    );
    assert!(job.finished_at.is_none() && job.error.is_none());
}

/// A rejection cancels the job and each node, with the name of the actor in the error.
#[test]
fn a_rejection_cancels_the_job() {
    // given
    let mut job = waiting();
    let (editor, now) = (ActorId::generate(), OffsetDateTime::now_utc());

    // when
    job.reject(editor, "Lea", now).unwrap();

    // then
    assert_eq!(job.status, JobStatus::Cancelled);
    assert_eq!(job.error.as_deref(), Some("Rejected by Lea"));
    assert_eq!(job.finished_at, Some(now));
    let approval = job.approval.unwrap();
    assert_eq!(approval.state, ApprovalState::Rejected);
    assert_eq!(approval.decided_by, Some(editor));
    assert!(job
        .runs
        .iter()
        .all(|run| run.status == NodeStatus::Cancelled));
}

/// Only a job that waits takes an approval or a rejection. Another job does not change.
#[test]
fn only_a_waiting_job_takes_a_decision() {
    // given
    let mut pending = job();
    let mut decided = waiting();
    let (editor, now) = (ActorId::generate(), OffsetDateTime::now_utc());
    decided.approve(editor, now).unwrap();

    // when
    let approved = pending.approve(editor, now);
    let rejected = pending.reject(editor, "Lea", now);
    let again = decided.approve(editor, now);
    let late = decided.reject(editor, "Lea", now);

    // then
    assert_eq!(approved, Err(DomainError::NotAllowed));
    assert_eq!(rejected, Err(DomainError::NotAllowed));
    assert_eq!(again, Err(DomainError::NotAllowed));
    assert_eq!(late, Err(DomainError::NotAllowed));
    assert_eq!(pending.status, JobStatus::Pending);
    assert!(pending.approval.is_none());
    assert_eq!(decided.status, JobStatus::Pending);
    let state = decided.approval.map(|approval| approval.state);
    assert_eq!(state, Some(ApprovalState::Approved));
}
