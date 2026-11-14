CREATE TABLE jobs (
    id uuid PRIMARY KEY,
    pipeline_id uuid NOT NULL REFERENCES pipelines (id) ON DELETE CASCADE,
    project_id uuid NOT NULL REFERENCES projects (id) ON DELETE CASCADE,
    workspace_id uuid NOT NULL REFERENCES workspaces (id),
    created_by uuid NOT NULL REFERENCES actors (id),
    rerun_of uuid REFERENCES jobs (id) ON DELETE SET NULL,
    trigger_id uuid REFERENCES triggers (id) ON DELETE SET NULL,
    status text NOT NULL
        CHECK (status IN ('awaiting_approval', 'pending', 'running', 'completed', 'failed',
            'cancelled', 'orphaned')),
    nodes jsonb NOT NULL,
    runs jsonb NOT NULL,
    need_container boolean NOT NULL,
    need_checkout boolean NOT NULL DEFAULT false,
    agent_id uuid REFERENCES apps (id) ON DELETE SET NULL,
    error text,
    -- The approval of a job from a fork: it waits, then an editor approves or rejects it.
    approval text CHECK (approval IN ('waiting', 'approved', 'rejected')),
    approved_by uuid REFERENCES actors (id),
    approved_at timestamptz,
    rejected_by uuid REFERENCES actors (id),
    rejected_at timestamptz,
    checkout jsonb,
    pipeline_file jsonb,
    file_errors jsonb,
    version bigint NOT NULL DEFAULT 1 CHECK (version >= 1),
    created_at timestamptz NOT NULL DEFAULT now(),
    started_at timestamptz,
    finished_at timestamptz,
    CHECK ((status IN ('awaiting_approval', 'pending', 'running')) = (finished_at IS NULL)),
    CHECK (status <> 'awaiting_approval' OR approval IS NOT DISTINCT FROM 'waiting'),
    CHECK ((approval IS NOT DISTINCT FROM 'approved') = (approved_by IS NOT NULL)),
    CHECK ((approved_by IS NULL) = (approved_at IS NULL)),
    CHECK ((approval IS NOT DISTINCT FROM 'rejected') = (rejected_by IS NOT NULL)),
    CHECK ((rejected_by IS NULL) = (rejected_at IS NULL))
);

CREATE INDEX jobs_pipeline_id_idx ON jobs (pipeline_id);
CREATE INDEX jobs_project_id_idx ON jobs (project_id);
CREATE INDEX jobs_workspace_id_idx ON jobs (workspace_id, created_at);
CREATE INDEX jobs_created_by_idx ON jobs (created_by);
CREATE INDEX jobs_rerun_of_idx ON jobs (rerun_of);
CREATE INDEX jobs_trigger_id_idx ON jobs (trigger_id);
CREATE INDEX jobs_approved_by_idx ON jobs (approved_by);
CREATE INDEX jobs_rejected_by_idx ON jobs (rejected_by);
CREATE INDEX jobs_agent_id_idx ON jobs (agent_id) WHERE status IN ('pending', 'running');
CREATE INDEX jobs_pool_idx ON jobs (workspace_id, id) WHERE status = 'pending' AND agent_id IS NULL;

-- The commit of a job in the linked repository, and the event that started the job. An automatic
-- event makes one job for each pipeline and commit.
CREATE TABLE job_sources (
    job_id uuid PRIMARY KEY REFERENCES jobs (id) ON DELETE CASCADE,
    pipeline_id uuid NOT NULL REFERENCES pipelines (id) ON DELETE CASCADE,
    provider text NOT NULL CHECK (provider IN ('github', 'gitlab')),
    repository text NOT NULL,
    url text NOT NULL,
    sha text NOT NULL,
    ref text,
    branch text,
    tag text,
    pull_requests jsonb NOT NULL DEFAULT '[]',
    from_fork boolean NOT NULL DEFAULT false,
    fork_repository text,
    author_login text,
    avatar text,
    message_title text NOT NULL DEFAULT '',
    event text NOT NULL
        CHECK (event IN ('push', 'pull_request', 'tag', 'manual', 'provider_rerun', 'trigger')),
    automatic boolean NOT NULL,
    changes_pipeline_files boolean NOT NULL DEFAULT false,
    CHECK (from_fork = (fork_repository IS NOT NULL)),
    CHECK (branch IS NULL OR tag IS NULL)
);

CREATE UNIQUE INDEX job_sources_commit_idx ON job_sources (pipeline_id, sha) WHERE automatic;
CREATE INDEX job_sources_branch_idx ON job_sources (pipeline_id, branch);
CREATE INDEX job_sources_event_idx ON job_sources (pipeline_id, event);
CREATE INDEX job_sources_author_idx ON job_sources (pipeline_id, author_login);
CREATE INDEX job_sources_pull_requests_idx ON job_sources USING gin (pull_requests jsonb_path_ops);

CREATE TABLE job_logs (
    id uuid PRIMARY KEY,
    job_id uuid NOT NULL REFERENCES jobs (id) ON DELETE CASCADE,
    node_id text NOT NULL,
    lines jsonb NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX job_logs_job_id_idx ON job_logs (job_id, id);
CREATE INDEX job_logs_created_at_idx ON job_logs (created_at);
