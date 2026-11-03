CREATE TABLE invitations (
    id uuid PRIMARY KEY,
    workspace_id uuid NOT NULL REFERENCES workspaces (id) ON DELETE CASCADE,
    email text NOT NULL,
    role text NOT NULL,
    token_hash bytea NOT NULL UNIQUE,
    invited_by uuid NOT NULL REFERENCES actors (id),
    expires_at timestamptz NOT NULL,
    accepted_by uuid REFERENCES actors (id),
    accepted_at timestamptz,
    created_at timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX invitations_workspace_id_idx ON invitations (workspace_id);
CREATE INDEX invitations_email_idx ON invitations (email);
