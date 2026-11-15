-- The GitHub App of a self-hosted instance, made with the manifest flow. The cloud App comes from
-- the settings and has no row. Each secret is sealed with the id of the row.
CREATE TABLE vcs_apps (
    id uuid PRIMARY KEY,
    provider text NOT NULL UNIQUE CHECK (provider IN ('github')),
    app_id bigint NOT NULL,
    slug text NOT NULL,
    name text NOT NULL,
    owner text,
    web_url text NOT NULL,
    client_id text NOT NULL,
    sealed_client_secret bytea NOT NULL,
    sealed_private_key bytea NOT NULL,
    sealed_webhook_secret bytea NOT NULL,
    created_by uuid NOT NULL REFERENCES actors (id),
    created_at timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX vcs_apps_created_by_idx ON vcs_apps (created_by);

-- The state of a flow on the provider: the install of the App in a workspace, or the manifest of
-- the App of the instance. The table keeps the hash of the state only.
CREATE TABLE vcs_setup_states (
    hash bytea PRIMARY KEY,
    kind text NOT NULL CHECK (kind IN ('github_install', 'github_manifest')),
    workspace_id uuid REFERENCES workspaces (id) ON DELETE CASCADE,
    created_by uuid NOT NULL REFERENCES actors (id),
    expires_at timestamptz NOT NULL,
    used_at timestamptz,
    created_at timestamptz NOT NULL DEFAULT now(),
    CHECK ((kind = 'github_install') = (workspace_id IS NOT NULL))
);

CREATE INDEX vcs_setup_states_workspace_id_idx ON vcs_setup_states (workspace_id);
CREATE INDEX vcs_setup_states_created_by_idx ON vcs_setup_states (created_by);
CREATE INDEX vcs_setup_states_expires_at_idx ON vcs_setup_states (expires_at);

-- The connections of a workspace: an installation of the GitHub App, or the token of a GitLab
-- service account, sealed with the id of the row.
CREATE TABLE vcs_connections (
    id uuid PRIMARY KEY,
    workspace_id uuid NOT NULL REFERENCES workspaces (id) ON DELETE CASCADE,
    provider text NOT NULL CHECK (provider IN ('github', 'gitlab')),
    auth_kind text NOT NULL CHECK (auth_kind IN ('app_installation', 'access_token')),
    app_id uuid REFERENCES vcs_apps (id) ON DELETE CASCADE,
    external_id text NOT NULL,
    account text NOT NULL,
    account_kind text NOT NULL CHECK (account_kind IN ('organization', 'user', 'service_account')),
    avatar_url text,
    sealed_token bytea,
    token_expires_at timestamptz,
    token_warned_at timestamptz,
    scopes text[] NOT NULL DEFAULT '{}',
    permissions jsonb NOT NULL DEFAULT '{}',
    all_repositories boolean NOT NULL DEFAULT false,
    repository_count integer NOT NULL DEFAULT 0 CHECK (repository_count >= 0),
    delivery_mode text NOT NULL DEFAULT 'webhook' CHECK (delivery_mode IN ('webhook', 'polling')),
    poll_interval_seconds integer NOT NULL DEFAULT 120
        CHECK (poll_interval_seconds BETWEEN 60 AND 900),
    last_event_at timestamptz,
    state text NOT NULL DEFAULT 'active'
        CHECK (state IN ('active', 'suspended', 'needs_attention', 'disconnected')),
    state_reason text,
    created_by uuid NOT NULL REFERENCES actors (id),
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now(),
    UNIQUE (workspace_id, provider, external_id),
    CHECK ((auth_kind = 'access_token') = (sealed_token IS NOT NULL)),
    CHECK (auth_kind = 'app_installation' OR app_id IS NULL)
);

CREATE INDEX vcs_connections_external_idx ON vcs_connections (provider, external_id);
CREATE INDEX vcs_connections_app_id_idx ON vcs_connections (app_id);
CREATE INDEX vcs_connections_created_by_idx ON vcs_connections (created_by);
CREATE INDEX vcs_connections_token_expires_at_idx ON vcs_connections (token_expires_at)
    WHERE token_expires_at IS NOT NULL;

-- The repository of a project: at most one for each project.
CREATE TABLE repository_links (
    project_id uuid PRIMARY KEY REFERENCES projects (id) ON DELETE CASCADE,
    workspace_id uuid NOT NULL REFERENCES workspaces (id),
    connection_id uuid NOT NULL REFERENCES vcs_connections (id) ON DELETE CASCADE,
    external_id text NOT NULL,
    full_name text NOT NULL,
    web_url text NOT NULL,
    default_branch text NOT NULL,
    private boolean NOT NULL,
    role text CHECK (role IN ('developer', 'maintainer', 'owner')),
    pipeline_source text NOT NULL DEFAULT 'editor' CHECK (pipeline_source IN ('editor', 'file')),
    reports_enabled boolean NOT NULL DEFAULT true,
    delivery_mode text NOT NULL DEFAULT 'webhook' CHECK (delivery_mode IN ('webhook', 'polling')),
    poll_interval_seconds integer NOT NULL DEFAULT 120
        CHECK (poll_interval_seconds BETWEEN 60 AND 900),
    next_poll_at timestamptz,
    hook_id text,
    sealed_hook_secret bytea,
    ruleset_id bigint,
    last_event_at timestamptz,
    state text NOT NULL DEFAULT 'active'
        CHECK (state IN ('active', 'suspended', 'needs_attention', 'disconnected')),
    state_reason text,
    created_by uuid NOT NULL REFERENCES actors (id),
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now(),
    CHECK ((hook_id IS NULL) = (sealed_hook_secret IS NULL))
);

CREATE INDEX repository_links_workspace_id_idx ON repository_links (workspace_id);
CREATE INDEX repository_links_connection_idx ON repository_links (connection_id, external_id);
CREATE INDEX repository_links_created_by_idx ON repository_links (created_by);
CREATE INDEX repository_links_poll_idx ON repository_links (next_poll_at)
    WHERE next_poll_at IS NOT NULL;

-- The rules that block the merge when a pipeline fails: one for each pipeline on GitHub, one for
-- the project (no pipeline) on GitLab.
CREATE TABLE merge_blocks (
    project_id uuid NOT NULL REFERENCES repository_links (project_id) ON DELETE CASCADE,
    pipeline_id uuid REFERENCES pipelines (id) ON DELETE CASCADE,
    context text,
    enabled boolean NOT NULL,
    state text NOT NULL CHECK (state IN ('off', 'applying', 'on', 'needs_permission',
        'not_possible', 'out_of_sync', 'failed')),
    state_reason text,
    updated_by uuid NOT NULL REFERENCES actors (id),
    updated_at timestamptz NOT NULL DEFAULT now(),
    CHECK ((pipeline_id IS NULL) = (context IS NULL))
);

CREATE UNIQUE INDEX merge_blocks_target_idx ON merge_blocks (project_id, pipeline_id)
    NULLS NOT DISTINCT;
CREATE INDEX merge_blocks_pipeline_id_idx ON merge_blocks (pipeline_id);
CREATE INDEX merge_blocks_updated_by_idx ON merge_blocks (updated_by);

-- The deliveries of the webhooks and the keys of the polls, so a second delivery does nothing.
CREATE TABLE vcs_deliveries (
    provider text NOT NULL CHECK (provider IN ('github', 'gitlab')),
    delivery_id text NOT NULL,
    app_id uuid REFERENCES vcs_apps (id) ON DELETE CASCADE,
    connection_id uuid REFERENCES vcs_connections (id) ON DELETE SET NULL,
    project_id uuid REFERENCES projects (id) ON DELETE SET NULL,
    event text NOT NULL,
    action text,
    status integer CHECK (status BETWEEN 100 AND 599),
    received_at timestamptz NOT NULL DEFAULT now(),
    processed_at timestamptz,
    error text,
    PRIMARY KEY (provider, delivery_id)
);

CREATE INDEX vcs_deliveries_received_at_idx ON vcs_deliveries (received_at);
CREATE INDEX vcs_deliveries_app_id_idx ON vcs_deliveries (app_id, received_at);
CREATE INDEX vcs_deliveries_connection_id_idx ON vcs_deliveries (connection_id);
CREATE INDEX vcs_deliveries_project_id_idx ON vcs_deliveries (project_id);

-- The last head that a poll saw on a ref of a linked repository, with the cursor of the provider.
CREATE TABLE vcs_poll_state (
    project_id uuid NOT NULL REFERENCES repository_links (project_id) ON DELETE CASCADE,
    ref text NOT NULL,
    last_sha text,
    etag text,
    updated_after timestamptz,
    polled_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (project_id, ref)
);

-- The outbox of the reports: the state that a job must report on its commit, and what the
-- provider got.
CREATE TABLE vcs_reports (
    job_id uuid NOT NULL REFERENCES jobs (id) ON DELETE CASCADE,
    context text NOT NULL,
    project_id uuid NOT NULL REFERENCES projects (id) ON DELETE CASCADE,
    connection_id uuid NOT NULL REFERENCES vcs_connections (id) ON DELETE CASCADE,
    mode text NOT NULL CHECK (mode IN ('check_run', 'commit_status')),
    wanted_state text NOT NULL CHECK (wanted_state IN ('awaiting_approval', 'pending', 'running',
        'completed', 'failed', 'cancelled', 'orphaned', 'invalid_file')),
    sequence bigint NOT NULL CHECK (sequence >= 1),
    state text NOT NULL DEFAULT 'sending'
        CHECK (state IN ('sending', 'sent', 'retrying', 'not_sent')),
    sent_state text,
    sent_sequence bigint,
    sent_at timestamptz,
    external_id text,
    external_url text,
    attempts integer NOT NULL DEFAULT 0 CHECK (attempts >= 0),
    next_attempt_at timestamptz,
    last_error text,
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (job_id, context)
);

CREATE INDEX vcs_reports_due_idx ON vcs_reports (next_attempt_at)
    WHERE next_attempt_at IS NOT NULL;
CREATE INDEX vcs_reports_project_id_idx ON vcs_reports (project_id);
CREATE INDEX vcs_reports_connection_id_idx ON vcs_reports (connection_id);

-- The GitLab deploy tokens of the checkouts. The row stays until the token is deleted on GitLab.
-- No row holds a token.
CREATE TABLE vcs_checkout_tokens (
    id uuid PRIMARY KEY,
    job_id uuid REFERENCES jobs (id) ON DELETE SET NULL,
    connection_id uuid NOT NULL REFERENCES vcs_connections (id) ON DELETE CASCADE,
    project_external_id text NOT NULL,
    token_external_id text NOT NULL,
    expires_at timestamptz NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now(),
    UNIQUE (connection_id, project_external_id, token_external_id)
);

CREATE INDEX vcs_checkout_tokens_job_id_idx ON vcs_checkout_tokens (job_id);
CREATE INDEX vcs_checkout_tokens_expires_at_idx ON vcs_checkout_tokens (expires_at);
