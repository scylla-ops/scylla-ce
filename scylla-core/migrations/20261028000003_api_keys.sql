CREATE TABLE api_keys (
    id uuid PRIMARY KEY,
    actor_id uuid NOT NULL REFERENCES actors (id),
    hash bytea NOT NULL UNIQUE,
    created_at timestamptz NOT NULL DEFAULT now(),
    revoked_at timestamptz
);

CREATE INDEX api_keys_actor_id_idx ON api_keys (actor_id);
