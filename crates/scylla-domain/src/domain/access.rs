use crate::domain::permission::Permission;
use std::fmt;
use std::slice;

/// Who may run a use case. `Public` and `Authenticated` ask no permission; each permission of
/// `Requires` or `RequiresAll` is checked, in order, and the first refusal stops.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Access {
    Public,
    Authenticated,
    Requires(Permission),
    RequiresAll(Vec<Permission>),
}

impl Access {
    #[must_use]
    pub fn permissions(&self) -> &[Permission] {
        match self {
            Self::Public | Self::Authenticated => &[],
            Self::Requires(permission) => slice::from_ref(permission),
            Self::RequiresAll(permissions) => permissions,
        }
    }
}

impl fmt::Display for Access {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Public => f.write_str("public"),
            Self::Authenticated => f.write_str("authenticated"),
            Self::Requires(_) | Self::RequiresAll(_) => {
                let keys: Vec<_> = self.permissions().iter().map(Permission::key).collect();
                f.write_str(&keys.join("+"))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Access;
    use crate::domain::ids::{JobId, OrganizationId, PipelineId, TriggerId, UserId};
    use crate::domain::permission::Permission;

    #[test]
    fn the_display_of_access_keeps_its_literal_text() {
        let org = OrganizationId::new("org-1");
        assert_eq!(Access::Public.to_string(), "public");
        assert_eq!(Access::Authenticated.to_string(), "authenticated");
        assert_eq!(
            Access::Requires(Permission::ReadOrganization(org.clone())).to_string(),
            "readOrganization"
        );
        assert_eq!(
            Access::RequiresAll(vec![
                Permission::ReadOrganization(org.clone()),
                Permission::ManageOrgGrants(org.clone()),
            ])
            .to_string(),
            "readOrganization+manageOrgGrants"
        );
        assert_eq!(
            Access::RequiresAll(vec![
                Permission::ManageOrgGrants(org.clone()),
                Permission::ReadOrganization(org),
            ])
            .to_string(),
            "manageOrgGrants+readOrganization"
        );

        let user = UserId::new("u1");
        let pipeline = PipelineId::new("p1");
        let trigger = TriggerId::new("t1");
        let job = JobId::new("j1");

        assert_eq!(
            Access::RequiresAll(vec![Permission::UpdateUser(user), Permission::CreateUser])
                .to_string(),
            "updateUser+createUser"
        );
        assert_eq!(
            Access::RequiresAll(vec![
                Permission::ManageTriggers(pipeline.clone()),
                Permission::RunPipeline(pipeline),
            ])
            .to_string(),
            "manageTriggers+runPipeline"
        );
        assert_eq!(
            Access::RequiresAll(vec![
                Permission::ManageTrigger(trigger.clone()),
                Permission::RunTriggerPipeline(trigger),
            ])
            .to_string(),
            "manageTriggers+runPipeline"
        );
        assert_eq!(
            Access::RequiresAll(vec![
                Permission::ReadJobLogs(job.clone()),
                Permission::ReadJob(job),
            ])
            .to_string(),
            "readJobLogs+readJob"
        );
    }
}
