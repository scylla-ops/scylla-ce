//! The samples of the use of the machine of an agent: the CPU and the memory that the agent sends
//! on its stream, and their figures in a period and in its buckets of hours, days or weeks in the
//! time zone of the workspace.

use time::{Duration, OffsetDateTime};

use super::place;
use crate::identity::apps::AppId;
use crate::projects::jobs::FiguresPeriod;
use crate::{DomainError, Span, TimeZone};

/// The time between two samples that the server asks of an agent.
pub const SAMPLE_INTERVAL: Duration = Duration::seconds(60);

/// The shortest time between two samples of an agent that the server keeps.
pub const SAMPLE_GAP: Duration = Duration::seconds(30);

/// The time that the server keeps a sample.
pub const SAMPLE_RETENTION: Duration = Duration::days(7);

/// A sample of the use of the machine of an agent, as the agent sends it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sample {
    /// The use of all the CPUs of the machine, in percent: 0 to 100.
    pub cpu_percent: f32,
    /// The memory in use on the machine, in bytes.
    pub memory_used_bytes: u64,
}

impl Sample {
    /// Refuses a CPU use that is not a number, and moves a CPU use outside 0 to 100 to the
    /// nearest bound.
    pub fn parse(cpu_percent: f32, memory_used_bytes: u64) -> Result<Self, DomainError> {
        if !cpu_percent.is_finite() {
            return Err(DomainError::OutOfRange { min: 0, max: 100 });
        }
        Ok(Self {
            cpu_percent: cpu_percent.clamp(0.0, 100.0),
            memory_used_bytes,
        })
    }
}

/// A sample with the time when the server got it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HostSample {
    /// The time when the server got the sample.
    pub at: OffsetDateTime,
    /// The sample.
    pub sample: Sample,
}

/// The use of the machine of an agent in a period or in a bucket. With no sample, each value is 0.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct MetricsFigures {
    /// The number of samples.
    pub samples: u32,
    /// The mean CPU use, in percent.
    pub cpu_avg: f32,
    /// The largest CPU use of one sample, in percent.
    pub cpu_max: f32,
    /// The mean memory in use, in bytes.
    pub memory_avg: u64,
    /// The largest memory in use of one sample, in bytes.
    pub memory_max: u64,
}

/// The use of the machine of an agent in one hour, day or week.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MetricsBucket {
    /// The start in the time zone of the workspace.
    pub start: OffsetDateTime,
    /// The samples of the bucket.
    pub figures: MetricsFigures,
}

/// The input of the use of the machine of an agent.
#[derive(Clone, Copy, Debug)]
pub struct MetricsQuery {
    /// The agent.
    pub agent: AppId,
    /// The period.
    pub period: FiguresPeriod,
    /// The size of the buckets, or `None` for no bucket.
    pub bucket: Option<Span>,
}

/// The sums of the samples of one part of the period, as the store counts them.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SampleSums {
    /// The number of samples.
    pub samples: u32,
    /// The sum of the CPU uses, in percent.
    pub cpu_sum: f64,
    /// The largest CPU use, in percent.
    pub cpu_max: f32,
    /// The sum of the memory in use, in bytes.
    pub memory_sum: f64,
    /// The largest memory in use, in bytes.
    pub memory_max: u64,
}

impl SampleSums {
    /// Returns the sums of the samples of two parts.
    fn add(self, other: Self) -> Self {
        Self {
            samples: self.samples.saturating_add(other.samples),
            cpu_sum: self.cpu_sum + other.cpu_sum,
            cpu_max: self.cpu_max.max(other.cpu_max),
            memory_sum: self.memory_sum + other.memory_sum,
            memory_max: self.memory_max.max(other.memory_max),
        }
    }

    /// Returns the figures of the samples: the means are the sums divided by the number.
    fn figures(self) -> MetricsFigures {
        if self.samples == 0 {
            return MetricsFigures::default();
        }
        let count = f64::from(self.samples);
        MetricsFigures {
            samples: self.samples,
            cpu_avg: (self.cpu_sum / count) as f32,
            cpu_max: self.cpu_max,
            memory_avg: (self.memory_sum / count).round() as u64,
            memory_max: self.memory_max,
        }
    }
}

/// The use of the machine of an agent in a period.
#[derive(Clone, Debug, PartialEq)]
pub struct AgentMetrics {
    /// The samples of the period.
    pub totals: MetricsFigures,
    /// Each bucket of the period, oldest first.
    pub buckets: Vec<MetricsBucket>,
    /// The time zone of the buckets.
    pub time_zone: TimeZone,
}

impl AgentMetrics {
    /// Makes the figures from the sums of each part with a sample: number 1 is the first part,
    /// and part `n` is the bucket of `starts[n - 1]`. With no bucket, the period is part 1.
    pub fn of_parts(
        starts: &[OffsetDateTime],
        found: &[(usize, SampleSums)],
        time_zone: TimeZone,
    ) -> Self {
        let parts = place(starts.len().max(1), found);
        let buckets = starts
            .iter()
            .zip(&parts)
            .map(|(start, sums)| MetricsBucket {
                start: *start,
                figures: sums.figures(),
            })
            .collect();
        let totals = parts
            .iter()
            .fold(SampleSums::default(), |all, sums| all.add(*sums));
        Self {
            totals: totals.figures(),
            buckets,
            time_zone,
        }
    }
}
