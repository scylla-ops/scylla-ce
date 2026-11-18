//! The conversions of the requests of the lists of the triggers of a workspace and of their fires
//! to queries. The responses are in `lists_proto.rs`.

use scylla_domain::projects::jobs::{FiguresPeriod, DEFAULT_GROUPS, MAX_GROUPS, MAX_LIST_IDS};
use scylla_domain::projects::pipelines::TriggerType;
use scylla_domain::projects::triggers::{
    FireGroupBy, FireSource, FireStatsQuery, FireStatus, ListFires, ListWorkspaceTriggers,
    TriggerOrder, WorkspaceFires, MAX_UPCOMING_DAYS,
};
use scylla_domain::tenancy::workspaces::WorkspaceId;
use scylla_domain::{Id, Span};
use scylla_proto::common::v1::Id as ProtoId;
use scylla_proto::pipelines::v1 as pb;
use time::{Duration, OffsetDateTime};

use crate::api::parse;
use crate::Error;

/// Reads a request of `ListWorkspaceTriggers` at the time `now`. The query sees each project of
/// the workspace: the caller sets the projects that it can see.
pub(super) fn summaries_query(
    request: pb::ListWorkspaceTriggersRequest,
    now: OffsetDateTime,
) -> Result<ListWorkspaceTriggers, Error> {
    let until = window(request.upcoming)?.map(|length| now + length);
    Ok(ListWorkspaceTriggers {
        workspace: parse::required(request.workspace_id, "workspace_id")?,
        visible: None,
        projects: ids(request.project_ids, "project_ids")?,
        pipelines: ids(request.pipeline_ids, "pipeline_ids")?,
        kinds: kinds(request.kinds)?,
        enabled_only: request.enabled_only,
        order: order(request.order)?,
        until,
        since: parse::or(request.since, "since", now - Duration::hours(24))?,
        page: parse::optional(request.page, "page")?,
    })
}

/// Reads a request of `ListTriggerFires`. The query sees each project of the workspace.
pub(super) fn fires_query(request: pb::ListTriggerFiresRequest) -> Result<ListFires, Error> {
    let from: Option<OffsetDateTime> = request
        .from
        .map(|at| parse::field(at, "from"))
        .transpose()?;
    let to: Option<OffsetDateTime> = request.to.map(|at| parse::field(at, "to")).transpose()?;
    if let (Some(from), Some(to)) = (from, to) {
        if to <= from {
            return Err(Error::invalid("to", "must be after from"));
        }
    }
    let workspace = parse::required(request.workspace_id, "workspace_id")?;
    let fires = workspace_fires(
        workspace,
        [
            request.project_ids,
            request.pipeline_ids,
            request.trigger_ids,
        ],
        request.sources,
    )?;
    Ok(ListFires {
        fires,
        kinds: kinds(request.kinds)?,
        statuses: statuses(request.statuses)?,
        from,
        to,
        page: parse::optional(request.page, "page")?,
    })
}

/// Reads a request of `GetTriggerFireStats` at the time `now`. The query sees each project of the
/// workspace.
pub(super) fn stats_query(
    request: pb::GetTriggerFireStatsRequest,
    now: OffsetDateTime,
) -> Result<FireStatsQuery, Error> {
    // Read the choices of the counts.
    let bucket = match request.bucket() {
        pb::FireStatsBucket::Unspecified => None,
        pb::FireStatsBucket::Hour => Some(Span::Hour),
        pb::FireStatsBucket::Day => Some(Span::Day),
        pb::FireStatsBucket::Week => Some(Span::Week),
    };
    let group_by = match request.group_by() {
        pb::FireStatsGroup::Unspecified => None,
        pb::FireStatsGroup::Trigger => Some(FireGroupBy::Trigger),
        pb::FireStatsGroup::Pipeline => Some(FireGroupBy::Pipeline),
        pb::FireStatsGroup::Project => Some(FireGroupBy::Project),
    };
    let limit = match request.group_limit {
        0 => DEFAULT_GROUPS,
        limit if limit > MAX_GROUPS => {
            let reason = format!("must be at most {MAX_GROUPS}");
            return Err(Error::invalid("group_limit", reason));
        }
        limit => limit,
    };

    // Read the period and the fires to count.
    let from: OffsetDateTime = parse::required(request.from, "from")?;
    let to: Option<OffsetDateTime> = request.to.map(|at| parse::field(at, "to")).transpose()?;
    let period = FiguresPeriod::new(from, to, now)
        .map_err(|error| Error::invalid("to", error.to_string()))?;
    let workspace = parse::required(request.workspace_id, "workspace_id")?;
    let fires = workspace_fires(
        workspace,
        [
            request.project_ids,
            request.pipeline_ids,
            request.trigger_ids,
        ],
        request.sources,
    )?;
    Ok(FireStatsQuery {
        fires,
        period,
        bucket,
        group_by,
        limit,
    })
}

/// Reads the fires of a workspace to read: the ids of the projects, of the pipelines and of the
/// triggers, and the sources.
fn workspace_fires(
    workspace: WorkspaceId,
    [projects, pipelines, triggers]: [Vec<ProtoId>; 3],
    sources: Vec<i32>,
) -> Result<WorkspaceFires, Error> {
    let sources = sources
        .into_iter()
        .map(|raw| match pb::FireSource::try_from(raw) {
            Ok(pb::FireSource::Schedule) => Some(FireSource::Schedule),
            Ok(pb::FireSource::Webhook) => Some(FireSource::Webhook),
            Ok(pb::FireSource::Manual) => Some(FireSource::Manual),
            Ok(pb::FireSource::Unspecified) | Err(_) => None,
        })
        .collect::<Option<Vec<_>>>()
        .ok_or_else(|| Error::invalid("sources", "must be known sources"))?;
    Ok(WorkspaceFires {
        workspace,
        visible: None,
        projects: ids(projects, "project_ids")?,
        pipelines: ids(pipelines, "pipeline_ids")?,
        triggers: ids(triggers, "trigger_ids")?,
        sources,
    })
}

/// Reads a list of ids of a request, at most `MAX_LIST_IDS`.
fn ids<T: ?Sized>(values: Vec<ProtoId>, field: &'static str) -> Result<Vec<Id<T>>, Error> {
    if values.len() > MAX_LIST_IDS {
        let reason = format!("must have at most {MAX_LIST_IDS} ids");
        return Err(Error::invalid(field, reason));
    }
    values
        .into_iter()
        .map(|value| parse::field(value, field))
        .collect()
}

/// Reads the kinds of a filter.
fn kinds(raw: Vec<i32>) -> Result<Vec<TriggerType>, Error> {
    raw.into_iter()
        .map(|value| match pb::TriggerKind::try_from(value) {
            Ok(pb::TriggerKind::Cron) => Some(TriggerType::Cron),
            Ok(pb::TriggerKind::Webhook) => Some(TriggerType::Webhook),
            Ok(pb::TriggerKind::Push) => Some(TriggerType::Push),
            Ok(pb::TriggerKind::PullRequest) => Some(TriggerType::PullRequest),
            Ok(pb::TriggerKind::Tag) => Some(TriggerType::Tag),
            Ok(pb::TriggerKind::Unspecified) | Err(_) => None,
        })
        .collect::<Option<Vec<_>>>()
        .ok_or_else(|| Error::invalid("kinds", "must be known kinds"))
}

/// Reads the results of a filter.
fn statuses(raw: Vec<i32>) -> Result<Vec<FireStatus>, Error> {
    raw.into_iter()
        .map(|value| match pb::FireStatus::try_from(value) {
            Ok(pb::FireStatus::Ok) => Some(FireStatus::Ok),
            Ok(pb::FireStatus::Error) => Some(FireStatus::Error),
            Ok(pb::FireStatus::Missed) => Some(FireStatus::Missed),
            Ok(pb::FireStatus::Unspecified) | Err(_) => None,
        })
        .collect::<Option<Vec<_>>>()
        .ok_or_else(|| Error::invalid("statuses", "must be known results"))
}

/// Reads the order of a list of summaries.
fn order(raw: i32) -> Result<TriggerOrder, Error> {
    match pb::TriggerOrder::try_from(raw) {
        Ok(pb::TriggerOrder::Unspecified | pb::TriggerOrder::NextFire) => {
            Ok(TriggerOrder::NextFire)
        }
        Ok(pb::TriggerOrder::Name) => Ok(TriggerOrder::Name),
        Err(_) => Err(Error::invalid("order", "must be a known order")),
    }
}

/// Reads the window of the upcoming fires: from 0 to `MAX_UPCOMING_DAYS` days, or `None` when it
/// is not set.
fn window(raw: Option<prost_types::Duration>) -> Result<Option<Duration>, Error> {
    let Some(raw) = raw else {
        return Ok(None);
    };
    let longest = Duration::days(MAX_UPCOMING_DAYS);
    let invalid = || {
        Error::invalid(
            "upcoming",
            format!("must be from 0 to {MAX_UPCOMING_DAYS} days"),
        )
    };
    if !(0..=longest.whole_seconds()).contains(&raw.seconds)
        || !(0..1_000_000_000).contains(&raw.nanos)
    {
        return Err(invalid());
    }
    let length = Duration::new(raw.seconds, raw.nanos);
    match length <= longest {
        true => Ok(Some(length)),
        false => Err(invalid()),
    }
}
