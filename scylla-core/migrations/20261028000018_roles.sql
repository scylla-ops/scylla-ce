-- The roles that are rows: custom roles, and the built-in role of the workspace creators.
CREATE TABLE roles (
    id uuid PRIMARY KEY,
    workspace_id uuid REFERENCES workspaces (id) ON DELETE CASCADE,
    level text NOT NULL CHECK (level IN ('platform', 'workspace', 'project')),
    holder text NOT NULL CHECK (holder IN ('people', 'apps')),
    name text NOT NULL,
    description text NOT NULL DEFAULT '',
    builtin boolean NOT NULL DEFAULT false,
    created_by uuid REFERENCES actors (id),
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now(),
    CHECK (workspace_id IS NULL OR level <> 'platform'),
    CHECK (level <> 'platform' OR holder = 'people')
);

CREATE UNIQUE INDEX roles_owner_name ON roles (workspace_id, lower(name)) NULLS NOT DISTINCT;

CREATE TABLE role_permissions (
    role_id uuid NOT NULL REFERENCES roles (id) ON DELETE CASCADE,
    permission text NOT NULL,
    PRIMARY KEY (role_id, permission)
);

-- A grant of a role that is a row. The roles of the schema are in SpiceDB.
CREATE TABLE role_grants (
    actor_id uuid NOT NULL REFERENCES actors (id),
    role_id uuid NOT NULL REFERENCES roles (id) ON DELETE CASCADE,
    scope_kind text NOT NULL CHECK (scope_kind IN ('platform', 'workspace', 'project')),
    scope_id uuid NOT NULL,
    created_by uuid REFERENCES actors (id),
    created_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (actor_id, role_id, scope_kind, scope_id)
);

CREATE INDEX role_grants_role ON role_grants (role_id);

INSERT INTO roles (id, level, holder, name, description, builtin)
VALUES (
    '00000000-0000-0000-0000-000000000001',
    'platform',
    'people',
    'Workspace creator',
    'Creates workspaces. Becomes owner of each workspace that it creates.',
    true
);

INSERT INTO role_permissions (role_id, permission)
VALUES ('00000000-0000-0000-0000-000000000001', 'workspaces.create');
