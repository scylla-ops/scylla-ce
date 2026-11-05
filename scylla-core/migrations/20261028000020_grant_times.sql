-- The time and the author of a grant of a role of the schema. The role itself is in SpiceDB.
CREATE TABLE relation_grants (
    actor_id uuid NOT NULL REFERENCES actors (id),
    scope_kind text NOT NULL CHECK (scope_kind IN ('workspace', 'project')),
    scope_id uuid NOT NULL,
    relation text NOT NULL,
    created_by uuid REFERENCES actors (id),
    created_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (actor_id, scope_kind, scope_id, relation)
);

CREATE INDEX relation_grants_scope ON relation_grants (scope_kind, scope_id);
CREATE INDEX role_grants_scope ON role_grants (scope_kind, scope_id);
