CREATE TABLE projects (
    id uuid PRIMARY KEY,
    owner_id uuid NOT NULL REFERENCES users (id),
    name text NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX projects_owner_id_idx ON projects (owner_id);
