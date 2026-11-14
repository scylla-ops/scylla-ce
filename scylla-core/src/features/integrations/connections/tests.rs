//! Unit tests of the pure rules of the connections.

use std::collections::BTreeMap;

use scylla_domain::integrations::connections::ConnectionState;
use scylla_domain::integrations::links::GitLabRole;
use time::{Duration, OffsetDateTime};

use super::checks;

/// Makes the permissions of an installation from pairs.
fn held(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(name, access)| ((*name).to_owned(), (*access).to_owned()))
        .collect()
}

/// A permission is missing when the installation has no access or a lower access than Scylla
/// asks for. A higher access is enough.
#[test]
fn missing_permissions_compares_the_access() {
    // given
    let all = held(&[
        ("checks", "write"),
        ("contents", "write"),
        ("pull_requests", "read"),
        ("metadata", "read"),
        ("administration", "admin"),
    ]);
    let old = held(&[
        ("checks", "read"),
        ("contents", "read"),
        ("metadata", "read"),
    ]);

    // when
    let none = checks::missing_permissions(&all);
    let some = checks::missing_permissions(&old);

    // then
    assert!(none.is_empty());
    assert_eq!(some, ["checks", "pull_requests", "administration"]);
}

/// Scylla does not ask for `statuses`. An installation that still has it misses nothing, and
/// its connection is active.
#[test]
fn missing_permissions_ignores_the_statuses_of_an_old_app() {
    // given
    let old_app = held(&[
        ("checks", "write"),
        ("statuses", "write"),
        ("contents", "read"),
        ("pull_requests", "read"),
        ("metadata", "read"),
        ("administration", "write"),
    ]);

    // when
    let missing = checks::missing_permissions(&old_app);
    let state = checks::installation_state(false, &missing);

    // then
    assert!(checks::WANTED.iter().all(|(name, _)| *name != "statuses"));
    assert!(missing.is_empty());
    assert_eq!(state, (ConnectionState::Active, None));
}

/// A suspended installation is suspended. A missing permission needs attention, except the
/// administration, which only the merge rules need.
#[test]
fn installation_state_follows_the_suspension_and_the_permissions() {
    // given
    let administration = vec!["administration".to_owned()];
    let checks_missing = vec!["checks".to_owned()];

    // when
    let suspended = checks::installation_state(true, &[]);
    let optional = checks::installation_state(false, &administration);
    let required = checks::installation_state(false, &checks_missing);

    // then
    assert_eq!(
        suspended,
        (ConnectionState::Suspended, Some("APP_SUSPENDED"))
    );
    assert_eq!(optional, (ConnectionState::Active, None));
    assert_eq!(
        required,
        (
            ConnectionState::NeedsAttention,
            Some("PERMISSIONS_NOT_ACCEPTED")
        )
    );
}

/// The access levels of GitLab give the roles: 30 Developer, 40 Maintainer, 50 Owner.
#[test]
fn gitlab_role_reads_the_access_level() {
    // when
    let roles: Vec<_> = [20, 30, 40, 50].map(checks::gitlab_role).into();

    // then
    assert_eq!(
        roles,
        [
            None,
            Some(GitLabRole::Developer),
            Some(GitLabRole::Maintainer),
            Some(GitLabRole::Owner)
        ]
    );
}

/// A token of a person, a token that expires in less than 14 days and a Developer role give a
/// warning each. A bot that is Maintainer with a far expiry gives none.
#[test]
fn token_warnings_tell_each_risk() {
    // given
    let now = OffsetDateTime::now_utc();
    let soon = Some(now + Duration::days(6));
    let far = Some(now + Duration::days(300));
    let mixed = [GitLabRole::Maintainer, GitLabRole::Developer];

    // when
    let all = checks::token_warnings(false, soon, &mixed, now);
    let none = checks::token_warnings(true, far, &[GitLabRole::Maintainer], now);
    let never = checks::token_warnings(true, None, &[], now);

    // then
    assert_eq!(
        all,
        [
            checks::NOT_A_SERVICE_ACCOUNT,
            checks::TOKEN_EXPIRES_SOON,
            checks::ROLE_DEVELOPER
        ]
    );
    assert!(none.is_empty());
    assert!(never.is_empty());
}

/// The expiry of a GitLab token is a date, read as the start of that day in UTC. A bad date
/// gives no expiry.
#[test]
fn expiry_of_reads_a_date() {
    // when
    let date = checks::expiry_of("2027-11-01");
    let time = checks::expiry_of("2027-11-01T00:00:00Z");
    let bad = ["2027-13-01", "soon", ""].map(checks::expiry_of);

    // then
    let expected = OffsetDateTime::from_unix_timestamp(1_825_027_200).ok();
    assert_eq!(date, expected);
    assert_eq!(time, expected);
    assert_eq!(bad, [None, None, None]);
}
