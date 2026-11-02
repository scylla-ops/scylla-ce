CREATE TABLE projects (
    id uuid PRIMARY KEY,
    owner_id uuid NOT NULL REFERENCES actors (id),
    created_by uuid NOT NULL REFERENCES actors (id),
    name text NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX projects_owner_id_idx ON projects (owner_id);
CREATE INDEX projects_created_by_idx ON projects (created_by);
