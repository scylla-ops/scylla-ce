//! Unit tests of the order and of the pages of the summaries of the pipelines.

use time::Duration;

use super::{PipelineSummary, SummaryOrder, SummaryPage};
use crate::projects::jobs::JobStatus;
use crate::projects::pipelines::PipelineId;
use crate::projects::ProjectId;
use crate::{PageRequest, PageSize};

/// Makes a summary with a name, the state of its last ended job, its counts and its median.
fn summary(
    name: &str,
    last_ended: Option<JobStatus>,
    (completed, failed): (u32, u32),
    median: Option<i64>,
) -> PipelineSummary {
    PipelineSummary {
        pipeline_id: PipelineId::generate(),
        name: name.to_owned(),
        project_id: ProjectId::generate(),
        project_name: "app".to_owned(),
        recent: Vec::new(),
        completed,
        failed,
        median: median.map(Duration::seconds),
        last_ended,
        active: false,
        trigger_kinds: Vec::new(),
        next_fire: None,
    }
}

/// Returns the names of a page, in its order.
fn names(page: &SummaryPage) -> Vec<&str> {
    page.items
        .iter()
        .map(|summary| summary.name.as_str())
        .collect()
}

/// Makes the request of a page of a size.
fn first(size: i32) -> PageRequest<PipelineId> {
    PageRequest {
        after: None,
        size: PageSize::parse(size).unwrap(),
    }
}

/// The failing pipelines come first, then the pipelines with no ended job, then the green ones.
/// In a group, the lowest success rate comes first, then the name.
#[test]
fn failing_first_puts_the_red_pipelines_first() {
    // given
    let all = vec![
        summary("green", Some(JobStatus::Completed), (3, 1), None),
        summary("new", None, (0, 0), None),
        summary("cancelled", Some(JobStatus::Cancelled), (4, 0), None),
        summary("broken", Some(JobStatus::Failed), (1, 3), None),
        summary("Better", Some(JobStatus::Orphaned), (3, 1), None),
        summary("alpha", Some(JobStatus::Completed), (3, 1), None),
    ];

    // when
    let page = SummaryPage::of(all, SummaryOrder::FailingFirst, first(0));

    // then
    assert_eq!(
        names(&page),
        ["broken", "Better", "cancelled", "new", "alpha", "green"]
    );
    assert_eq!((page.total, page.green), (6, 2));
    assert_eq!(page.next_after, None);
}

/// The longest median comes first, and a pipeline with no median comes last.
#[test]
fn slowest_puts_the_longest_median_first() {
    // given
    let all = vec![
        summary("none", None, (0, 0), None),
        summary("short", None, (1, 0), Some(10)),
        summary("long", None, (1, 0), Some(300)),
        summary("middle", None, (1, 0), Some(60)),
    ];

    // when
    let page = SummaryPage::of(all, SummaryOrder::Slowest, first(0));

    // then
    assert_eq!(names(&page), ["long", "middle", "short", "none"]);
}

/// The order of the names has no difference between upper and lower case.
#[test]
fn name_ignores_the_case() {
    // given
    let all = vec![
        summary("deploy", None, (0, 0), None),
        summary("Build", None, (0, 0), None),
        summary("api", None, (0, 0), None),
    ];

    // when
    let page = SummaryPage::of(all, SummaryOrder::Name, first(0));

    // then
    assert_eq!(names(&page), ["api", "Build", "deploy"]);
}

/// A page starts after the pipeline of the request. A pipeline that is not in the list gives an
/// empty page.
#[test]
fn a_page_starts_after_its_pipeline() {
    // given
    let all: Vec<_> = ["a", "b", "c"]
        .into_iter()
        .map(|name| summary(name, None, (0, 0), None))
        .collect();
    let page_one = SummaryPage::of(all.clone(), SummaryOrder::Name, first(2));
    let after = page_one.next_after;

    // when
    let next = PageRequest {
        after,
        size: PageSize::parse(2).unwrap(),
    };
    let page_two = SummaryPage::of(all.clone(), SummaryOrder::Name, next);
    let unknown = PageRequest {
        after: Some(PipelineId::generate()),
        size: PageSize::parse(2).unwrap(),
    };
    let lost = SummaryPage::of(all, SummaryOrder::Name, unknown);

    // then
    assert_eq!(names(&page_one), ["a", "b"]);
    assert_eq!(after, Some(page_one.items[1].pipeline_id));
    assert_eq!(names(&page_two), ["c"]);
    assert_eq!(page_two.next_after, None);
    assert_eq!(page_two.total, 3);
    assert!(lost.items.is_empty());
    assert_eq!(lost.next_after, None);
}
