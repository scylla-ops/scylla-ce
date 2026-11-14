//! Unit tests of the output of the check runs: the map of the states, the title, the table of the
//! nodes, the errors of an invalid file, and the last log lines in the limit of GitHub.

use scylla_domain::integrations::reports::ReportedStatus;
use scylla_domain::projects::jobs::{CheckoutRun, Job, JobId, JobStatus, NodeRun, NodeStatus};
use scylla_domain::projects::pipeline_files::{CheckoutSettings, FileError, Location};
use scylla_domain::projects::pipelines::{Node, NodeId, Runtime, Step};
use scylla_domain::Id;
use time::{Duration, OffsetDateTime};

use super::super::output::{self, Tail, MAX_OUTPUT};
use super::super::states;

/// The link of the run in the tests.
const LINK: &str = "https://scylla.test/acme/projects/p/pipelines/q/jobs/j";

/// Makes a node.
fn node(id: &str) -> Node {
    Node {
        id: NodeId::parse(id.to_owned()).unwrap(),
        needs: Vec::new(),
        step: Step::Exec(vec!["true".to_owned()]),
        env: Vec::new(),
        timeout_seconds: 60,
        retries: 1,
        working_dir: None,
        runtime: Runtime::Process,
    }
}

/// Makes a failed job: the checkout passed in 3 seconds, `build` passed in 62 seconds, `test`
/// failed after 2 attempts in 58 seconds, and `deploy` was skipped.
fn failed_job() -> Job {
    let start = OffsetDateTime::UNIX_EPOCH;
    let nodes = vec![node("build"), node("test"), node("deploy")];
    let ended = |run: &mut NodeRun, status, attempt, seconds| {
        run.status = status;
        run.attempt = attempt;
        run.started_at = Some(start);
        run.finished_at = Some(start + Duration::seconds(seconds));
    };
    let mut runs: Vec<NodeRun> = nodes.iter().map(NodeRun::pending).collect();
    ended(&mut runs[0], NodeStatus::Completed, 1, 62);
    ended(&mut runs[1], NodeStatus::Failed, 2, 58);
    runs[2].status = NodeStatus::Skipped;
    let mut checkout = CheckoutRun::pending(CheckoutSettings::default());
    checkout.status = NodeStatus::Completed;
    checkout.started_at = Some(start);
    checkout.finished_at = Some(start + Duration::seconds(3));
    Job {
        id: JobId::generate(),
        pipeline_id: Id::generate(),
        project_id: Id::generate(),
        workspace_id: Id::generate(),
        created_by: Id::generate(),
        rerun_of: None,
        trigger_id: None,
        status: JobStatus::Failed,
        nodes,
        runs,
        agent_id: None,
        error: None,
        version: 1,
        created_at: start,
        started_at: Some(start),
        finished_at: Some(start + Duration::seconds(123)),
        source: None,
        approval: None,
        pipeline_file: None,
        file_errors: Vec::new(),
        checkout: Some(checkout),
    }
}

/// Each state of a run has the status and the conclusion of a check run of the map.
#[test]
fn each_state_of_a_run_has_a_check_run_state() {
    // given
    let all = ReportedStatus::ALL;

    // when
    let map: Vec<_> = all
        .iter()
        .map(|status| states::check_run(*status))
        .collect();

    // then
    let done = |conclusion| ("completed", Some(conclusion));
    let expected = [
        ("queued", None),
        ("queued", None),
        ("in_progress", None),
        done("success"),
        done("failure"),
        done("cancelled"),
        done("failure"),
        done("failure"),
    ];
    assert_eq!(map, expected);
}

/// A failed run has a title with the rows that passed, the link of the run, a table of the
/// checkout and the nodes with their state, duration and attempts, and the last lines of the
/// failed node.
#[test]
fn a_failed_run_shows_its_nodes_and_the_lines_of_the_failed_node() {
    // given
    let job = failed_job();
    let lines = vec![
        "running 4 tests".to_owned(),
        "test login ... FAILED".to_owned(),
    ];

    // when
    let failed = output::failed_node(&job).map(str::to_owned);
    let tail = Tail {
        node: failed.clone().unwrap(),
        lines,
    };
    let output = output::of("build", ReportedStatus::Failed, &job, LINK, Some(&tail));

    // then
    assert_eq!(failed.as_deref(), Some("test"));
    assert_eq!(output.title, "build failed: 2/4 nodes passed");
    assert!(output
        .summary
        .starts_with(&format!("[View the run in Scylla]({LINK})")));
    let rows = [
        "| `checkout` | ✓ Passed | 3s |  |",
        "| `build` | ✓ Passed | 1m 2s | 1 |",
        "| `test` | ✗ Failed | 58s | 2 |",
        "| `deploy` | Skipped |  |  |",
    ];
    for row in rows {
        assert!(output.summary.contains(row), "{row} in {}", output.summary);
    }
    let text = output.text.unwrap();
    assert!(text.starts_with("Last 2 lines of `test`:"), "{text}");
    assert!(text.contains("```text\nrunning 4 tests\ntest login ... FAILED\n```"));
}

/// A failed checkout shows the lines of the checkout, and a run with no line has no text.
#[test]
fn a_failed_checkout_shows_its_lines() {
    // given
    let mut job = failed_job();
    if let Some(checkout) = job.checkout.as_mut() {
        checkout.status = NodeStatus::Failed;
    }
    let tail = Tail {
        node: CheckoutRun::NODE.to_owned(),
        lines: vec!["fatal: repository not found".to_owned()],
    };
    let empty = Tail {
        node: "test".to_owned(),
        lines: Vec::new(),
    };

    // when
    let failed = output::failed_node(&job).map(str::to_owned);
    let output = output::of("build", ReportedStatus::Failed, &job, LINK, Some(&tail));
    let silent = output::of("build", ReportedStatus::Failed, &job, LINK, Some(&empty));

    // then
    assert_eq!(failed.as_deref(), Some(CheckoutRun::NODE));
    let text = output.text.unwrap();
    assert!(text.starts_with("Last line of `checkout`:"), "{text}");
    assert!(silent.text.is_none());
}

/// The text keeps the last lines in 65,535 bytes, and its fence is longer than each run of
/// backticks in the lines.
#[test]
fn the_text_stays_in_the_limit_of_github() {
    // given
    let job = failed_job();
    let mut lines: Vec<String> = (0..50)
        .map(|n| format!("{n:02} {}", "é".repeat(1000)))
        .collect();
    lines.push("```` four backticks".to_owned());
    let tail = Tail {
        node: "test".to_owned(),
        lines,
    };

    // when
    let output = output::of("build", ReportedStatus::Failed, &job, LINK, Some(&tail));

    // then
    let text = output.text.unwrap();
    assert!(text.len() <= MAX_OUTPUT, "{}", text.len());
    assert!(!text.contains("\n00 "));
    assert!(text.contains("\n49 "));
    assert!(text.contains("`````text\n"));
    assert!(text.ends_with("```` four backticks\n`````\n"));
}

/// An invalid file gives its title and its errors with their lines, and no table.
#[test]
fn an_invalid_file_shows_its_errors() {
    // given
    let mut job = failed_job();
    job.runs.clear();
    job.checkout = None;
    let mut error = FileError::of_file(".scylla/pipelines/build.toml", "\"tset\" is not a node");
    error.location = Some(Location {
        line: 14,
        column: 5,
    });
    job.file_errors = vec![error];

    // when
    let output = output::of("build", ReportedStatus::InvalidFile, &job, LINK, None);

    // then
    assert_eq!(output.title, "Invalid pipeline file");
    let error = ".scylla/pipelines/build.toml:14:5: \"tset\" is not a node";
    assert!(output.summary.contains(error), "{}", output.summary);
    assert!(!output.summary.contains("| Node |"));
    assert!(output.text.is_none());
}

/// A run that waits for an approval, a queued run and a running run have their titles.
#[test]
fn an_open_run_has_its_title() {
    // given
    let job = failed_job();

    // when
    let title = |status| output::of("build", status, &job, LINK, None).title;

    // then
    assert_eq!(
        title(ReportedStatus::AwaitingApproval),
        "Waiting for approval"
    );
    assert_eq!(title(ReportedStatus::Pending), "Queued");
    assert_eq!(
        title(ReportedStatus::Running),
        "build is running: 2/4 nodes passed"
    );
    assert_eq!(
        title(ReportedStatus::Completed),
        "build passed: 2/4 nodes passed"
    );
}
