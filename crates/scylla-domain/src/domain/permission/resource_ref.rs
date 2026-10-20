use crate::domain::ids::{
    AppCredentialId, AppId, GrantId, JobId, OrganizationId, PipelineId, ProjectId, SecretId,
    TriggerId, UserId,
};
use crate::domain::role::RoleName;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResourceRef {
    System,
    User(UserId),
    Organization(OrganizationId),
    Project(ProjectId),
    Pipeline(PipelineId),
    Job(JobId),
    Secret(SecretId),
    Trigger(TriggerId),
    App(AppId),
    AppSecret(AppCredentialId),
    Grant(GrantId),
    Role(RoleName),
}

impl ResourceRef {
    #[must_use]
    pub fn kind(&self) -> &'static str {
        match self {
            Self::System => "system",
            Self::User(_) => "user",
            Self::Organization(_) => "organization",
            Self::Project(_) => "project",
            Self::Pipeline(_) => "pipeline",
            Self::Job(_) => "job",
            Self::Secret(_) => "secret",
            Self::Trigger(_) => "trigger",
            Self::App(_) => "app",
            Self::AppSecret(_) => "app_secret",
            Self::Grant(_) => "grant",
            Self::Role(_) => "role",
        }
    }
}

impl std::fmt::Display for ResourceRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::System => write!(f, "{}", self.kind()),
            Self::User(id) => write!(f, "{}:{}", self.kind(), id.as_str()),
            Self::Organization(id) => write!(f, "{}:{}", self.kind(), id.as_str()),
            Self::Project(id) => write!(f, "{}:{}", self.kind(), id.as_str()),
            Self::Pipeline(id) => write!(f, "{}:{}", self.kind(), id.as_str()),
            Self::Job(id) => write!(f, "{}:{}", self.kind(), id.as_str()),
            Self::Secret(id) => write!(f, "{}:{}", self.kind(), id.as_str()),
            Self::Trigger(id) => write!(f, "{}:{}", self.kind(), id.as_str()),
            Self::App(id) => write!(f, "{}:{}", self.kind(), id.as_str()),
            Self::AppSecret(id) => write!(f, "{}:{}", self.kind(), id.as_str()),
            Self::Grant(id) => write!(f, "{}:{}", self.kind(), id.as_str()),
            Self::Role(id) => write!(f, "{}:{}", self.kind(), id.as_str()),
        }
    }
}
