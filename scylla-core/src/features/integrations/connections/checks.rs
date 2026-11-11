//! The pure rules of the connections: the permissions that Scylla asks of the GitHub App, the
//! state of an installation, the role of a GitLab service account, and the warnings of a GitLab
//! token.

use std::collections::BTreeMap;

use scylla_domain::integrations::connections::ConnectionState;
use scylla_domain::integrations::links::GitLabRole;
use time::{Date, Duration, Month, OffsetDateTime};

/// The permissions that Scylla asks of the GitHub App, with their access.
pub(crate) const WANTED: &[(&str, &str)] = &[
    ("checks", "write"),
    ("statuses", "write"),
    ("contents", "read"),
    ("pull_requests", "read"),
    ("metadata", "read"),
    ("administration", "write"),
];

/// The permission that only the merge rules need: a connection works with no such permission.
const OPTIONAL: &str = "administration";

/// The time before the expiry of a GitLab token when Scylla warns.
const EXPIRY_WARNING: Duration = Duration::days(14);

/// The warning of a token of a person, not of a service account or a bot.
pub(crate) const NOT_A_SERVICE_ACCOUNT: &str = "NOT_A_SERVICE_ACCOUNT";

/// The warning of a token that expires in less than 14 days.
pub(crate) const TOKEN_EXPIRES_SOON: &str = "TOKEN_EXPIRES_SOON";

/// The warning of a service account that is only Developer on a project.
pub(crate) const ROLE_DEVELOPER: &str = "ROLE_DEVELOPER";

/// Returns the rank of an access of GitHub: none, read, write, admin.
fn rank(access: Option<&str>) -> u8 {
    match access {
        Some("admin") => 3,
        Some("write") => 2,
        Some("read") => 1,
        _ => 0,
    }
}

/// Returns the permissions that Scylla asks for and that an installation does not have, in the
/// order of `WANTED`.
pub(crate) fn missing_permissions(held: &BTreeMap<String, String>) -> Vec<String> {
    WANTED
        .iter()
        .filter(|(name, access)| rank(held.get(*name).map(String::as_str)) < rank(Some(access)))
        .map(|(name, _)| (*name).to_owned())
        .collect()
}

/// Returns the state of an installation and its reason: suspended, else a missing permission
/// that is not optional, else active.
pub(crate) fn installation_state(
    suspended: bool,
    missing: &[String],
) -> (ConnectionState, Option<&'static str>) {
    if suspended {
        return (ConnectionState::Suspended, Some("APP_SUSPENDED"));
    }
    if missing.iter().any(|name| name != OPTIONAL) {
        return (
            ConnectionState::NeedsAttention,
            Some("PERMISSIONS_NOT_ACCEPTED"),
        );
    }
    (ConnectionState::Active, None)
}

/// Returns the role of an access level of GitLab: 30 is Developer, 40 Maintainer, 50 Owner.
pub(crate) fn gitlab_role(level: u32) -> Option<GitLabRole> {
    match level {
        50.. => Some(GitLabRole::Owner),
        40..=49 => Some(GitLabRole::Maintainer),
        30..=39 => Some(GitLabRole::Developer),
        _ => None,
    }
}

/// Returns the warnings of the check of a GitLab token, at the time `now`: the token of a
/// person, a token that expires soon, and a project where the account is only Developer.
pub(crate) fn token_warnings(
    bot: bool,
    expires_at: Option<OffsetDateTime>,
    roles: &[GitLabRole],
    now: OffsetDateTime,
) -> Vec<&'static str> {
    let mut warnings = Vec::new();
    if !bot {
        warnings.push(NOT_A_SERVICE_ACCOUNT);
    }
    if expires_at.is_some_and(|at| at < now + EXPIRY_WARNING) {
        warnings.push(TOKEN_EXPIRES_SOON);
    }
    if roles.contains(&GitLabRole::Developer) {
        warnings.push(ROLE_DEVELOPER);
    }
    warnings
}

/// Reads the expiry of a GitLab token, a date such as `2027-11-01`, as the start of that day in
/// UTC. A time after the date is ignored.
pub(crate) fn expiry_of(text: &str) -> Option<OffsetDateTime> {
    let date = text.get(..10)?;
    let mut parts = date.split('-');
    let year: i32 = parts.next()?.parse().ok()?;
    let month: u8 = parts.next()?.parse().ok()?;
    let day: u8 = parts.next()?.parse().ok()?;
    let date = Date::from_calendar_date(year, Month::try_from(month).ok()?, day).ok()?;
    Some(date.midnight().assume_utc())
}
