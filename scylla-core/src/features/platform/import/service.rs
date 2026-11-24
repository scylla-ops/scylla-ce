//! The run of the import, its shared state, and its report.

use std::collections::{HashMap, HashSet};
use std::fmt;

use scylla_domain::authz::Relationship;
use scylla_domain::identity::actors::{Actor, ActorId};
use scylla_domain::identity::users as user_kind;
use scylla_domain::projects::ProjectId;
use scylla_domain::tenancy::grants::{MEMBER_KIND, OWNER};
use scylla_domain::tenancy::workspaces::WorkspaceId;
use scylla_extension::{CreateContext, Gate, HookError, Hooks, Rejection};
use sqlx::PgPool;

use super::{content, legacy, tenancy, users};
use crate::features::tenancy::workspaces;
use crate::infra::secret::Sealer;
use crate::Error;

/// The number of relationships in one write to SpiceDB.
const CHUNK: usize = 200;

/// What the import did, and each row that it left with its reason.
#[derive(Debug, Default)]
pub struct Report {
    /// The users.
    pub users: usize,
    /// The instance admins.
    pub admins: usize,
    /// The workspaces of the organizations.
    pub workspaces: usize,
    /// The projects.
    pub projects: usize,
    /// The secrets.
    pub secrets: usize,
    /// The pipelines.
    pub pipelines: usize,
    /// The roles.
    pub roles: usize,
    /// Each row that the import left, with its reason.
    pub skipped: Vec<String>,
}

impl fmt::Display for Report {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(
            f,
            "imported: {} users, {} instance admins, {} workspaces, {} projects, {} secrets, {} pipelines, {} roles",
            self.users, self.admins, self.workspaces, self.projects, self.secrets, self.pipelines, self.roles
        )?;
        for line in &self.skipped {
            writeln!(f, "skipped: {line}")?;
        }
        Ok(())
    }
}

/// The state that the steps of the import share.
pub(super) struct Run<'a> {
    /// The database of this base.
    pub(super) pool: &'a PgPool,
    /// The hooks of the edition.
    pub(super) hooks: &'a Hooks,
    /// The key of the secrets of this base.
    pub(super) sealer: Option<&'a Sealer>,
    /// The key of the secrets of the old product.
    pub(super) old_sealer: Option<&'a Sealer>,
    /// The new user of each old user id.
    pub(super) users: HashMap<String, ActorId>,
    /// The handles that the import gave.
    pub(super) handles: HashSet<String>,
    /// The new workspace and its author of each old organization id.
    pub(super) workspaces: HashMap<String, (WorkspaceId, ActorId)>,
    /// The new project, its workspace and its author of each old project id.
    pub(super) projects: HashMap<String, (ProjectId, WorkspaceId, ActorId)>,
    /// The roles to write to SpiceDB after the rows.
    pub(super) roles: Vec<Relationship>,
    /// The report.
    pub(super) report: Report,
}

/// A user that the import adds to a workspace, as an owner or with a role.
pub(super) struct Member {
    /// The workspace.
    pub(super) workspace: WorkspaceId,
    /// The author of the workspace: the actor of the hooks.
    pub(super) by: ActorId,
    /// The user.
    pub(super) user: ActorId,
    /// The role in the workspace or on one of its objects, or `None` for an owner.
    pub(super) role: Option<Relationship>,
    /// The user and the place in the report, for example `bob in organization Acme`.
    pub(super) about: String,
}

impl Run<'_> {
    /// Adds a user to a workspace after the hooks of a new member, as `Grant` does: an owner in
    /// the transaction of the hooks, a role in the roles to write after the rows. A member that
    /// a hook refuses stays out, and the report names it.
    pub(super) async fn join(&mut self, member: Member) -> Result<(), Error> {
        // Run the hooks of a new member.
        let (object, name) = match &member.role {
            Some(role) => (Some(role.object().to_string()), role.relation()),
            None => (None, OWNER),
        };
        let actor = Actor {
            id: member.by,
            kind: user_kind::KIND,
        };
        let hook = CreateContext::new(MEMBER_KIND, actor, member.workspace).with_member(
            member.user,
            object,
            name,
        );
        let mut tx = self.pool.begin().await?;
        match self.hooks.before_create(&mut tx, &hook).await {
            Ok(()) => {}
            Err(HookError::Rejected(refusal)) => {
                let line = format!("role {name} of {}: {}", member.about, refusal.message);
                self.report.skipped.push(line);
                return Ok(());
            }
            Err(error) => return Err(error.into()),
        }

        // Add an owner, or keep the role to write after the rows.
        match member.role {
            Some(role) => self.roles.push(role),
            None => {
                workspaces::add_owner(&mut tx, member.workspace, member.user).await?;
            }
        }
        tx.commit().await?;
        Ok(())
    }
}

/// Imports the rows of the old database into this base, which must have no user, then writes
/// the roles. Each owner and each role runs the hooks of a new member first. A row that does not
/// fit is left and named in the report.
pub async fn import(
    old: &PgPool,
    pool: &PgPool,
    gate: &dyn Gate,
    hooks: &Hooks,
    sealer: Option<&Sealer>,
    old_sealer: Option<&Sealer>,
) -> Result<Report, Error> {
    // Refuse a base that has users.
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM users")
        .fetch_one(pool)
        .await?;
    if count > 0 {
        let message = "the import needs a base with no user";
        return Err(Error::Rejected(Rejection::new(
            "IMPORT_TARGET_NOT_EMPTY",
            message,
        )));
    }

    // Read the old rows, then import each kind in the order of the references.
    let legacy = legacy::read(old).await?;
    let mut run = Run {
        pool,
        hooks,
        sealer,
        old_sealer,
        users: HashMap::new(),
        handles: HashSet::new(),
        workspaces: HashMap::new(),
        projects: HashMap::new(),
        roles: Vec::new(),
        report: Report::default(),
    };
    users::import(&mut run, &legacy).await?;
    tenancy::import(&mut run, &legacy).await?;
    content::import(&mut run, &legacy).await?;

    // Write the roles to SpiceDB.
    for chunk in run.roles.chunks(CHUNK) {
        gate.write(chunk).await?;
    }
    run.report.roles = run.roles.len();
    tracing::info!(
        event = "import.done",
        users = run.report.users,
        skipped = run.report.skipped.len()
    );
    Ok(run.report)
}
