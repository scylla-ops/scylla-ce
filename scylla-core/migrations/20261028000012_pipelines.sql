CREATE TABLE pipelines (
    id uuid PRIMARY KEY,
    project_id uuid NOT NULL REFERENCES projects (id) ON DELETE CASCADE,
    workspace_id uuid NOT NULL REFERENCES workspaces (id),
    created_by uuid NOT NULL REFERENCES actors (id),
    name text NOT NULL,
    nodes jsonb NOT NULL,
    version bigint NOT NULL DEFAULT 1 CHECK (version >= 1),
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX pipelines_project_id_idx ON pipelines (project_id);
CREATE INDEX pipelines_workspace_id_idx ON pipelines (workspace_id);
CREATE INDEX pipelines_created_by_idx ON pipelines (created_by);
