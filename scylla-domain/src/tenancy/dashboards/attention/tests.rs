//! Unit tests of the open items: the page, the counts, the key of a page and the node of a job.

use time::{Duration, OffsetDateTime};

use super::{
    AttentionCursor, AttentionItem, AttentionKind, AttentionList, AttentionQuery, AttentionReach,
    ItemAgent, ItemNode, Reach, Subject,
};
use crate::identity::apps::AppId;
use crate::projects::jobs::{JobStatus, NodeRun, NodeStatus};
use crate::projects::pipelines::{Node, NodeId, Runtime, Step};
use crate::projects::ProjectId;
use crate::{Id, PageRequest, PageSize};

/// 2026-11-02 00:00 UTC.
fn start() -> OffsetDateTime {
    OffsetDateTime::from_unix_timestamp(1_793_577_600).unwrap()
}

/// Makes an item of an agent of a kind, `minutes` after the start.
fn item(kind: AttentionKind, minutes: i64) -> AttentionItem {
    let at = start() + Duration::minutes(minutes);
    AttentionItem {
        kind,
        at,
        count: 1,
        is_new: false,
        place: None,
        subject: Subject::Agent(ItemAgent {
            id: AppId::generate(),
            name: "runner".to_owned(),
            last_seen_at: at,
        }),
    }
}

/// Makes a query of a caller that reads each kind, with a page size.
fn query(size: i32) -> AttentionQuery {
    AttentionQuery {
        workspace: Id::generate(),
        projects: Vec::new(),
        kinds: Vec::new(),
        since: Some(start() + Duration::minutes(30)),
        page: PageRequest {
            after: None,
            size: PageSize::parse(size).unwrap(),
        },
        reach: AttentionReach {
            jobs: Reach::Workspace,
            fires: Reach::Workspace,
            approvals: Reach::Workspace,
            agents: Reach::Workspace,
        },
    }
}

/// Makes a node with two retries.
fn node(id: &str) -> Node {
    Node {
        id: NodeId::parse(id.to_owned()).unwrap(),
        needs: Vec::new(),
        step: Step::Exec(vec!["true".to_owned()]),
        env: Vec::new(),
        timeout_seconds: 1_800,
        retries: 2,
        working_dir: None,
        runtime: Runtime::Process,
    }
}

/// Makes the state of a node at an attempt.
fn run(node: &Node, status: NodeStatus, attempt: u32) -> NodeRun {
    NodeRun {
        status,
        attempt,
        started_at: (attempt > 0).then(start),
        error: (status == NodeStatus::Failed).then(|| "exit 1".to_owned()),
        ..NodeRun::pending(node)
    }
}

/// The items come newest first, the new items are after the last visit, and the counts hold
/// each kind that the caller reads, also with no item.
#[test]
fn a_page_sorts_marks_and_counts_the_items() {
    // given
    let items = vec![
        item(AttentionKind::JobFailed, 10),
        item(AttentionKind::AgentOffline, 50),
        item(AttentionKind::JobFailed, 40),
    ];

    // when
    let list = AttentionList::page(items, &query(0), None);

    // then
    let minutes: Vec<i64> = list
        .items
        .iter()
        .map(|item| (item.at - start()).whole_minutes())
        .collect();
    assert_eq!(minutes, [50, 40, 10]);
    let new: Vec<bool> = list.items.iter().map(|item| item.is_new).collect();
    assert_eq!(new, [true, true, false]);
    assert_eq!((list.open, list.new, list.counts.len()), (3, 2, 6));
    let failed = list.counts[0];
    assert_eq!(
        (failed.kind, failed.open, failed.new),
        (AttentionKind::JobFailed, 2, 1)
    );
    assert_eq!(list.counts[2].open, 0);
}

/// The key of the last item of a page gives the next page, with no item two times.
#[test]
fn a_key_gives_the_next_page() {
    // given
    let items: Vec<_> = (0..5).map(|n| item(AttentionKind::JobFailed, n)).collect();
    let first = AttentionList::page(items.clone(), &query(2), None);
    let token = first.next_after.unwrap().text();

    // when
    let mut next = query(2);
    next.page.after = AttentionCursor::parse(&token);
    let second = AttentionList::page(items.clone(), &next, None);
    next.page.after = second.next_after;
    let third = AttentionList::page(items, &next, None);

    // then
    let minutes = |list: &AttentionList| -> Vec<i64> {
        let at = |item: &AttentionItem| (item.at - start()).whole_minutes();
        list.items.iter().map(at).collect()
    };
    assert_eq!(minutes(&first), [4, 3]);
    assert_eq!(minutes(&second), [2, 1]);
    assert_eq!((minutes(&third), third.next_after), (vec![0], None));
    assert_eq!(third.open, 5);
    assert!(AttentionCursor::parse("not a key").is_none());
}

/// A kind that the caller does not read is not in the counts, and a filter of projects asks for
/// no agent.
#[test]
fn the_counts_hold_the_kinds_that_the_caller_reads() {
    // given
    let mut some = query(0);
    some.reach.fires = Reach::of(Some(Vec::new()));
    some.reach.approvals = Reach::of(Some(vec![ProjectId::generate()]));
    let mut filtered = some.clone();
    filtered.projects = vec![ProjectId::generate()];

    // when
    let kinds = some.open_kinds();
    let in_projects = filtered.open_kinds();

    // then
    use AttentionKind::*;
    assert_eq!(
        kinds,
        [JobFailed, JobOrphaned, AgentOffline, ApprovalWaiting]
    );
    assert_eq!(in_projects, [JobFailed, JobOrphaned, ApprovalWaiting]);
}

/// The node of a failed job is the node that failed. The node of an orphaned job is the node
/// that ran when the agent went away.
#[test]
fn the_node_of_a_job_is_the_node_that_stopped_it() {
    // given
    let nodes = vec![node("build"), node("test")];
    let failed = vec![
        run(&nodes[0], NodeStatus::Completed, 1),
        run(&nodes[1], NodeStatus::Failed, 3),
    ];
    let orphaned = vec![
        run(&nodes[0], NodeStatus::Cancelled, 2),
        run(&nodes[1], NodeStatus::Cancelled, 0),
    ];

    // when
    let of_failed = ItemNode::of(JobStatus::Failed, &nodes, &failed).unwrap();
    let of_orphaned = ItemNode::of(JobStatus::Orphaned, &nodes, &orphaned).unwrap();
    let none = ItemNode::of(JobStatus::Failed, &nodes, &orphaned);

    // then
    assert_eq!(of_failed.id.as_str(), "test");
    assert_eq!(of_failed.error.as_deref(), Some("exit 1"));
    let attempts = (
        of_failed.attempt,
        of_failed.max_attempts,
        of_failed.timeout_seconds,
    );
    assert_eq!(attempts, (3, 3, 1_800));
    assert_eq!((of_orphaned.id.as_str(), of_orphaned.attempt), ("build", 2));
    assert!(none.is_none());
}
