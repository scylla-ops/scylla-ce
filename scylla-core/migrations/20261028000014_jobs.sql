CREATE TABLE jobs (
    id uuid PRIMARY KEY,
    pipeline_id uuid NOT NULL REFERENCES pipelines (id) ON DELETE CASCADE,
    project_id uuid NOT NULL REFERENCES projects (id) ON DELETE CASCADE,
    workspace_id uuid NOT NULL REFERENCES workspaces (id),
    created_by uuid NOT NULL REFERENCES actors (id),
    rerun_of uuid REFERENCES jobs (id) ON DELETE SET NULL,
    trigger_id uuid REFERENCES triggers (id) ON DELETE SET NULL,
    status text NOT NULL
        CHECK (status IN ('pending', 'running', 'completed', 'failed', 'cancelled', 'orphaned')),
    nodes jsonb NOT NULL,
    runs jsonb NOT NULL,
    need_container boolean NOT NULL,
    agent_id uuid REFERENCES apps (id) ON DELETE SET NULL,
    error text,
    version bigint NOT NULL DEFAULT 1 CHECK (version >= 1),
    created_at timestamptz NOT NULL DEFAULT now(),
    started_at timestamptz,
    finished_at timestamptz,
    CHECK ((status IN ('pending', 'running')) = (finished_at IS NULL))
);

CREATE INDEX jobs_pipeline_id_idx ON jobs (pipeline_id);
CREATE INDEX jobs_project_id_idx ON jobs (project_id);
CREATE INDEX jobs_workspace_id_idx ON jobs (workspace_id, created_at);
CREATE INDEX jobs_created_by_idx ON jobs (created_by);
CREATE INDEX jobs_rerun_of_idx ON jobs (rerun_of);
CREATE INDEX jobs_trigger_id_idx ON jobs (trigger_id);
CREATE INDEX jobs_agent_id_idx ON jobs (agent_id) WHERE status IN ('pending', 'running');
CREATE INDEX jobs_pool_idx ON jobs (workspace_id, id) WHERE status = 'pending' AND agent_id IS NULL;

CREATE TABLE job_logs (
    id uuid PRIMARY KEY,
    job_id uuid NOT NULL REFERENCES jobs (id) ON DELETE CASCADE,
    node_id text NOT NULL,
    lines jsonb NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX job_logs_job_id_idx ON job_logs (job_id, id);
CREATE INDEX job_logs_created_at_idx ON job_logs (created_at);
