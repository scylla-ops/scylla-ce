CREATE TABLE pipelines (
    id uuid PRIMARY KEY,
    project_id uuid NOT NULL REFERENCES projects (id) ON DELETE CASCADE,
    workspace_id uuid NOT NULL REFERENCES workspaces (id),
    created_by uuid NOT NULL REFERENCES actors (id),
    name text NOT NULL,
    nodes jsonb NOT NULL,
    version bigint NOT NULL DEFAULT 1 CHECK (version >= 1),
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now(),
    -- Where the pipeline comes from: the editor of Scylla, or a file in the linked repository.
    source text NOT NULL DEFAULT 'editor' CHECK (source IN ('editor', 'file')),
    file_key text,
    file_path text,
    file_sha text,
    file_rules jsonb,
    file_errors jsonb,
    checkout jsonb,
    archived_at timestamptz,
    removed_at timestamptz,
    UNIQUE (project_id, file_key),
    CHECK ((source = 'file') = (file_key IS NOT NULL)),
    CHECK ((source = 'file') = (file_path IS NOT NULL)),
    CHECK (source = 'file' OR (file_sha IS NULL AND file_rules IS NULL AND file_errors IS NULL
        AND removed_at IS NULL))
);

CREATE INDEX pipelines_project_id_idx ON pipelines (project_id);
CREATE INDEX pipelines_workspace_id_idx ON pipelines (workspace_id);
CREATE INDEX pipelines_created_by_idx ON pipelines (created_by);
