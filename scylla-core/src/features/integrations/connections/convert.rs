//! The proto values of the connections: a provider, a connection with its project count and the
//! pages of the provider, and a repository. No value holds a token.

use scylla_domain::integrations::connections::{
    AccountKind, AuthKind, ConnectionState, DeliveryMode, VcsConnection,
};
use scylla_domain::integrations::VcsProvider;
use scylla_proto::integrations::v1 as pb;

use super::checks;
use super::repositories::Seen;
use super::service::ProviderInfo;
use super::store::Counted;
use crate::api::parse::{self, ToProto};
use crate::infra::vcs::VcsOptions;

/// Makes the proto value of a provider.
pub(super) fn provider_info(info: ProviderInfo) -> pb::VcsProviderInfo {
    pb::VcsProviderInfo {
        provider: provider(info.provider) as i32,
        available: info.unavailable.is_none(),
        unavailable_reason: info.unavailable.unwrap_or_default().to_owned(),
        web_url: info.web_url,
    }
}

/// Makes the proto value of a connection, with the URLs of the providers of the settings.
pub(super) fn connection(counted: Counted, settings: &VcsOptions) -> pb::VcsConnection {
    let Counted {
        connection,
        projects,
    } = counted;
    let missing = match connection.provider {
        VcsProvider::GitHub => checks::missing_permissions(&connection.permissions),
        VcsProvider::GitLab => Vec::new(),
    };
    pb::VcsConnection {
        id: Some(connection.id.to_proto()),
        workspace_id: Some(connection.workspace_id.to_proto()),
        provider: provider(connection.provider) as i32,
        auth_kind: auth_kind(connection.auth_kind) as i32,
        settings_url: settings_url(&connection, settings),
        external_id: connection.external_id,
        account: connection.account,
        account_kind: account_kind(connection.account_kind) as i32,
        avatar_url: connection.avatar_url.unwrap_or_default(),
        state: state(connection.state) as i32,
        state_reason: connection.state_reason.unwrap_or_default(),
        delivery_mode: delivery_mode(connection.delivery_mode) as i32,
        poll_interval_seconds: connection.poll_interval_seconds,
        all_repositories: connection.all_repositories,
        repository_count: connection.repository_count,
        project_count: projects,
        token_expires_at: connection.token_expires_at.map(parse::timestamp),
        scopes: connection.scopes,
        missing_permissions: missing,
        created_by: Some(connection.created_by.to_proto()),
        created_at: Some(parse::timestamp(connection.created_at)),
    }
}

/// Makes the proto value of a repository.
pub(super) fn repository(seen: Seen) -> pb::Repository {
    pb::Repository {
        external_id: seen.external_id,
        full_name: seen.full_name,
        web_url: seen.web_url,
        default_branch: seen.default_branch,
        private: seen.private,
        role: seen
            .role
            .map(|role| role.as_str().to_owned())
            .unwrap_or_default(),
        linked_project_id: seen.linked.map(ToProto::to_proto),
    }
}

/// Returns the page of a connection on its provider: the settings of the installation on
/// GitHub, the profile of the account on GitLab.
fn settings_url(connection: &VcsConnection, settings: &VcsOptions) -> String {
    let (account, id) = (&connection.account, &connection.external_id);
    match (connection.provider, connection.account_kind) {
        (VcsProvider::GitHub, AccountKind::Organization) => format!(
            "{}/organizations/{account}/settings/installations/{id}",
            settings.github_web_url
        ),
        (VcsProvider::GitHub, _) => {
            format!("{}/settings/installations/{id}", settings.github_web_url)
        }
        (VcsProvider::GitLab, _) => format!("{}/{account}", settings.gitlab_url),
    }
}

/// Returns the proto value of a provider.
fn provider(value: VcsProvider) -> pb::VcsProvider {
    match value {
        VcsProvider::GitHub => pb::VcsProvider::Github,
        VcsProvider::GitLab => pb::VcsProvider::Gitlab,
    }
}

/// Returns the proto value of an auth kind.
fn auth_kind(value: AuthKind) -> pb::AuthKind {
    match value {
        AuthKind::AppInstallation => pb::AuthKind::AppInstallation,
        AuthKind::AccessToken => pb::AuthKind::AccessToken,
    }
}

/// Returns the proto value of an account kind.
fn account_kind(value: AccountKind) -> pb::AccountKind {
    match value {
        AccountKind::Organization => pb::AccountKind::Organization,
        AccountKind::User => pb::AccountKind::User,
        AccountKind::ServiceAccount => pb::AccountKind::ServiceAccount,
    }
}

/// Returns the proto value of a state.
fn state(value: ConnectionState) -> pb::ConnectionState {
    match value {
        ConnectionState::Active => pb::ConnectionState::Active,
        ConnectionState::Suspended => pb::ConnectionState::Suspended,
        ConnectionState::NeedsAttention => pb::ConnectionState::NeedsAttention,
        ConnectionState::Disconnected => pb::ConnectionState::Disconnected,
    }
}

/// Returns the proto value of a delivery mode.
fn delivery_mode(value: DeliveryMode) -> pb::DeliveryMode {
    match value {
        DeliveryMode::Webhook => pb::DeliveryMode::Webhook,
        DeliveryMode::Polling => pb::DeliveryMode::Polling,
    }
}
