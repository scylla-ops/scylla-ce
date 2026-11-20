-- The saved layouts of the dashboard of a workspace. The server keeps the widgets and does not
-- read them. A private dashboard is visible to its creator only.
CREATE TABLE dashboards (
    id uuid PRIMARY KEY,
    workspace_id uuid NOT NULL REFERENCES workspaces (id) ON DELETE CASCADE,
    created_by uuid NOT NULL REFERENCES actors (id),
    name text NOT NULL,
    visibility text NOT NULL CHECK (visibility IN ('private', 'workspace')),
    team_default boolean NOT NULL DEFAULT false,
    schema_version integer NOT NULL CHECK (schema_version BETWEEN 1 AND 65535),
    widgets jsonb NOT NULL,
    version bigint NOT NULL DEFAULT 1 CHECK (version >= 1),
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now(),
    CHECK (NOT team_default OR visibility = 'workspace')
);

CREATE INDEX dashboards_workspace_id_idx ON dashboards (workspace_id);
CREATE INDEX dashboards_created_by_idx ON dashboards (created_by);
CREATE UNIQUE INDEX dashboards_team_default_idx ON dashboards (workspace_id) WHERE team_default;
