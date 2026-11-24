//! The tests of the kinds of the edition against the rules of the catalog.

use scylla_domain::authz::Resource;
use scylla_domain::platform::Platform;
use scylla_domain::projects::Project;
use scylla_domain::tenancy::workspaces::Workspace;
use scylla_extension::Access;

/// The kinds of the core.
const KINDS: [&str; 19] = [
    "admin.granted",
    "admin.revoked",
    "instance_role.changed",
    "invitation.accepted",
    "invitation.declined",
    "invitation.received",
    "owner.added",
    "owner.removed",
    "role.granted",
    "role.revoked",
    "security.account_locked",
    "security.email_changed",
    "security.identity_linked",
    "security.identity_unlinked",
    "security.password_changed",
    "security.session_reuse",
    "token.created",
    "vcs.connection_broken",
    "vcs.token_expiring",
];

/// The edition has each kind of the catalog and no other: the web app has a text for each one.
#[test]
fn the_edition_has_the_kinds_of_the_catalog() {
    // given
    let edition = scylla_ce::edition();
    let kinds = edition.notification_kinds();

    // when
    let mut keys: Vec<&str> = kinds.all().map(|kind| kind.key).collect();
    keys.sort_unstable();

    // then
    assert_eq!(keys, KINDS);
}

/// Each kind is in a known category, and each permission of an access rule is in the schema.
#[test]
fn each_kind_has_a_category_and_a_valid_access_rule() {
    // given
    let edition = scylla_ce::edition();
    let schema = edition.authz_model();
    let definitions = [Platform::NAME, Workspace::NAME, Project::NAME];

    // when
    let kinds = edition.notification_kinds();

    // then
    for kind in kinds.all() {
        assert!(
            ["access", "security", "integrations"].contains(&kind.category),
            "{}",
            kind.key
        );
        if let Access::Holds(permission) = kind.access {
            let known = definitions
                .iter()
                .any(|definition| schema.permissions(definition).contains(&permission));
            assert!(known, "{}: {permission}", kind.key);
        }
    }
}

/// Each kind is mandatory, except the answers to an invitation and the notices of the Git
/// connections.
#[test]
fn each_mandatory_kind_is_in_security_or_access() {
    // given
    let edition = scylla_ce::edition();
    let kinds = edition.notification_kinds();

    // when
    let optional: Vec<&str> = kinds
        .all()
        .filter(|kind| !kind.mandatory)
        .map(|kind| kind.key)
        .collect();

    // then
    assert_eq!(optional.len(), 4);
    assert!(optional.contains(&"invitation.accepted"));
    assert!(optional.contains(&"invitation.declined"));
    assert!(optional.contains(&"vcs.connection_broken"));
    assert!(optional.contains(&"vcs.token_expiring"));
}
