//! Unit tests of the order of the trigger summaries and of the groups of the fire counts.

use time::OffsetDateTime;

use super::*;
use crate::PageSize;

/// Returns the summary of a cron trigger with a name and a next fire.
fn summary(name: &str, next: Option<i64>, enabled: bool) -> TriggerSummary {
    let at = OffsetDateTime::from_unix_timestamp(1_800_000_000).unwrap();
    let trigger = Trigger {
        id: TriggerId::generate(),
        pipeline_id: Id::generate(),
        project_id: Id::generate(),
        workspace_id: Id::generate(),
        created_by: Id::generate(),
        name: name.to_owned(),
        kind: TriggerKind::Cron("0 * * * *".to_owned()),
        inputs: Vec::new(),
        enabled,
        next_fire_at: next.map(|minutes| at + time::Duration::minutes(minutes)),
        last_fired_at: None,
        last_result: None,
        created_at: at,
        updated_at: at,
        from_file: false,
    };
    TriggerSummary {
        trigger,
        pipeline_name: "build".to_owned(),
        project_name: "web".to_owned(),
        active: true,
        upcoming: Vec::new(),
        last_job_status: None,
        last_fire: None,
        failed_fires: 0,
        missed_times: 0,
    }
}

/// Returns the names of the summaries of a page.
fn names(page: &Page<TriggerSummary, TriggerId>) -> Vec<&str> {
    page.items
        .iter()
        .map(|summary| summary.trigger.name.as_str())
        .collect()
}

#[test]
fn the_soonest_next_fire_comes_first_and_a_stopped_trigger_last() {
    let all = vec![
        summary("later", Some(30), true),
        summary("stopped", Some(1), false),
        summary("soon", Some(5), true),
        summary("Beta", None, true),
    ];

    let page = TriggerOrder::NextFire.page(all, PageRequest::default());

    assert_eq!(names(&page), ["soon", "later", "Beta", "stopped"]);
    assert!(page.next_after.is_none());
}

#[test]
fn the_next_page_starts_after_the_trigger_of_the_request() {
    let all = vec![
        summary("c", None, true),
        summary("A", None, true),
        summary("b", None, true),
    ];
    let size = PageSize::parse(2).unwrap();

    let first = TriggerOrder::Name.page(all.clone(), PageRequest { after: None, size });
    let after = first.next_after;
    let second = TriggerOrder::Name.page(all, PageRequest { after, size });

    assert_eq!(names(&first), ["A", "b"]);
    assert_eq!(names(&second), ["c"]);
    assert!(second.next_after.is_none());
}

#[test]
fn the_groups_with_the_most_problems_come_first() {
    let group = |failed: u32, missed: u32, total: u32| FireGroup {
        key: FireGroupKey::Trigger(TriggerId::generate()),
        figures: FireFigures {
            total,
            failed,
            missed,
            ..FireFigures::default()
        },
    };
    let groups = vec![
        group(0, 0, 9),
        group(1, 1, 2),
        group(0, 1, 5),
        group(0, 0, 3),
    ];

    let top = FireStats::top(groups, 3);

    let totals: Vec<u32> = top.iter().map(|group| group.figures.total).collect();
    assert_eq!(totals, [2, 5, 9]);
}
