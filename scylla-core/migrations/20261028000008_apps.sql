CREATE TABLE apps (
    id uuid PRIMARY KEY REFERENCES actors (id),
    workspace_id uuid NOT NULL REFERENCES workspaces (id),
    name text NOT NULL,
    runner boolean NOT NULL,
    created_by uuid NOT NULL REFERENCES actors (id),
    created_at timestamptz NOT NULL DEFAULT now(),
    last_seen_at timestamptz,
    version text,
    os text,
    arch text,
    hostname text,
    cpu_count integer CHECK (cpu_count >= 0),
    memory_bytes bigint CHECK (memory_bytes >= 0),
    capacity integer NOT NULL DEFAULT 1 CHECK (capacity >= 1),
    container_runtime text,
    disabled_at timestamptz
);

CREATE INDEX apps_workspace_id_idx ON apps (workspace_id);
CREATE INDEX apps_created_by_idx ON apps (created_by);

CREATE TABLE app_keys (
    id uuid PRIMARY KEY,
    app_id uuid NOT NULL REFERENCES apps (id) ON DELETE CASCADE,
    public_key bytea NOT NULL CHECK (length(public_key) = 32),
    created_at timestamptz NOT NULL DEFAULT now(),
    revoked_at timestamptz,
    label text,
    last_used_at timestamptz
);

CREATE INDEX app_keys_app_id_idx ON app_keys (app_id);

CREATE TABLE app_join_codes (
    hash bytea PRIMARY KEY,
    app_id uuid NOT NULL REFERENCES apps (id) ON DELETE CASCADE,
    created_by uuid NOT NULL REFERENCES actors (id),
    expires_at timestamptz NOT NULL,
    used_at timestamptz,
    created_at timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX app_join_codes_app_id_idx ON app_join_codes (app_id);
CREATE INDEX app_join_codes_created_by_idx ON app_join_codes (created_by);
