//! Unit tests of the checkout against a local bare repository: the commit, a ref, the depth, the
//! failures, the masks, and the run of a job after its checkout.

use std::path::Path;

use scylla_proto::agents::v1::{
    dispatch_node, Dispatch, DispatchCheckout, DispatchNode, EventKind,
};
use scylla_proto::common::v1::Id;
use scylla_proto::pipelines::v1::Exec;
use tokio::sync::{mpsc, watch};

use super::origin::{git, sent, Origin};
use super::{features, run, Failure, Git};
use crate::agent::Settings;
use crate::jobs::dag;

/// Runs a checkout in a work folder with a git, and returns its end with what the agent sent.
async fn check_out(
    checkout: &DispatchCheckout,
    git: Option<Git>,
    dir: &Path,
) -> (Result<(), Failure>, Vec<EventKind>, Vec<String>) {
    let (outbox, mut inbox) = mpsc::channel(1000);
    let (_stop, stopped) = watch::channel(false);
    let ended = run(checkout, git, dir, "job", &outbox, stopped).await;
    let (kinds, lines) = sent(&mut inbox, ".checkout");
    (ended, kinds, lines)
}

/// The hello tells the feature checkout when Git runs.
#[tokio::test]
async fn the_agent_tells_checkout_when_git_runs() {
    // when
    let features = features(Git::detect().await);

    // then
    assert_eq!(features, vec!["checkout".to_owned()]);
}

/// The checkout fetches only the commit at depth 1, detaches HEAD on it, and writes no header in
/// `.git/config`, with the git of the machine and with a git older than 2.31.
#[tokio::test]
async fn a_checkout_fetches_the_commit_at_depth_one() {
    for version in [Git::detect().await, Some(Git::parse("git version 2.30.2"))] {
        fetches_the_commit_at_depth_one(version).await;
    }
}

/// Checks the checkout of the commit at depth 1 with a git.
async fn fetches_the_commit_at_depth_one(version: Option<Git>) {
    // given
    let origin = Origin::new();
    let dir = origin.work();

    // when
    let (ended, kinds, _) = check_out(&origin.checkout(1, "", 1), version, &dir).await;

    // then
    assert!(ended.is_ok());
    assert_eq!(
        kinds,
        vec![EventKind::CheckoutStarted, EventKind::CheckoutCompleted]
    );
    assert_eq!(git(&dir, &["rev-parse", "HEAD"]), origin.shas[1]);
    assert_eq!(git(&dir, &["rev-list", "--count", "HEAD"]), "1");
    assert!(dir.join(".scylla/pipeline.toml").is_file());
    let config = std::fs::read_to_string(dir.join(".git/config")).unwrap();
    assert!(
        !config.contains("s3cr3t-token") && !config.contains("extraHeader"),
        "{config}"
    );
}

/// A commit that is on no branch comes from its ref, with the full history at depth 0.
#[tokio::test]
async fn a_checkout_fetches_the_ref_of_a_pull_request() {
    // given
    let origin = Origin::new();
    let dir = origin.work();

    // when
    let (ended, _, _) = check_out(
        &origin.checkout(2, "refs/pull/1/head", 0),
        Git::detect().await,
        &dir,
    )
    .await;

    // then
    assert!(ended.is_ok());
    assert_eq!(git(&dir, &["rev-parse", "HEAD"]), origin.shas[2]);
    assert_eq!(git(&dir, &["rev-list", "--count", "HEAD"]), "3");
}

/// A ref that is not at the commit of the job fails the checkout.
#[tokio::test]
async fn a_checkout_fails_when_the_ref_is_not_at_the_commit() {
    // given
    let origin = Origin::new();
    let dir = origin.work();

    // when
    let (ended, kinds, lines) = check_out(
        &origin.checkout(1, "refs/pull/1/head", 1),
        Git::detect().await,
        &dir,
    )
    .await;

    // then
    let Err(Failure::Failed(error)) = ended else {
        panic!("the checkout did not fail");
    };
    assert!(error.starts_with("HEAD is "), "{error}");
    assert_eq!(
        kinds,
        vec![EventKind::CheckoutStarted, EventKind::CheckoutFailed]
    );
    assert!(
        lines
            .iter()
            .any(|line| line.contains("the checkout failed: HEAD is ")),
        "{lines:?}"
    );
}

/// The log and the error of a failed checkout show `***` in place of the header.
#[tokio::test]
async fn a_failed_checkout_masks_the_header() {
    // given
    let origin = Origin::new();
    let dir = origin.work();
    let mut checkout = origin.checkout(1, "", 1);
    checkout.clone_url = origin.root.join("s3cr3t-token.git").display().to_string();

    // when
    let (ended, kinds, lines) = check_out(&checkout, Git::detect().await, &dir).await;

    // then
    let Err(Failure::Failed(error)) = ended else {
        panic!("the checkout did not fail");
    };
    assert_eq!(error, "git fetch ended with exit code 128");
    assert_eq!(
        kinds,
        vec![EventKind::CheckoutStarted, EventKind::CheckoutFailed]
    );
    assert!(
        lines.iter().any(|line| line.contains("***.git")),
        "{lines:?}"
    );
    assert!(
        lines.iter().all(|line| !line.contains("s3cr3t-token")),
        "{lines:?}"
    );
}

/// Runs a job of one node that prints the commit of HEAD, and returns the events and the lines
/// of the node that the agent sent.
async fn run_job(checkout: DispatchCheckout, root: &Path) -> (Vec<EventKind>, Vec<String>) {
    let node = DispatchNode {
        id: "build".to_owned(),
        step: Some(dispatch_node::Step::Exec(Exec {
            argv: ["git", "rev-parse", "HEAD"].map(str::to_owned).to_vec(),
        })),
        timeout_seconds: 60,
        ..DispatchNode::default()
    };
    let dispatch = Dispatch {
        job_id: Some(Id {
            value: "job".to_owned(),
        }),
        nodes: vec![node],
        checkout: Some(checkout),
        ..Dispatch::default()
    };
    let settings = Settings {
        workspace_root: root.join("jobs"),
        capacity: 1,
        container_runtime: None,
        keep_workspace: false,
    };
    let (outbox, mut inbox) = mpsc::channel(1000);
    let (_stop, stopped) = watch::channel(false);
    dag::run(dispatch, &settings, Git::detect().await, &outbox, stopped).await;
    sent(&mut inbox, "build")
}

/// The first node of a job starts after the checkout, in the folder of the code.
#[tokio::test]
async fn a_job_runs_its_nodes_on_the_code() {
    // given
    let origin = Origin::new();

    // when
    let (kinds, lines) = run_job(origin.checkout(1, "", 1), &origin.root).await;

    // then
    assert_eq!(
        kinds,
        vec![
            EventKind::JobStarted,
            EventKind::CheckoutStarted,
            EventKind::CheckoutCompleted,
            EventKind::NodeStarted,
            EventKind::NodeCompleted,
            EventKind::JobCompleted,
        ]
    );
    assert_eq!(lines, vec![origin.shas[1].clone()]);
}

/// A failed checkout fails the job and skips its nodes: no node starts.
#[tokio::test]
async fn a_failed_checkout_starts_no_node() {
    // given
    let origin = Origin::new();
    let mut checkout = origin.checkout(1, "", 1);
    checkout.sha = "not-a-sha".to_owned();

    // when
    let (kinds, _) = run_job(checkout, &origin.root).await;

    // then
    assert_eq!(
        kinds,
        vec![
            EventKind::JobStarted,
            EventKind::CheckoutStarted,
            EventKind::CheckoutFailed,
            EventKind::NodeSkipped,
            EventKind::JobFailed,
        ]
    );
}
