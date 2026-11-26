//! Unit tests of the search, the health filter, the order and the pages of the project list.

use time::{Duration, OffsetDateTime};

use super::{ListProjects, ListedProject, ProjectHealth, ProjectOrder, ProjectPage, ProjectSearch};
use crate::identity::actors::ActorId;
use crate::projects::{Project, ProjectId};
use crate::tenancy::workspaces::WorkspaceId;
use crate::{DomainError, PageRequest, PageSize};

/// Makes a listed project with a name, its count of failing pipelines and its running state.
fn listed(name: &str, failing: u32, running: bool) -> ListedProject {
    ListedProject {
        project: Project {
            id: ProjectId::generate(),
            workspace_id: WorkspaceId::generate(),
            created_by: ActorId::generate(),
            name: name.to_owned(),
            description: String::new(),
            created_at: OffsetDateTime::UNIX_EPOCH,
            icon: None,
        },
        repository: None,
        pipelines: Vec::new(),
        failing,
        running,
        last_job_at: None,
    }
}

/// Makes the input of a list with a search text, an order and a health.
fn query(search: &str, order: ProjectOrder, health: ProjectHealth) -> ListProjects {
    ListProjects {
        search: ProjectSearch::parse(search.to_owned()).unwrap(),
        order,
        health,
        ..ListProjects::default()
    }
}

/// Returns the names of a page, in its order.
fn names(page: &ProjectPage) -> Vec<&str> {
    page.items
        .iter()
        .map(|project| project.name.as_str())
        .collect()
}

/// The search keeps a project when the text is in its name, its description, its repository or
/// the name of one of its pipelines, with no difference between upper and lower case.
#[test]
fn the_search_reads_the_names_the_repository_and_the_pipelines() {
    // given
    let mut described = listed("docs", 0, false);
    described.project.description = "Builds the WEB site".to_owned();
    let mut linked = listed("cloud", 0, false);
    linked.repository = Some("acme/cloud-web".to_owned());
    let mut piped = listed("api", 0, false);
    piped.pipelines = vec!["Web E2E".to_owned()];
    let all = vec![
        listed("Web", 0, false),
        described,
        linked,
        piped,
        listed("cli", 0, false),
    ];

    // when
    let page = ProjectPage::of(
        all,
        &query("  web ", ProjectOrder::Name, ProjectHealth::All),
    );

    // then
    assert_eq!(names(&page), ["api", "cloud", "docs", "Web"]);
    assert_eq!(page.counts.total, 4);
}

/// The counts read the projects of the search, and the health filter does not change them.
#[test]
fn the_counts_ignore_the_health_filter() {
    // given
    let mut red = listed("red", 2, false);
    red.pipelines = vec!["CI".to_owned(), "E2E".to_owned()];
    let mut busy = listed("busy", 0, true);
    busy.pipelines = vec!["Deploy".to_owned()];
    let all = vec![red, busy, listed("calm", 0, false)];

    // when
    let failed = ProjectPage::of(
        all.clone(),
        &query("", ProjectOrder::Name, ProjectHealth::Failed),
    );
    let running = ProjectPage::of(all, &query("", ProjectOrder::Name, ProjectHealth::Running));

    // then
    assert_eq!(names(&failed), ["red"]);
    assert_eq!(names(&running), ["busy"]);
    assert_eq!(failed.counts, running.counts);
    assert_eq!(
        (
            failed.counts.total,
            failed.counts.failed,
            failed.counts.running
        ),
        (3, 1, 1)
    );
    assert_eq!(failed.counts.pipelines, 3);
}

/// Failed first puts the failing projects first, then the running projects, then the others,
/// each group by name.
#[test]
fn failed_first_puts_the_failing_then_the_running_projects_first() {
    // given
    let all = vec![
        listed("b-calm", 0, false),
        listed("z-busy", 0, true),
        listed("y-red", 1, true),
        listed("a-calm", 0, false),
        listed("x-red", 3, false),
    ];

    // when
    let page = ProjectPage::of(
        all,
        &query("", ProjectOrder::FailedFirst, ProjectHealth::All),
    );

    // then
    assert_eq!(
        names(&page),
        ["x-red", "y-red", "z-busy", "a-calm", "b-calm"]
    );
}

/// Last job puts the project with the newest job first, and the projects with no job last.
#[test]
fn last_job_puts_the_newest_job_first() {
    // given
    let now = OffsetDateTime::UNIX_EPOCH + Duration::days(10);
    let mut old = listed("old", 0, false);
    old.last_job_at = Some(now - Duration::days(2));
    let mut new = listed("new", 0, false);
    new.last_job_at = Some(now);
    let all = vec![listed("none", 0, false), old, new];

    // when
    let page = ProjectPage::of(all, &query("", ProjectOrder::LastJob, ProjectHealth::All));

    // then
    assert_eq!(names(&page), ["new", "old", "none"]);
}

/// The position order keeps the order of creation, and a page starts after the project of the
/// request, with the key of the next page.
#[test]
fn the_pages_follow_the_order_of_the_request() {
    // given
    let mut all: Vec<_> = ["a", "b", "c", "d", "e"]
        .into_iter()
        .map(|name| listed(name, 0, false))
        .collect();
    all.sort_by_key(|listed| listed.project.id.to_bytes());
    let created: Vec<_> = all
        .iter()
        .map(|listed| listed.project.name.clone())
        .collect();
    all.reverse();
    let mut request = query("", ProjectOrder::Position, ProjectHealth::All);
    request.page = PageRequest {
        after: None,
        size: PageSize::parse(2).unwrap(),
    };

    // when
    let first = ProjectPage::of(all.clone(), &request);
    request.page.after = first.next_after;
    request.page.size = PageSize::parse(3).unwrap();
    let second = ProjectPage::of(all, &request);

    // then
    assert_eq!(names(&first), created[..2]);
    assert_eq!(first.next_after, Some(first.items[1].id));
    assert_eq!(names(&second), created[2..]);
    assert_eq!(second.next_after, None);
}

/// A search text of more than 100 characters is refused.
#[test]
fn a_long_search_text_is_refused() {
    // when
    let long = ProjectSearch::parse("x".repeat(101));
    let longest = ProjectSearch::parse(format!(" {} ", "x".repeat(100)));

    // then
    assert_eq!(long, Err(DomainError::TooLong { max: 100 }));
    assert!(longest.is_ok());
}
