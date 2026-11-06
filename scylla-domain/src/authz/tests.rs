//! Unit tests of the permission model: the schema of the parts, and the typed values.

use crate::authz::{Model, Schema, SchemaError};
use crate::identity::actors::{Actor, ActorId};
use crate::identity::agents::AgentCaller;
use crate::identity::apps::{App, AppHolder};
use crate::notifications::NotificationCaller;
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
const CORE: &str = include_str!("core.zed");

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
        NotificationCaller::part(),
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
