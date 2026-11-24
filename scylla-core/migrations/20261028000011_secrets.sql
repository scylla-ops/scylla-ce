CREATE TABLE secrets (
    id uuid PRIMARY KEY,
    project_id uuid NOT NULL REFERENCES projects (id) ON DELETE CASCADE,
    name text COLLATE "C" NOT NULL CHECK (name ~ '^[A-Za-z0-9._-]{1,128}$'),
    sealed_value bytea NOT NULL,
    created_by uuid NOT NULL REFERENCES actors (id),
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now(),
    UNIQUE (project_id, name)
);

CREATE INDEX secrets_created_by_idx ON secrets (created_by);
