CREATE TABLE agents (
    id uuid PRIMARY KEY REFERENCES actors (id),
    workspace_id uuid NOT NULL REFERENCES workspaces (id)
);

CREATE INDEX agents_workspace_id_idx ON agents (workspace_id);

CREATE TABLE agent_keys (
    id uuid PRIMARY KEY,
    agent_id uuid NOT NULL REFERENCES agents (id),
    public_key bytea NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now(),
    revoked_at timestamptz
);

CREATE INDEX agent_keys_agent_id_idx ON agent_keys (agent_id);
