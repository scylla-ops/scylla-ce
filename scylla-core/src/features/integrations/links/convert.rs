//! The proto values of the repository links: a link with the account of its connection, a
//! branch or a tag, and the values of a request.

use scylla_domain::integrations::connections::{ConnectionState, DeliveryMode};
use scylla_domain::integrations::links::{PipelineSource, ReportMode};
use scylla_domain::integrations::VcsProvider;
use scylla_proto::integrations::v1 as pb;

use super::refs::{RefInfo, RefKind};
use super::store::Linked;
use crate::api::parse::{self, ToProto};

impl ToProto<pb::RepositoryLink> for Linked {
    fn to_proto(self) -> pb::RepositoryLink {
        let Linked {
            link,
            provider,
            account,
            ..
        } = self;
        pb::RepositoryLink {
            project_id: Some(link.project_id.to_proto()),
            workspace_id: Some(link.workspace_id.to_proto()),
            connection_id: Some(link.connection_id.to_proto()),
            provider: provider_of(provider) as i32,
            account,
            external_id: link.external_id,
            full_name: link.full_name,
            web_url: link.web_url,
            default_branch: link.default_branch,
            private: link.private,
            pipeline_source: pipeline_source(link.pipeline_source) as i32,
            reports_enabled: link.reports_enabled,
            report_mode: report_mode(link.report_mode) as i32,
            delivery_mode: delivery_mode(link.delivery_mode) as i32,
            poll_interval_seconds: link.poll_interval_seconds,
            last_event_at: link.last_event_at.map(parse::timestamp),
            state: state(link.state) as i32,
            state_reason: link.state_reason.unwrap_or_default(),
            created_by: Some(link.created_by.to_proto()),
            created_at: Some(parse::timestamp(link.created_at)),
        }
    }
}

impl ToProto<pb::GitRefInfo> for RefInfo {
    fn to_proto(self) -> pb::GitRefInfo {
        let kind = match self.kind {
            RefKind::Branch => pb::GitRefKind::Branch,
            RefKind::Tag => pb::GitRefKind::Tag,
        };
        pb::GitRefInfo {
            name: self.name,
            kind: kind as i32,
            sha: self.sha,
            default_branch: self.default_branch,
            web_url: self.web_url,
        }
    }
}

/// Reads the kind of a filter of refs: not set gives both kinds.
pub(super) fn ref_kind(value: i32) -> Option<Option<RefKind>> {
    match pb::GitRefKind::try_from(value).ok()? {
        pb::GitRefKind::Unspecified => Some(None),
        pb::GitRefKind::Branch => Some(Some(RefKind::Branch)),
        pb::GitRefKind::Tag => Some(Some(RefKind::Tag)),
    }
}

/// Reads a report mode of a request: not set keeps the mode.
pub(super) fn report_mode_of(value: i32) -> Option<Option<ReportMode>> {
    match pb::ReportMode::try_from(value).ok()? {
        pb::ReportMode::Unspecified => Some(None),
        pb::ReportMode::CheckRun => Some(Some(ReportMode::CheckRun)),
        pb::ReportMode::CommitStatus => Some(Some(ReportMode::CommitStatus)),
    }
}

/// Returns the proto value of a provider.
fn provider_of(value: VcsProvider) -> pb::VcsProvider {
    match value {
        VcsProvider::GitHub => pb::VcsProvider::Github,
        VcsProvider::GitLab => pb::VcsProvider::Gitlab,
    }
}

/// Returns the proto value of a pipeline source.
fn pipeline_source(value: PipelineSource) -> pb::PipelineSource {
    match value {
        PipelineSource::Editor => pb::PipelineSource::Editor,
        PipelineSource::File => pb::PipelineSource::File,
    }
}

/// Returns the proto value of a report mode.
fn report_mode(value: ReportMode) -> pb::ReportMode {
    match value {
        ReportMode::CheckRun => pb::ReportMode::CheckRun,
        ReportMode::CommitStatus => pb::ReportMode::CommitStatus,
    }
}

/// Returns the proto value of a delivery mode.
fn delivery_mode(value: DeliveryMode) -> pb::DeliveryMode {
    match value {
        DeliveryMode::Webhook => pb::DeliveryMode::Webhook,
        DeliveryMode::Polling => pb::DeliveryMode::Polling,
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
