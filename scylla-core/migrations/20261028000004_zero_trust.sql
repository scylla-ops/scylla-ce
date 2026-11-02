DROP TABLE api_keys;

CREATE TABLE users (
    id uuid PRIMARY KEY REFERENCES actors (id),
    issuer text NOT NULL,
    subject text NOT NULL,
    UNIQUE (issuer, subject)
);

CREATE TABLE agents (
    id uuid PRIMARY KEY REFERENCES actors (id),
    registered_by uuid NOT NULL REFERENCES actors (id)
);

CREATE INDEX agents_registered_by_idx ON agents (registered_by);

CREATE TABLE agent_keys (
    id uuid PRIMARY KEY,
    agent_id uuid NOT NULL REFERENCES agents (id),
    public_key bytea NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now(),
    revoked_at timestamptz
);

CREATE INDEX agent_keys_agent_id_idx ON agent_keys (agent_id);

CREATE TABLE assertion_nonces (
    nonce text PRIMARY KEY,
    expires_at timestamptz NOT NULL
);

CREATE TABLE revoked_tokens (
    revocation_id bytea PRIMARY KEY,
    revoked_at timestamptz NOT NULL DEFAULT now()
);
