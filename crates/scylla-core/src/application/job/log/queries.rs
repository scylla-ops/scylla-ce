//! The job log's reads. One block per query, in the order it runs: the struct, its access,
//! its output type, what `Fetch` reads.

use super::JobLogUseCases;
use crate::application::JobLogLiveStream;
use crate::application::pagination::{PaginatedResult, PaginationParams};
use crate::domain::errors::DomainResult;
use crate::domain::ids::JobId;
use crate::domain::job::{JobLog, LogStream};
use crate::domain::permission::Permission;
use crate::domain::pipeline::NodeId;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use futures_util::stream::{self, StreamExt, TryStreamExt};
use scylla_extension::{Access, Authorized, Describe, Fetch, Fetched, Query, Run};
use std::collections::HashSet;

#[derive(Debug)]
pub struct ListJobLogs {
    pub job_id: JobId,
    pub node_id: Option<NodeId>,
    pub pagination: Option<PaginationParams>,
}

impl Describe for ListJobLogs {
    fn access(&self) -> Access {
        let logs = Permission::ReadJobLogs(self.job_id.clone());
        match self.node_id {
            Some(_) => Access::RequiresAll(vec![logs, Permission::ReadJob(self.job_id.clone())]),
            None => Access::Requires(logs),
        }
    }
}

impl Query for ListJobLogs {
    type Output = PaginatedResult<JobLog>;
}

#[async_trait]
impl Run<Fetch<ListJobLogs>> for JobLogUseCases {
    async fn run(&self, input: Authorized<ListJobLogs>) -> DomainResult<Fetched<ListJobLogs>> {
        let query = input.command();
        let pagination = query.pagination.as_ref();
        if !self
            .node_readable(&query.job_id, query.node_id.as_ref())
            .await?
        {
            let params = query.pagination.unwrap_or_default();
            return Ok(input.fetched(PaginatedResult::new(Vec::new(), &params, 0)));
        }
        let page = match &query.node_id {
            Some(node_id) => {
                self.log_repo
                    .list_by_job_and_node(&query.job_id, node_id, pagination)
                    .await?
            }
            None => self.log_repo.list_by_job(&query.job_id, pagination).await?,
        };
        Ok(input.fetched(page))
    }
}

/// The persisted lines, then the live ones; a live line already in the snapshot is dropped. A
/// node that has not started replays nothing.
#[derive(Debug)]
pub struct TailJobLogs {
    pub job_id: JobId,
    pub node_id: Option<NodeId>,
}

impl Describe for TailJobLogs {
    fn access(&self) -> Access {
        let logs = Permission::ReadJobLogs(self.job_id.clone());
        match self.node_id {
            Some(_) => Access::RequiresAll(vec![logs, Permission::ReadJob(self.job_id.clone())]),
            None => Access::Requires(logs),
        }
    }
}

impl Query for TailJobLogs {
    type Output = JobLogLiveStream;
}

#[async_trait]
impl Run<Fetch<TailJobLogs>> for JobLogUseCases {
    async fn run(&self, input: Authorized<TailJobLogs>) -> DomainResult<Fetched<TailJobLogs>> {
        let query = input.command();
        let node_id = query.node_id.as_ref();
        let live = self.stream_port.subscribe(&query.job_id, node_id).await?;
        let historical = if self.node_readable(&query.job_id, node_id).await? {
            self.log_repo
                .list_all_by_job(&query.job_id, node_id)
                .await?
        } else {
            Vec::new()
        };
        let cutoff = historical.last().map(JobLog::timestamp);

        let seen: HashSet<(DateTime<Utc>, NodeId, LogStream, String)> = historical
            .iter()
            .map(|l| {
                (
                    l.timestamp(),
                    l.node_id().clone(),
                    *l.stream(),
                    l.line().to_string(),
                )
            })
            .collect();

        let historical_stream = stream::iter(historical.into_iter().map(Ok));
        let filtered_live = live.try_filter(move |log| {
            let strictly_newer = cutoff.is_none_or(|c| log.timestamp() > c);
            let keep = if strictly_newer {
                true
            } else {
                !seen.contains(&(
                    log.timestamp(),
                    log.node_id().clone(),
                    *log.stream(),
                    log.line().to_string(),
                ))
            };
            std::future::ready(keep)
        });

        let tail: JobLogLiveStream = Box::pin(historical_stream.chain(filtered_live));
        Ok(input.fetched(tail))
    }
}
