CREATE TABLE projects (
    id uuid PRIMARY KEY,
    workspace_id uuid NOT NULL REFERENCES workspaces (id),
    created_by uuid NOT NULL REFERENCES actors (id),
    name text NOT NULL,
    description text NOT NULL DEFAULT '',
    created_at timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX projects_workspace_id_idx ON projects (workspace_id);
CREATE INDEX projects_created_by_idx ON projects (created_by);
