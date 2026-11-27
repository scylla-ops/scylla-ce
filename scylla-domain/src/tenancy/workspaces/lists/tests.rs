//! Unit tests of the lists of workspaces.

use super::{WorkspaceRoles, WorkspaceSearch};
use crate::identity::actors::ActorId;
use crate::DomainError;

/// Returns the roles of a workspace with one owner, two roles of the schema and one row.
fn roles(owner: ActorId, admin: ActorId, other: ActorId) -> WorkspaceRoles {
    WorkspaceRoles {
        owners: vec![owner],
        relations: vec![(admin, "viewer".to_owned()), (admin, "admin".to_owned())],
        rows: vec![(other, "Auditor".to_owned()), (owner, "Auditor".to_owned())],
    }
}

/// A search text is trimmed and kept in lower case, and a text over the limit fails.
#[test]
fn a_search_is_trimmed_and_in_lower_case() {
    // given
    let long = "a".repeat(WorkspaceSearch::MAX_LEN + 1);

    // when
    let search = WorkspaceSearch::parse("  Sébastien ".to_owned()).unwrap();
    let too_long = WorkspaceSearch::parse(long);

    // then
    assert_eq!(search.as_str(), "sébastien");
    assert!(matches!(too_long, Err(DomainError::TooLong { max: 100 })));
}

/// Each actor counts one time, whatever the number of its roles.
#[test]
fn each_member_counts_one_time() {
    // given
    let (owner, admin, other) = (
        ActorId::generate(),
        ActorId::generate(),
        ActorId::generate(),
    );

    // when
    let count = roles(owner, admin, other).member_count();

    // then
    assert_eq!(count, 3);
}

/// The owner comes first, then the strongest role of the schema, then a row. No role gives
/// `None`.
#[test]
fn the_role_of_an_actor_is_its_strongest_role() {
    // given
    let (owner, admin, other) = (
        ActorId::generate(),
        ActorId::generate(),
        ActorId::generate(),
    );
    let roles = roles(owner, admin, other);

    // when
    let found = [owner, admin, other, ActorId::generate()].map(|actor| roles.role_of(actor));

    // then
    assert_eq!(
        found,
        [
            Some("owner".to_owned()),
            Some("admin".to_owned()),
            Some("Auditor".to_owned()),
            None
        ]
    );
}
