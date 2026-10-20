use bon::bon;
use chrono::{DateTime, Utc};

use crate::domain::clock;
use crate::domain::ids::{OrganizationId, ProjectId};
use crate::domain::organization::Organization;
use crate::domain::project::Project;
use crate::domain::project::{ProjectDescription, ProjectName};

pub struct ProjectBuilder;

#[bon]
#[allow(clippy::new_ret_no_self, clippy::must_use_candidate)]
impl ProjectBuilder {
    #[builder(start_fn = new, finish_fn = build)]
    pub fn assemble(
        #[builder(start_fn)] org: &Organization,
        #[builder(start_fn, into)] name: String,
        id: Option<ProjectId>,
        #[builder(into)] description: Option<String>,
        #[builder(default = true)] is_active: bool,
        created_at: Option<DateTime<Utc>>,
        updated_at: Option<DateTime<Utc>>,
        #[builder(default = 0)] version: u64,
    ) -> Project {
        Self::assemble_from_org_id(
            org.id().clone(),
            name,
            id,
            description,
            is_active,
            created_at,
            updated_at,
            version,
        )
    }

    #[builder(start_fn = for_org_id, finish_fn = build)]
    pub fn assemble_from_org_id(
        #[builder(start_fn)] organization_id: OrganizationId,
        #[builder(start_fn, into)] name: String,
        id: Option<ProjectId>,
        #[builder(into)] description: Option<String>,
        #[builder(default = true)] is_active: bool,
        created_at: Option<DateTime<Utc>>,
        updated_at: Option<DateTime<Utc>>,
        #[builder(default = 0)] version: u64,
    ) -> Project {
        let now = created_at.unwrap_or_else(clock::now);
        let description =
            description.map(|s| ProjectDescription::new(s).expect("test project desc invalid"));
        Project::from_persistence(
            id.unwrap_or_else(ProjectId::generate),
            ProjectName::new(name).expect("test project name invalid"),
            description,
            organization_id,
            is_active,
            now,
            updated_at.unwrap_or(now),
            version,
        )
    }
}

#[must_use]
pub fn project(org: &Organization, name: &str) -> Project {
    ProjectBuilder::new(org, name).build()
}
