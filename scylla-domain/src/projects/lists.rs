//! The list of the projects: the search text, the order and the health filter of a request, the
//! facts of each project that they read, and one page with the counts of all the pages.

#[cfg(test)]
mod tests;

use std::cmp::Ordering;

use time::OffsetDateTime;

use super::{Project, ProjectId};
use crate::tenancy::workspaces::WorkspaceId;
use crate::{DomainError, PageRequest};

/// The search text of a project list: trimmed, at most `MAX_LEN` characters, kept in lower
/// case. An empty text keeps each project.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ProjectSearch(String);

impl ProjectSearch {
    /// The maximum number of characters, after trim.
    pub const MAX_LEN: usize = 100;

    /// Trims the text and checks its length.
    pub fn parse(raw: String) -> Result<Self, DomainError> {
        let trimmed = raw.trim();
        if trimmed.chars().count() > Self::MAX_LEN {
            return Err(DomainError::TooLong { max: Self::MAX_LEN });
        }
        Ok(Self(lower(trimmed)))
    }

    /// Returns `true` when the text is in the name, the description, the repository or the name
    /// of a pipeline of a project, with no difference between upper and lower case.
    fn keeps(&self, listed: &ListedProject) -> bool {
        let has = |text: &str| lower(text).contains(&self.0);
        self.0.is_empty()
            || has(&listed.project.name)
            || has(&listed.project.description)
            || listed.repository.as_deref().is_some_and(has)
            || listed.pipelines.iter().any(|name| has(name))
    }
}

/// The order of a project list.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ProjectOrder {
    /// The order that the members of the workspace set, then the projects with no place,
    /// oldest first.
    #[default]
    Position,
    /// The name, with no difference between upper and lower case.
    Name,
    /// The project with the newest job first. No job comes last.
    LastJob,
    /// The failing projects, then the running projects, then the others. In each group, the
    /// name.
    FailedFirst,
}

/// A filter of a project list by health.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ProjectHealth {
    /// Each project.
    #[default]
    All,
    /// Only the projects with at least one failing pipeline.
    Failed,
    /// Only the projects with a job that runs or waits.
    Running,
}

/// The input of a project list: the projects that the caller can see.
#[derive(Clone, Debug, Default)]
pub struct ListProjects {
    /// Only the projects of this workspace, if it is set.
    pub workspace_id: Option<WorkspaceId>,
    /// Only the projects that have this text.
    pub search: ProjectSearch,
    /// The order of the projects.
    pub order: ProjectOrder,
    /// Only the projects of this health.
    pub health: ProjectHealth,
    /// The page to return.
    pub page: PageRequest<ProjectId>,
}

/// A project with the facts that the search, the order and the health read.
#[derive(Clone, Debug)]
pub struct ListedProject {
    /// The project.
    pub project: Project,
    /// The full name of the linked repository, if the project has one.
    pub repository: Option<String>,
    /// The names of the pipelines of the project.
    pub pipelines: Vec<String>,
    /// The number of pipelines whose last ended job did not complete.
    pub failing: u32,
    /// `true` when a job of the project runs or waits.
    pub running: bool,
    /// The creation time of the newest job of the project.
    pub last_job_at: Option<OffsetDateTime>,
}

impl ListedProject {
    /// Returns `true` when the project has this health.
    fn has(&self, health: ProjectHealth) -> bool {
        match health {
            ProjectHealth::All => true,
            ProjectHealth::Failed => self.failing > 0,
            ProjectHealth::Running => self.running,
        }
    }

    /// Returns the group of the order with the failing projects first: failing, running, other.
    fn health_group(&self) -> u8 {
        match (self.failing > 0, self.running) {
            (true, _) => 0,
            (false, true) => 1,
            (false, false) => 2,
        }
    }

    /// Compares the names of two projects with no difference between upper and lower case.
    fn compare_names(&self, other: &Self) -> Ordering {
        lower(&self.project.name).cmp(&lower(&other.project.name))
    }
}

impl ProjectOrder {
    /// Compares two projects in this order. Two projects with the same keys go in id order,
    /// which is the order of creation.
    pub fn compare(self, left: &ListedProject, right: &ListedProject) -> Ordering {
        let keys = match self {
            Self::Position => Ordering::Equal,
            Self::Name => left.compare_names(right),
            Self::LastJob => match (left.last_job_at, right.last_job_at) {
                (Some(left_at), Some(right_at)) => right_at.cmp(&left_at),
                (Some(_), None) => Ordering::Less,
                (None, Some(_)) => Ordering::Greater,
                (None, None) => Ordering::Equal,
            }
            .then_with(|| left.compare_names(right)),
            Self::FailedFirst => left
                .health_group()
                .cmp(&right.health_group())
                .then_with(|| left.compare_names(right)),
        };
        keys.then_with(|| {
            let id = |listed: &ListedProject| listed.project.id.to_bytes();
            id(left).cmp(&id(right))
        })
    }
}

/// The counts of a project list, on all the pages.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ProjectCounts {
    /// The number of projects.
    pub total: u32,
    /// The number of projects with at least one failing pipeline.
    pub failed: u32,
    /// The number of projects with a job that runs or waits.
    pub running: u32,
    /// The number of pipelines of these projects.
    pub pipelines: u32,
}

/// One page of projects, with the counts of the projects that the search keeps.
#[derive(Clone, Debug)]
pub struct ProjectPage {
    /// The projects of the page, in the order of the request.
    pub items: Vec<Project>,
    /// The last project of the page, if a next page exists.
    pub next_after: Option<ProjectId>,
    /// The counts of the projects that the search keeps. The health filter does not change
    /// them.
    pub counts: ProjectCounts,
}

impl ProjectPage {
    /// Keeps the projects of the search, counts them, keeps the projects of the health, sorts
    /// them, and keeps the page after the project of the request. A project that is not in the
    /// list gives an empty page.
    pub fn of(all: Vec<ListedProject>, query: &ListProjects) -> Self {
        // Keep the projects of the search, and count them.
        let found: Vec<_> = all
            .into_iter()
            .filter(|listed| query.search.keeps(listed))
            .collect();
        let count = |value: usize| u32::try_from(value).unwrap_or(u32::MAX);
        let counts = ProjectCounts {
            total: count(found.len()),
            failed: count(found.iter().filter(|listed| listed.failing > 0).count()),
            running: count(found.iter().filter(|listed| listed.running).count()),
            pipelines: count(found.iter().map(|listed| listed.pipelines.len()).sum()),
        };

        // Keep the projects of the health, then sort them.
        let mut kept: Vec<_> = found
            .into_iter()
            .filter(|listed| listed.has(query.health))
            .collect();
        kept.sort_by(|left, right| query.order.compare(left, right));

        // Keep the projects after the project of the request, then one page of them.
        let start = match query.page.after {
            None => 0,
            Some(after) => kept
                .iter()
                .position(|listed| listed.project.id == after)
                .map_or(kept.len(), |index| index + 1),
        };
        let mut items: Vec<_> = kept
            .into_iter()
            .skip(start)
            .map(|listed| listed.project)
            .collect();
        let size = query.page.size.get();
        let next_after = (items.len() > size).then(|| items[size - 1].id);
        items.truncate(size);
        Self {
            items,
            next_after,
            counts,
        }
    }
}

/// Returns a text in lower case.
fn lower(text: &str) -> String {
    text.chars().flat_map(char::to_lowercase).collect()
}
