//! The reports of the runs on the commits: the outbox of the states of the jobs, the sender that
//! posts a GitHub check run or a commit status on GitHub or GitLab, `VcsReportService`, and the
//! health of the connections (a refused call, a GitLab token that expires).

mod checks;
mod convert;
mod failures;
mod github;
mod gitlab;
mod grpc;
mod health;
mod mount;
pub(crate) mod notices;
mod outbox;
mod output;
mod sender;
mod service;
mod states;
mod store;

#[cfg(test)]
mod tests;

pub(crate) use checks::reruns;
pub(crate) use mount::mount;
pub(crate) use outbox::job_changed;
