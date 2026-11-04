CREATE TABLE api_tokens (
    id uuid PRIMARY KEY,
    actor_id uuid NOT NULL REFERENCES actors (id) ON DELETE CASCADE,
    name text NOT NULL,
    secret_hash bytea NOT NULL UNIQUE,
    hint text NOT NULL,
    created_by uuid NOT NULL REFERENCES actors (id),
    created_at timestamptz NOT NULL DEFAULT now(),
    expires_at timestamptz NOT NULL,
    last_used_at timestamptz,
    revoked_at timestamptz,
    CHECK (expires_at > created_at)
);

CREATE INDEX api_tokens_actor_id_idx ON api_tokens (actor_id, created_at);
CREATE INDEX api_tokens_created_by_idx ON api_tokens (created_by);
