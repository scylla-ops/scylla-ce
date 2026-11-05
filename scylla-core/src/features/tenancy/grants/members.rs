//! The gRPC routes `ListMembers`, `GetMemberAccess` and `ListHeldPermissions` of `AccessService`.

use std::collections::HashMap;

use scylla_domain::authz::{Check, Object, Resource};
use scylla_domain::identity::actors::ActorId;
use scylla_domain::platform::{Platform, PlatformId};
use scylla_domain::projects::{Project, ProjectId};
use scylla_domain::tenancy::catalog::{self, Level};
use scylla_domain::tenancy::grants::{
    AccessCaller, GrantScope, HeldRole, Member, ProjectMembers, WorkspaceMembers,
};
use scylla_domain::tenancy::workspaces::{Workspace, WorkspaceId};
use scylla_proto::access::v1 as pb;
use tonic::Request;
use uuid::Uuid;

use super::effective::effective;
use super::grpc::{not_an_id, GrantsGrpc};
use super::{held, store};
use crate::api::parse::{self, timestamp, ToProto};
use crate::Error;

/// An object with members: a workspace, or a project of a workspace.
struct Membered {
    /// The workspace.
    workspace: WorkspaceId,
    /// The project.
    project: Option<ProjectId>,
    /// The id of the object, as text.
    id: String,
}

/// The route on the members of an object.
#[derive(Clone, Copy)]
enum MemberRoute {
    /// `ListMembers`.
    List,
    /// `GetMemberAccess`.
    Access,
}

/// Makes the check of a route on a workspace or on a project. Another kind is a request error.
fn member_check(route: MemberRoute, kind: &str, id: &str) -> Result<Check, Error> {
    match kind {
        Workspace::NAME => {
            let id = WorkspaceId::parse(id).map_err(|_| not_an_id())?;
            Ok(match route {
                MemberRoute::List => WorkspaceMembers::LIST_MEMBERS.on(id),
                MemberRoute::Access => WorkspaceMembers::GET_MEMBER_ACCESS.on(id),
            })
        }
        Project::NAME => {
            let id = ProjectId::parse(id).map_err(|_| not_an_id())?;
            Ok(match route {
                MemberRoute::List => ProjectMembers::LIST_MEMBERS.on(id),
                MemberRoute::Access => ProjectMembers::GET_MEMBER_ACCESS.on(id),
            })
        }
        _ => Err(Error::invalid("object.kind", "has no members")),
    }
}

impl GrantsGrpc {
    /// Checks a route on the object of a request, and finds its workspace.
    async fn membered(
        &self,
        caller: &scylla_extension::Principal,
        object: Option<pb::ObjectRef>,
        route: MemberRoute,
    ) -> Result<Membered, Error> {
        let (kind, name, id) = self.target(object)?;
        let check = member_check(route, &name, &id)?;
        let (check, workspace) = self.within(caller, kind, &id, check).await?;
        self.gate.authorize(caller, check).await?;
        let project = (name == Project::NAME)
            .then(|| ProjectId::parse(&id).map_err(|_| not_an_id()))
            .transpose()?;
        Ok(Membered {
            workspace,
            project,
            id,
        })
    }

    /// Returns `true` if the caller can read the members of a workspace. It records nothing on the
    /// operation.
    async fn reads_workspace(
        &self,
        caller: &scylla_extension::Principal,
        workspace: WorkspaceId,
    ) -> Result<bool, Error> {
        let answers = self
            .gate
            .permissions(
                caller,
                &Object::of(workspace),
                Some(workspace),
                &[WorkspaceMembers::LIST_MEMBERS.name()],
            )
            .await?;
        Ok(answers.first().copied().unwrap_or(false))
    }

    /// Handles `ListMembers`.
    pub(super) async fn members(
        &self,
        request: Request<pb::ListMembersRequest>,
    ) -> Result<pb::ListMembersResponse, Error> {
        let caller = self.gate.caller(&request).await?;
        let object = request.into_inner().object;
        let target = self.membered(&caller, object, MemberRoute::List).await?;

        // Read the roles on the scopes that the caller can read, then the instance admins.
        let gate = self.gate.as_ref();
        let mut roles = match target.project {
            Some(project) => {
                let with_workspace = self.reads_workspace(&caller, target.workspace).await?;
                held::on_project(&self.pool, gate, target.workspace, project, with_workspace)
                    .await?
            }
            None => held::in_workspace(&self.pool, gate, target.workspace).await?,
        };
        roles.extend(held::instance_admins(&self.pool).await?);
        let members = held::members(&self.pool, roles).await?;
        Ok(pb::ListMembersResponse {
            members: members.into_iter().map(ToProto::to_proto).collect(),
        })
    }

    /// Handles `GetMemberAccess`.
    pub(super) async fn member_access(
        &self,
        request: Request<pb::GetMemberAccessRequest>,
    ) -> Result<pb::GetMemberAccessResponse, Error> {
        let caller = self.gate.caller(&request).await?;
        let request = request.into_inner();
        let actor: ActorId = parse::required(request.actor_id, "actor_id")?;
        let target = self
            .membered(&caller, request.object, MemberRoute::Access)
            .await?;

        // Read the roles of the actor, and the permissions of those that are rows.
        let gate = self.gate.as_ref();
        let roles = held::of_actor(&self.pool, gate, actor, target.workspace).await?;
        let ids: Vec<Uuid> = roles
            .iter()
            .filter_map(|role| Uuid::parse_str(&role.key).ok())
            .collect();
        let mut tx = self.pool.begin().await?;
        let rows: HashMap<Uuid, Vec<String>> = store::permissions_of_roles(&mut tx, &ids).await?;
        tx.commit().await?;

        // Find what each role gives on the object.
        let level = match target.project {
            Some(_) => Level::Project,
            None => Level::Workspace,
        };
        let permissions = effective(&self.schema, &roles, &rows, level, &target.id)
            .into_iter()
            .map(|one| pb::PermissionAccess {
                permission: one.permission.to_owned(),
                sources: one.sources.iter().map(source).collect(),
            })
            .collect();

        // Show only the roles that the caller can read.
        let visible = if self.reads_workspace(&caller, target.workspace).await? {
            roles
        } else {
            roles
                .into_iter()
                .filter(|role| role.scope.id == target.id || role.scope.level == Level::Platform)
                .collect()
        };
        let member = held::members(&self.pool, visible).await?.into_iter().next();
        Ok(pb::GetMemberAccessResponse {
            member: member.map(ToProto::to_proto),
            permissions,
        })
    }

    /// Handles `ListHeldPermissions`. An unknown object has none.
    pub(super) async fn held_permissions(
        &self,
        request: Request<pb::ListHeldPermissionsRequest>,
    ) -> Result<pb::ListHeldPermissionsResponse, Error> {
        // Authenticate the caller, then read the object.
        let caller = self.gate.caller(&request).await?;
        let object = request
            .into_inner()
            .object
            .ok_or_else(|| Error::invalid("object", "is required"))?;
        let id = object
            .id
            .map(|id| id.value)
            .ok_or_else(|| Error::invalid("object.id", "is required"))?;
        let level = Level::parse(&object.kind)
            .ok_or_else(|| Error::invalid("object.kind", "has no permissions to hold"))?;

        // Check the route permission on the caller itself.
        self.gate
            .authorize(
                &caller,
                AccessCaller::LIST_HELD_PERMISSIONS.on(caller.actor.id),
            )
            .await?;

        // Ask for each permission of the level. An unknown object has none.
        let located = if object.kind == Platform::NAME {
            let platform = PlatformId::parse(&id).map_err(|_| not_an_id())?;
            (platform == Platform::ID).then(|| (Object::of(platform), None))
        } else {
            let kind = self
                .grants
                .get(&object.kind)
                .ok_or_else(|| Error::invalid("object.kind", "has no permissions to hold"))?;
            let found = kind.object(&id).ok_or_else(not_an_id)?;
            let search = kind
                .workspace(self.pool.clone(), &id)
                .ok_or_else(not_an_id)?;
            search.await?.map(|workspace| (found, Some(workspace)))
        };
        let Some((found, workspace)) = located else {
            return Ok(pb::ListHeldPermissionsResponse::default());
        };
        let ids: Vec<&'static str> = catalog::areas()
            .filter(|area| !area.machine && level.covers(area.level))
            .flat_map(|area| area.permissions)
            .map(|permission| permission.id)
            .collect();
        let answers = self.gate.holds(&caller, &found, workspace, &ids).await?;
        Ok(pb::ListHeldPermissionsResponse {
            permissions: ids
                .into_iter()
                .zip(answers)
                .filter(|(_, holds)| *holds)
                .map(|(id, _)| id.to_owned())
                .collect(),
        })
    }
}

/// Makes the id message of a scope.
fn scope_id(scope: &GrantScope) -> scylla_proto::common::v1::Id {
    scylla_proto::common::v1::Id {
        value: scope.id.clone(),
    }
}

/// Makes the message of the role that gives a permission.
fn source(role: &HeldRole) -> pb::AccessSource {
    pb::AccessSource {
        scope_kind: role.scope.level.as_str().to_owned(),
        scope_id: Some(scope_id(&role.scope)),
        scope_name: role.scope.name.clone(),
        role_key: role.key.clone(),
        role_name: role.name.clone(),
    }
}

impl ToProto<pb::MemberRole> for HeldRole {
    fn to_proto(self) -> pb::MemberRole {
        pb::MemberRole {
            scope_kind: self.scope.level.as_str().to_owned(),
            scope_id: Some(scope_id(&self.scope)),
            scope_name: self.scope.name,
            role: self.role,
            role_key: self.key,
            role_name: self.name,
            granted_at: self.granted_at.map(timestamp),
            granted_by: self.granted_by.map(ToProto::to_proto),
        }
    }
}

impl ToProto<pb::Member> for Member {
    fn to_proto(self) -> pb::Member {
        pb::Member {
            actor_id: Some(self.profile.actor_id.to_proto()),
            actor_kind: self.profile.actor_kind.as_str().to_owned(),
            display_name: self.profile.display_name,
            handle: self.profile.handle.unwrap_or_default(),
            roles: self.roles.into_iter().map(ToProto::to_proto).collect(),
        }
    }
}
