//! The core piece of the SpiceDB schema.

/// The actors and the projects. `act_for` gives the actor itself and its delegates.
pub const CORE_SCHEMA: &str = r#"
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
"#;
