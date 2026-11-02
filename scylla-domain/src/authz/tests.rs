//! Unit tests of the permission model: the schema of the parts, and the typed values.

use crate::actors::{Actor, ActorId};
use crate::authz::{Model, Schema, SchemaError};
use crate::projects::{Project, ProjectId};

crate::authz! {
    /// An auditor of the projects, as an edition adds it.
    extend Project as ProjectAudit {
        /// An actor that audits the project.
        relation auditor: Actor;
        /// Read the audit log: the auditors, and the actors that can delete the project.
        permission audit = auditor->act_for + delete;
    }
}

/// The schema of the core definitions.
const CORE: &str = "\
definition actor {
    relation identity: actor
    relation delegate: actor
    permission act_for = identity + delegate
}

definition project {
    relation owner: actor
    relation editor: actor
    relation viewer: actor
    permission delete = owner->act_for
    permission edit = delete + editor->act_for
    permission view = edit + viewer->act_for
}
";

/// Returns the schema of the core definitions.
fn core() -> Schema {
    let mut schema = Schema::default();
    schema.add(Actor::part()).expect("actor fits");
    schema.add(Project::part()).expect("project fits");
    schema
}

/// The core parts give the core schema, in the SpiceDB schema language.
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
    assert!(text.contains("    relation viewer: actor\n    relation auditor: actor\n"));
    assert!(text.contains(
        "    permission view = edit + viewer->act_for\n    \
         permission audit = auditor->act_for + delete\n}"
    ));
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
    let actor = ActorId::generate();

    // when
    let delete = Project::DELETE.on(project);
    let owner = Project::OWNER.of(project, actor);
    let audit = ProjectAudit::AUDIT.on(project).at_least_as_fresh("zed");

    // then
    assert_eq!(delete.to_string(), format!("delete on project:{project}"));
    assert_eq!(
        owner.to_string(),
        format!("project:{project}#owner@actor:{actor}")
    );
    assert_eq!(audit.to_string(), format!("audit on project:{project}"));
    assert_eq!(audit.freshness(), Some("zed"));
}
