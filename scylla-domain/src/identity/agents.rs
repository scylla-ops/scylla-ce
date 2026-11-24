//! The agents: the Apps that run jobs, with their stream, their host, their protocol and their
//! labels, the busy slots of their jobs, and the samples of the use of their machines.

mod labels;
mod samples;
mod slots;

#[cfg(test)]
mod tests;

use time::OffsetDateTime;

use crate::identity::actors::Actor;
use crate::identity::apps::App;
use crate::projects::jobs::FiguresPeriod;
use crate::tenancy::workspaces::Workspace;
use crate::DomainError;

pub use labels::{fits, AgentLabel, AgentLabels, CONTAINER_LABEL};
pub use samples::{
    AgentMetrics, HostSample, MetricsBucket, MetricsFigures, MetricsQuery, Sample, SampleSums,
    SAMPLE_GAP, SAMPLE_INTERVAL, SAMPLE_RETENTION,
};
pub use slots::{Busy, SlotBucket, SlotFigures, SlotQuery, SlotUsage};

/// The oldest agent protocol that the server takes. An older agent gets `AGENT_TOO_OLD`.
pub const MIN_AGENT_PROTOCOL: u32 = 0;

/// The agent protocol of this version: the protocol of the agent of the same version.
pub const CURRENT_AGENT_PROTOCOL: u32 = 2;

/// The first agent protocol that gets the code of a job before its first node.
pub const CHECKOUT_PROTOCOL: u32 = 2;

crate::authz! {
    /// The routes of an agent on itself.
    extend Actor as AgentCaller {
        /// The route `OpenStream`: an agent opens its own stream.
        permission open_stream = nil;
    }

    /// The routes of the agents on their workspace.
    extend Workspace as AgentHolder {
        /// The route `GetSlotUsage`: the busy slots of the agents of this workspace.
        permission get_slot_usage = read;
        /// The route `GetAgentMetrics`, on each App of this workspace.
        permission get_agent_metrics = read;
    }

    /// The routes of the agents on one App.
    extend App as AgentAccess {
        /// The route `GetAgentMetrics`.
        permission get_agent_metrics = creator;
    }
}

/// Returns the bounds of the parts of a period: its start, then the start of each bucket after
/// the first. With no bucket, the period is one part.
pub fn bounds(period: FiguresPeriod, starts: &[OffsetDateTime]) -> Vec<OffsetDateTime> {
    let mut bounds = vec![period.from];
    bounds.extend(starts.iter().skip(1).copied());
    bounds
}

/// Returns the value of each part of `count` parts from the part numbers of `found`: number 1 is
/// the first part. A part that is not in `found` gets the default value.
fn place<T: Copy + Default>(count: usize, found: &[(usize, T)]) -> Vec<T> {
    (1..=count)
        .map(|number| {
            found
                .iter()
                .find(|(place, _)| *place == number)
                .map(|(_, value)| *value)
                .unwrap_or_default()
        })
        .collect()
}

/// How an agent fits the server, from its protocol.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Compatibility {
    /// The agent has the current protocol, or a newer one.
    Compatible,
    /// The agent connects, and gets only the jobs that it can run.
    Outdated,
    /// The server refuses the stream of the agent.
    Unsupported,
}

/// Why an online agent cannot run a job.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unfit {
    /// The protocol of the agent is too old: update the agent.
    UpdateAgent,
    /// The agent has the protocol, but git does not run on its machine.
    InstallGit,
}

/// The machine of an agent, from the hello of its stream.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Host {
    /// The version of the agent.
    pub version: String,
    /// The operating system.
    pub os: String,
    /// The CPU architecture.
    pub arch: String,
    /// The host name.
    pub hostname: String,
    /// The number of CPUs.
    pub cpu_count: u32,
    /// The memory in bytes.
    pub memory_bytes: u64,
    /// The largest number of jobs at the same time.
    pub capacity: u32,
    /// The container runtime, or `None` if the agent has none.
    pub container_runtime: Option<String>,
    /// The features of the agent, for example `checkout`.
    pub features: Vec<String>,
    /// The protocol of the agent. An agent older than the protocol sends 0.
    pub protocol: u32,
    /// The commit of the build of the agent, or `None` for a build with no commit.
    pub build: Option<String>,
    /// The start of the process of the agent, or `None` from an agent of an older version.
    pub started_at: Option<OffsetDateTime>,
}

impl Host {
    /// The maximum number of characters of each text, after trim.
    pub const MAX_TEXT: usize = 100;
    /// The largest capacity.
    pub const MAX_CAPACITY: u32 = 256;
    /// The largest number of features.
    pub const MAX_FEATURES: usize = 16;
    /// The feature of an agent that gets the code of a job before its first node.
    pub const CHECKOUT: &'static str = "checkout";
    /// The features that the server can need.
    pub const FEATURES: [&'static str; 1] = [Self::CHECKOUT];

    /// Returns `true` if the agent gets the code of a job before its first node.
    pub fn can_check_out(&self) -> bool {
        self.features
            .iter()
            .any(|feature| feature == Self::CHECKOUT)
    }

    /// Returns how the protocol of the agent fits the server.
    pub fn compatibility(&self) -> Compatibility {
        if !(MIN_AGENT_PROTOCOL..).contains(&self.protocol) {
            Compatibility::Unsupported
        } else if self.protocol < CURRENT_AGENT_PROTOCOL {
            Compatibility::Outdated
        } else {
            Compatibility::Compatible
        }
    }

    /// Returns the features that the server can need and that the agent does not tell.
    pub fn missing_features(&self) -> Vec<String> {
        Self::FEATURES
            .into_iter()
            .filter(|feature| !self.features.iter().any(|told| told == feature))
            .map(str::to_owned)
            .collect()
    }

    /// Returns why the agent cannot get the code of a job, or `None` if it can.
    pub fn checkout_gap(&self) -> Option<Unfit> {
        match (self.can_check_out(), self.protocol < CHECKOUT_PROTOCOL) {
            (true, _) => None,
            (false, true) => Some(Unfit::UpdateAgent),
            (false, false) => Some(Unfit::InstallGit),
        }
    }

    /// Trims and checks each text (one line, at most `MAX_TEXT` characters), the features (at
    /// most `MAX_FEATURES`, with no empty and no double one) and the capacity. A capacity of 0
    /// gives 1, and an empty build gives none.
    pub fn parse(mut host: Host) -> Result<Self, (&'static str, DomainError)> {
        // Keep each feature one time.
        let mut features: Vec<String> = Vec::new();
        for feature in &host.features {
            let feature = host_text(feature).map_err(|error| ("features", error))?;
            if !feature.is_empty() && !features.contains(&feature) {
                features.push(feature);
            }
        }
        if features.len() > Self::MAX_FEATURES {
            let error = DomainError::TooMany {
                max: Self::MAX_FEATURES,
            };
            return Err(("features", error));
        }
        host.features = features;

        // Check each text and the capacity.
        for (field, text) in [
            ("version", &mut host.version),
            ("os", &mut host.os),
            ("arch", &mut host.arch),
            ("hostname", &mut host.hostname),
        ] {
            *text = host_text(text).map_err(|error| (field, error))?;
        }
        host.container_runtime = optional_text(host.container_runtime, "container_runtime")?;
        host.build = optional_text(host.build, "build")?;
        if host.capacity > Self::MAX_CAPACITY {
            let error = DomainError::TooLarge {
                max: Self::MAX_CAPACITY as usize,
            };
            return Err(("capacity", error));
        }
        host.capacity = host.capacity.max(1);
        Ok(host)
    }
}

/// Checks an optional text of a host: an empty text gives `None`.
fn optional_text(
    text: Option<String>,
    field: &'static str,
) -> Result<Option<String>, (&'static str, DomainError)> {
    match text.as_deref().map(host_text) {
        Some(Ok(text)) if !text.is_empty() => Ok(Some(text)),
        Some(Err(error)) => Err((field, error)),
        _ => Ok(None),
    }
}

/// Trims a text of a host, and checks that it is one line of at most `Host::MAX_TEXT`
/// characters.
fn host_text(raw: &str) -> Result<String, DomainError> {
    let trimmed = raw.trim();
    if trimmed.chars().count() > Host::MAX_TEXT {
        return Err(DomainError::TooLong {
            max: Host::MAX_TEXT,
        });
    }
    if trimmed.chars().any(char::is_control) {
        return Err(DomainError::NotOneLine);
    }
    Ok(trimmed.to_owned())
}
