CREATE TABLE trigger_runners (
    workspace_id uuid PRIMARY KEY REFERENCES workspaces (id) ON DELETE CASCADE,
    actor_id uuid NOT NULL UNIQUE REFERENCES actors (id)
);

CREATE TABLE triggers (
    id uuid PRIMARY KEY,
    pipeline_id uuid NOT NULL REFERENCES pipelines (id) ON DELETE CASCADE,
    project_id uuid NOT NULL REFERENCES projects (id) ON DELETE CASCADE,
    workspace_id uuid NOT NULL REFERENCES workspaces (id),
    created_by uuid NOT NULL REFERENCES actors (id),
    name text NOT NULL,
    kind text NOT NULL CHECK (kind IN ('cron', 'webhook')),
    schedule text,
    sealed_secret bytea,
    inputs jsonb NOT NULL,
    enabled boolean NOT NULL DEFAULT true,
    next_fire_at timestamptz,
    last_fired_at timestamptz,
    last_status text CHECK (last_status IN ('ok', 'error')),
    last_error text,
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now(),
    UNIQUE (pipeline_id, name),
    CHECK ((kind = 'cron') = (schedule IS NOT NULL)),
    CHECK ((kind = 'webhook') = (sealed_secret IS NOT NULL))
);

CREATE INDEX triggers_project_id_idx ON triggers (project_id);
CREATE INDEX triggers_workspace_id_idx ON triggers (workspace_id);
CREATE INDEX triggers_created_by_idx ON triggers (created_by);
CREATE INDEX triggers_due_idx ON triggers (next_fire_at) WHERE enabled AND kind = 'cron';

CREATE TABLE trigger_deliveries (
    trigger_id uuid NOT NULL REFERENCES triggers (id) ON DELETE CASCADE,
    key text NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (trigger_id, key)
);

CREATE INDEX trigger_deliveries_created_at_idx ON trigger_deliveries (created_at);
