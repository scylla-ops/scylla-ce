//! Unit tests of the activity feed: the kinds, their areas and the rows to record.

use std::collections::HashSet;

use super::*;
use crate::projects::jobs::JobId;

/// Each kind is `<subject>.<action>` in lower snake case, and no two kinds have the same key.
#[test]
fn each_kind_has_a_unique_key_of_a_subject_and_an_action() {
    // given
    let word = |part: &str| {
        !part.is_empty()
            && part
                .chars()
                .all(|letter| letter.is_ascii_lowercase() || letter == '_')
    };

    // when
    let keys: Vec<&str> = Kind::ALL.iter().map(|kind| kind.as_str()).collect();
    let unique: HashSet<&str> = keys.iter().copied().collect();

    // then
    assert_eq!(unique.len(), keys.len());
    for key in keys {
        let parts = key.split_once('.');
        assert!(
            matches!(parts, Some((subject, action)) if word(subject) && word(action)),
            "{key}"
        );
    }
}

/// An area of a project has a read permission on the project, and an area of the workspace has
/// none.
#[test]
fn only_the_areas_of_a_project_have_a_permission_on_the_project() {
    // given
    let of_project = [Area::Jobs, Area::Triggers, Area::Secrets];

    // when
    let with_access: Vec<Area> = Area::ALL
        .iter()
        .copied()
        .filter(|area| area.access().is_some())
        .collect();

    // then
    assert_eq!(with_access, of_project);
}

/// The kinds of a subject share the area of that subject.
#[test]
fn the_kinds_of_a_subject_share_one_area() {
    // given
    let expected = [
        ("job", Area::Jobs),
        ("trigger", Area::Triggers),
        ("secret", Area::Secrets),
        ("member", Area::Members),
        ("connection", Area::Integrations),
        ("agent", Area::Agents),
    ];

    // when
    let areas: Vec<(&str, Area)> = Kind::ALL
        .iter()
        .map(|kind| (kind.as_str().split('.').next().unwrap_or(""), kind.area()))
        .collect();

    // then
    for (subject, area) in areas {
        assert!(expected.contains(&(subject, area)), "{subject}: {area:?}");
    }
}

/// A row keeps the bits of the id of its object, and the size of its parameters.
#[test]
fn a_new_row_keeps_its_object_and_its_parameters() {
    // given
    let job = JobId::generate();
    let workspace = WorkspaceId::generate();

    // when
    let row = NewActivity::new(Kind::JobEnded, Place::Workspace(workspace))
        .subject("job", job)
        .param("status", "failed");

    // then
    let subject = row.subject.clone().expect("a subject");
    assert_eq!(subject.kind, "job");
    assert_eq!(subject.id.to_db(), job.to_db());
    assert_eq!(row.params_size(), "status".len() + "failed".len());
}
