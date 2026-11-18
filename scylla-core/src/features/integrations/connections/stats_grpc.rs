//! The handler of `GetDeliveryStats`: the counts of the webhook deliveries of a workspace for the
//! dashboard, with the webhook triggers of the projects whose fires the caller can count.

use scylla_domain::integrations::connections::VcsConnectionHolder;
use scylla_domain::integrations::deliveries::{
    ConnectionDeliveries, DeliveryFigures, DeliveryStats, DeliveryStatsQuery, TriggerDeliveries,
};
use scylla_domain::projects::jobs::FiguresPeriod;
use scylla_domain::projects::triggers::{TriggerAccess, TriggerHolder};
use scylla_domain::Span;
use scylla_proto::integrations::v1 as pb;
use time::OffsetDateTime;
use tonic::Request;

use super::grpc::ConnectionsGrpc;
use super::stats;
use crate::api::parse::{self, ToProto};
use crate::features::projects::pipelines::access;
use crate::Error;

impl ConnectionsGrpc {
    /// Handles `GetDeliveryStats`: the counts of the deliveries of the connections of a workspace
    /// and of the webhook triggers that the caller can see, for a period.
    pub(super) async fn delivery_stats(
        &self,
        request: Request<pb::GetDeliveryStatsRequest>,
    ) -> Result<pb::GetDeliveryStatsResponse, Error> {
        // Authenticate the caller, then read the request.
        let caller = self.ctx.gate.caller(&request).await?;
        let now = OffsetDateTime::now_utc();
        let mut query = stats_query(request.into_inner(), now)?;

        // Check the route permission, then find the projects whose fires the caller can count.
        let permission = VcsConnectionHolder::GET_DELIVERY_STATS;
        self.authorize(&caller, permission, query.workspace).await?;
        let (holder, project) = (
            TriggerHolder::GET_TRIGGER_FIRE_STATS,
            TriggerAccess::GET_TRIGGER_FIRE_STATS,
        );
        let gate = self.ctx.gate.as_ref();
        query.visible =
            access::visible_projects(gate, &caller, query.workspace, holder, project).await;

        // Count the deliveries.
        let stats = stats::delivery_stats(&self.ctx, query).await?;
        Ok(stats.to_proto())
    }
}

/// Reads the request of `GetDeliveryStats`. Refuses no start, an end that is not after the
/// start, and a period that is too long.
fn stats_query(
    request: pb::GetDeliveryStatsRequest,
    now: OffsetDateTime,
) -> Result<DeliveryStatsQuery, Error> {
    let bucket = match request.bucket() {
        pb::DeliveryStatsBucket::Unspecified => None,
        pb::DeliveryStatsBucket::Hour => Some(Span::Hour),
        pb::DeliveryStatsBucket::Day => Some(Span::Day),
        pb::DeliveryStatsBucket::Week => Some(Span::Week),
    };
    let from: OffsetDateTime = parse::required(request.from, "from")?;
    let to: Option<OffsetDateTime> = request.to.map(|at| parse::field(at, "to")).transpose()?;
    let period = FiguresPeriod::new(from, to, now)
        .map_err(|error| Error::invalid("to", error.to_string()))?;
    Ok(DeliveryStatsQuery {
        workspace: parse::required(request.workspace_id, "workspace_id")?,
        visible: Some(Vec::new()),
        period,
        bucket,
    })
}

impl ToProto<pb::GetDeliveryStatsResponse> for DeliveryStats {
    fn to_proto(self) -> pb::GetDeliveryStatsResponse {
        let bucket = |(start, figures): (OffsetDateTime, DeliveryFigures)| pb::DeliveryBucket {
            start: Some(parse::timestamp(start)),
            figures: Some(figures.to_proto()),
        };
        let trigger = |one: TriggerDeliveries| pb::TriggerDeliveries {
            trigger_id: Some(one.trigger.to_proto()),
            figures: Some(one.figures.to_proto()),
            last_received_at: Some(parse::timestamp(one.last_received_at)),
        };
        pb::GetDeliveryStatsResponse {
            totals: Some(self.totals.to_proto()),
            connections: self.connections.into_iter().map(connection).collect(),
            triggers: self.triggers.into_iter().map(trigger).collect(),
            buckets: self.buckets.into_iter().map(bucket).collect(),
            time_zone: self.time_zone.as_str().to_owned(),
        }
    }
}

impl ToProto<pb::DeliveryFigures> for DeliveryFigures {
    fn to_proto(self) -> pb::DeliveryFigures {
        pb::DeliveryFigures {
            delivered: self.delivered,
            failed: self.failed,
        }
    }
}

/// Makes the proto value of the deliveries of a connection, with its newest delivery.
fn connection(one: ConnectionDeliveries) -> pb::ConnectionDeliveries {
    let last = one.last;
    pb::ConnectionDeliveries {
        connection_id: Some(one.connection.to_proto()),
        figures: Some(one.figures.to_proto()),
        last_received_at: last.as_ref().map(|last| parse::timestamp(last.received_at)),
        last_event: last
            .as_ref()
            .map(|last| last.event.clone())
            .unwrap_or_default(),
        last_repository: last
            .as_ref()
            .and_then(|last| last.repository.clone())
            .unwrap_or_default(),
        last_ref: last.and_then(|last| last.git_ref).unwrap_or_default(),
    }
}
