CREATE TABLE trigger_fires (
    id uuid PRIMARY KEY,
    trigger_id uuid NOT NULL REFERENCES triggers (id) ON DELETE CASCADE,
    pipeline_id uuid NOT NULL REFERENCES pipelines (id) ON DELETE CASCADE,
    project_id uuid NOT NULL REFERENCES projects (id) ON DELETE CASCADE,
    workspace_id uuid NOT NULL REFERENCES workspaces (id),
    source text NOT NULL CHECK (source IN ('schedule', 'webhook', 'manual')),
    -- The cron time of a schedule, else the time of the request.
    due_at timestamptz NOT NULL,
    fired_at timestamptz,
    status text NOT NULL CHECK (status IN ('ok', 'error', 'missed')),
    error text,
    job_id uuid REFERENCES jobs (id) ON DELETE SET NULL,
    CHECK ((status = 'missed') = (fired_at IS NULL)),
    CHECK (status <> 'missed' OR source = 'schedule'),
    CHECK ((status = 'error') = (error IS NOT NULL))
);

CREATE INDEX trigger_fires_workspace_idx ON trigger_fires (workspace_id, due_at);
CREATE INDEX trigger_fires_trigger_idx ON trigger_fires (trigger_id, due_at);
CREATE INDEX trigger_fires_project_id_idx ON trigger_fires (project_id);
CREATE INDEX trigger_fires_pipeline_id_idx ON trigger_fires (pipeline_id);
CREATE INDEX trigger_fires_job_id_idx ON trigger_fires (job_id);
CREATE INDEX trigger_fires_due_at_idx ON trigger_fires (due_at);
