CREATE TABLE workspace_owners (
    workspace_id uuid NOT NULL REFERENCES workspaces (id) ON DELETE CASCADE,
    user_id uuid NOT NULL REFERENCES users (id),
    created_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (workspace_id, user_id)
);

CREATE INDEX workspace_owners_user_id_idx ON workspace_owners (user_id);

INSERT INTO workspace_owners (workspace_id, user_id, created_at)
SELECT id, owner_id, created_at FROM workspaces;

ALTER TABLE workspaces DROP COLUMN owner_id;
