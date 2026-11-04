//! Unit tests of the permission model: the schema of the parts, and the typed values.

use crate::authz::{Model, Schema, SchemaError};
use crate::identity::actors::{Actor, ActorId};
use crate::identity::agents::AgentCaller;
use crate::identity::apps::{App, AppHolder};
use crate::projects::pipelines::{PipelineAccess, PipelineHolder};
use crate::projects::triggers::{TriggerAccess, TriggerHolder};
use crate::projects::{Project, ProjectCaller, ProjectHolder, ProjectId};
use crate::tenancy::grants::AccessCaller;
use crate::tenancy::workspaces::{Workspace, WorkspaceCaller, WorkspaceId};

crate::authz! {
    /// An auditor of the projects, as an edition adds it.
    extend Project as ProjectAudit {
        /// An actor that audits the project.
        relation auditor: Actor;
        /// Read the audit log: the auditors and the project admins.
        permission audit = auditor + delete;
    }
}

/// The schema of the core, in the order of the core plugins. It holds only roles: the owners
/// of the workspaces are in Postgres.
const CORE: &str = "\
definition actor {
    permission create_workspace = nil
    permission list_workspaces = nil
    permission accept_invitation = nil
    permission open_stream = nil
    permission list_projects = nil
    permission check_permissions = nil
}

definition workspace {
    relation admin: actor
    relation member: actor
    relation viewer: actor
    relation runner: actor
    permission manage = admin
    permission work = manage + member
    permission read = work + viewer
    permission get_workspace = read
    permission list_workspaces = read
    permission update_workspace = manage
    permission delete_workspace = nil
    permission add_owner = nil
    permission remove_owner = nil
    permission list_owners = read
    permission grant = admin
    permission revoke = admin
    permission list_access = read
    permission create_invitation = manage
    permission list_invitations = manage
    permission revoke_invitation = manage
    permission resend_invitation = manage
    permission create_app = manage
    permission get_app = read
    permission list_apps = read
    permission delete_app = manage
    permission disable_app = manage
    permission enable_app = manage
    permission create_join_code = manage
    permission add_app_key = manage
    permission revoke_app_key = manage
    permission create_project = work
    permission list_projects = read
    permission get_project = read
    permission update_project = work
    permission delete_project = manage
    permission create_secret = manage
    permission update_secret = manage
    permission delete_secret = manage
    permission list_secrets = work
    permission run_jobs = runner
    permission create_pipeline = work
    permission get_pipeline = read
    permission update_pipeline = work
    permission delete_pipeline = work
    permission list_project_pipelines = read
    permission list_workspace_pipelines = read
    permission run_pipeline = work
    permission get_job = read
    permission cancel_job = work
    permission delete_job = work
    permission rerun_job = work
    permission list_pipeline_jobs = read
    permission list_workspace_jobs = read
    permission tail_job_logs = read
    permission create_trigger = work
    permission get_trigger = read
    permission update_trigger = work
    permission delete_trigger = work
    permission set_trigger_enabled = work
    permission fire_trigger_now = work
    permission list_pipeline_triggers = read
}

definition app {
    permission get_app = nil
    permission delete_app = nil
    permission disable_app = nil
    permission enable_app = nil
    permission create_join_code = nil
    permission add_app_key = nil
    permission revoke_app_key = nil
}

definition project {
    relation admin: actor
    relation editor: actor
    relation viewer: actor
    relation runner: actor
    permission delete = admin
    permission edit = delete + editor
    permission view = edit + viewer
    permission get_project = view
    permission list_projects = view
    permission update_project = edit
    permission delete_project = delete
    permission create_secret = delete
    permission update_secret = delete
    permission delete_secret = delete
    permission list_secrets = edit
    permission grant = admin
    permission revoke = admin
    permission list_access = view
    permission run_jobs = runner
    permission create_pipeline = edit
    permission get_pipeline = view
    permission update_pipeline = edit
    permission delete_pipeline = edit
    permission list_project_pipelines = view
    permission run_pipeline = edit
    permission get_job = view
    permission cancel_job = edit
    permission delete_job = edit
    permission rerun_job = edit
    permission list_pipeline_jobs = view
    permission tail_job_logs = view
    permission create_trigger = edit
    permission get_trigger = view
    permission update_trigger = edit
    permission delete_trigger = edit
    permission set_trigger_enabled = edit
    permission fire_trigger_now = edit
    permission list_pipeline_triggers = view
}
";

/// Returns the schema of the core, in the order of the core plugins.
fn core() -> Schema {
    let mut schema = Schema::default();
    let parts = [
        Actor::part(),
        Workspace::part(),
        WorkspaceCaller::part(),
        App::part(),
        AppHolder::part(),
        AgentCaller::part(),
        Project::part(),
        ProjectHolder::part(),
        ProjectCaller::part(),
        PipelineAccess::part(),
        PipelineHolder::part(),
        TriggerAccess::part(),
        TriggerHolder::part(),
        AccessCaller::part(),
    ];
    for part in parts {
        schema.add(part).expect("core part fits");
    }
    schema
}

/// The core parts give the core schema, with one permission for each route.
#[test]
fn the_core_parts_give_the_core_schema() {
    // given
    let schema = core();

    // when
    let text = schema.to_string();

    // then
    assert_eq!(text, CORE);
}

/// An extension adds its relations and its permissions to its definition.
#[test]
fn an_extension_adds_to_its_definition() {
    // given
    let mut schema = core();

    // when
    schema.add(ProjectAudit::part()).unwrap();

    // then
    let text = schema.to_string();
    assert!(text.contains("    relation runner: actor\n    relation auditor: actor\n"));
    assert!(text.contains(
        "    permission list_pipeline_triggers = view\n    \
         permission audit = auditor + delete\n}"
    ));
}

/// The schema tells which definitions have a permission, with the extensions in them.
#[test]
fn the_schema_knows_where_a_permission_is() {
    // given
    let mut schema = core();
    schema.add(ProjectAudit::part()).unwrap();

    // when
    let on_both = (
        schema.has("workspace", "delete_project"),
        schema.has("project", "delete_project"),
    );
    let on_project = (
        schema.has("workspace", "audit"),
        schema.has("project", "audit"),
    );
    let a_relation = schema.has("project", "viewer");
    let named = (
        schema.permission("project", "audit"),
        schema.permission("project", "viewer"),
        schema.permission("nothing", "audit"),
    );

    // then
    assert_eq!(on_both, (true, true));
    assert_eq!(on_project, (false, true));
    assert!(!a_relation);
    assert_eq!(named, (Some("audit"), None, None));
}

/// A definition declared twice, an extension before its definition and a name declared two
/// times in a definition are refused.
#[test]
fn a_part_that_does_not_fit_is_refused() {
    // given
    let mut schema = core();
    let mut empty = Schema::default();

    // when
    let twice = schema.add(Project::part());
    let early = empty.add(ProjectAudit::part());
    schema.add(ProjectAudit::part()).unwrap();
    let again = schema.add(ProjectAudit::part());

    // then
    assert_eq!(twice, Err(SchemaError::Defined("project")));
    assert_eq!(early, Err(SchemaError::Undefined("project")));
    let duplicate = SchemaError::Duplicate {
        definition: "project",
        name: "auditor",
    };
    assert_eq!(again, Err(duplicate));
}

/// The typed constants make the checks and the relationships of their definition.
#[test]
fn the_typed_values_name_their_objects() {
    // given
    let project = ProjectId::generate();
    let workspace = WorkspaceId::generate();
    let actor = ActorId::generate();

    // when
    let delete = Project::DELETE_PROJECT.on(project).within(workspace);
    let viewer = Project::VIEWER.of(project, actor);
    let audit = ProjectAudit::AUDIT.on(project).at_least_as_fresh("zed");

    // then
    let expected = format!("delete_project on project:{project}");
    assert_eq!(delete.to_string(), expected);
    assert_eq!(delete.workspace(), Some(workspace));
    let expected = format!("project:{project}#viewer@actor:{actor}");
    assert_eq!(viewer.to_string(), expected);
    assert_eq!(audit.to_string(), format!("audit on project:{project}"));
    assert_eq!(audit.workspace(), None);
    assert_eq!(audit.freshness(), Some("zed"));
}
