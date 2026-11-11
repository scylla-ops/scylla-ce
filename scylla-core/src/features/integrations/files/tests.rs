//! Unit tests of the memory of the pipeline files, which the API shows only after 30 seconds.

use std::sync::Arc;
use std::time::Duration;

use scylla_domain::projects::ProjectId;

use super::cache;

/// The first check of a project is due, and a second check in the window is not. A sync records
/// its head and a check.
#[test]
fn a_check_of_the_head_is_due_once_in_a_window() {
    // given
    let project = ProjectId::generate();
    let window = Duration::from_secs(30);

    // when
    let first = cache::due(project, window);
    let second = cache::due(project, window);
    let none = cache::due(project, Duration::ZERO);
    cache::saw(project, "abc");

    // then
    assert!(first && !second && none);
    assert!(!cache::due(project, window));
    assert_eq!(cache::head(project).as_deref(), Some("abc"));
}

/// A blob of a project stays for this project only, and a blob larger than the memory is not
/// kept.
#[test]
fn a_blob_stays_for_its_project() {
    // given
    let (project, other) = (ProjectId::generate(), ProjectId::generate());
    let bytes: Arc<[u8]> = b"version = 1".as_slice().into();
    let large: Arc<[u8]> = vec![0; 33 << 20].into();

    // when
    cache::keep(project, "aaa", Arc::clone(&bytes));
    cache::keep(project, "large", large);

    // then
    assert_eq!(cache::blob(project, "aaa").as_deref(), Some(&bytes[..]));
    assert!(cache::blob(other, "aaa").is_none());
    assert!(cache::blob(project, "large").is_none());
}
