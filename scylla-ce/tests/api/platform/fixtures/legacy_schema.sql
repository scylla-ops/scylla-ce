-- The schema of the old product (scylla-ops/scylla-ce, migrations up to 20261025120000), with its
-- built-in roles. The import test loads it into a database of its own.
CREATE FUNCTION public.bump_authz_version() RETURNS trigger
    LANGUAGE plpgsql
    AS $$
BEGIN
    UPDATE authz_version SET version = version + 1;
    RETURN NULL;
END;
$$;
CREATE FUNCTION public.check_grant_references() RETURNS trigger
    LANGUAGE plpgsql
    AS $$
DECLARE
    organization TEXT;
BEGIN
    IF NEW.principal_kind = 'user' THEN
        PERFORM 1 FROM users WHERE id = NEW.principal_id FOR KEY SHARE;
    ELSE
        PERFORM 1 FROM apps WHERE id = NEW.principal_id FOR KEY SHARE;
    END IF;
    IF NOT FOUND THEN
        RAISE EXCEPTION 'grant principal %:% does not exist', NEW.principal_kind, NEW.principal_id
            USING ERRCODE = 'foreign_key_violation';
    END IF;
    IF NEW.scope_kind = 'organization' THEN
        SELECT id INTO organization FROM organizations WHERE id = NEW.scope_id FOR KEY SHARE;
    ELSIF NEW.scope_kind = 'project' THEN
        SELECT organization_id INTO organization FROM projects WHERE id = NEW.scope_id FOR KEY SHARE;
    END IF;
    IF NEW.scope_kind <> 'system' AND NOT FOUND THEN
        RAISE EXCEPTION 'grant scope %:% does not exist', NEW.scope_kind, NEW.scope_id
            USING ERRCODE = 'foreign_key_violation';
    END IF;
    PERFORM check_role_binding(NEW.role_id, NEW.principal_kind, NEW.scope_kind, organization);
    RETURN NEW;
END;
$$;
CREATE FUNCTION public.check_role_binding(role text, principal text, scope text, organization text) RETURNS void
    LANGUAGE plpgsql
    AS $$
DECLARE
    r RECORD;
BEGIN
    SELECT scope_kind, owner_org_id, kind INTO r FROM roles WHERE id = role FOR KEY SHARE;
    IF NOT FOUND
       OR r.scope_kind <> scope
       OR (r.kind = 'agent' AND principal = 'user')
       OR (r.owner_org_id IS NOT NULL AND r.owner_org_id IS DISTINCT FROM organization) THEN
        RAISE EXCEPTION 'role % cannot be given here', role
            USING ERRCODE = 'foreign_key_violation';
    END IF;
END;
$$;
CREATE FUNCTION public.check_role_change() RETURNS trigger
    LANGUAGE plpgsql
    AS $$
BEGIN
    IF EXISTS (
        SELECT 1 FROM grants g
        LEFT JOIN projects p ON g.scope_kind = 'project' AND p.id = g.scope_id
        WHERE g.role_id = NEW.id
          AND (g.scope_kind <> NEW.scope_kind
               OR (NEW.kind = 'agent' AND g.principal_kind = 'user')
               OR (NEW.owner_org_id IS NOT NULL
                   AND NEW.owner_org_id IS DISTINCT FROM CASE g.scope_kind
                       WHEN 'organization' THEN g.scope_id
                       WHEN 'project' THEN p.organization_id
                   END))
    ) THEN
        RAISE EXCEPTION 'role % is given where its new kind, scope or owner does not allow it', NEW.id
            USING ERRCODE = 'foreign_key_violation';
    END IF;
    RETURN NEW;
END;
$$;
CREATE FUNCTION public.check_role_name() RETURNS trigger
    LANGUAGE plpgsql
    AS $$
BEGIN
    IF NEW.owner_org_id IS NOT NULL AND EXISTS (
        SELECT 1 FROM roles r
        WHERE r.owner_org_id IS NULL
          AND lower(r.name) = lower(NEW.name)
    ) THEN
        RAISE EXCEPTION 'role name % is taken', NEW.name
            USING ERRCODE = 'unique_violation', CONSTRAINT = 'roles_name_key';
    END IF;
    RETURN NEW;
END;
$$;
CREATE FUNCTION public.delete_grants_for_principal() RETURNS trigger
    LANGUAGE plpgsql
    AS $$
BEGIN
    DELETE FROM grants WHERE principal_kind = TG_ARGV[0] AND principal_id = OLD.id;
    RETURN OLD;
END;
$$;
CREATE FUNCTION public.delete_grants_for_scope() RETURNS trigger
    LANGUAGE plpgsql
    AS $$
BEGIN
    DELETE FROM grants WHERE scope_kind = TG_ARGV[0] AND scope_id = OLD.id;
    RETURN OLD;
END;
$$;
CREATE TABLE public.agents (
    app_id text NOT NULL,
    last_seen timestamp with time zone,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    agent_version text,
    host_os text,
    host_arch text,
    hostname text,
    cpu_count integer,
    total_memory_mb bigint,
    host_reported_at timestamp with time zone
);
CREATE TABLE public.app_secrets (
    id text NOT NULL,
    app_id text NOT NULL,
    label text NOT NULL,
    secret_hash text NOT NULL,
    enabled boolean DEFAULT true NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL
);
CREATE TABLE public.app_tokens (
    id text NOT NULL,
    token_hash text CONSTRAINT app_tokens_token_not_null NOT NULL,
    app_id text NOT NULL,
    secret_id text NOT NULL,
    created_at timestamp with time zone NOT NULL,
    expires_at timestamp with time zone NOT NULL
);
CREATE TABLE public.apps (
    id text NOT NULL,
    organization_id text NOT NULL,
    name text NOT NULL,
    is_active boolean DEFAULT true NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL,
    kind text DEFAULT 'standard'::text NOT NULL,
    CONSTRAINT apps_kind_check CHECK ((kind = ANY (ARRAY['standard'::text, 'trigger_runner'::text])))
);
CREATE TABLE public.audit_log (
    id text NOT NULL,
    occurred_at timestamp with time zone NOT NULL,
    principal_kind text NOT NULL,
    principal_id text,
    action text NOT NULL,
    resource_kind text NOT NULL,
    resource_id text,
    decision text NOT NULL,
    policies text[] DEFAULT '{}'::text[] NOT NULL,
    reason text,
    created_at timestamp with time zone DEFAULT now() NOT NULL
);
CREATE TABLE public.authz_version (
    id boolean DEFAULT true NOT NULL,
    version bigint NOT NULL,
    CONSTRAINT authz_version_id_check CHECK (id)
);
CREATE TABLE public.grants (
    id text NOT NULL,
    principal_kind text NOT NULL,
    principal_id text NOT NULL,
    role_id text NOT NULL,
    scope_kind text NOT NULL,
    scope_id text NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    CONSTRAINT grants_principal_kind_check CHECK ((principal_kind = ANY (ARRAY['user'::text, 'app'::text]))),
    CONSTRAINT grants_scope_kind_check CHECK ((scope_kind = ANY (ARRAY['system'::text, 'organization'::text, 'project'::text])))
);
CREATE TABLE public.job_logs (
    id text NOT NULL,
    job_id text NOT NULL,
    node_id text NOT NULL,
    stream text NOT NULL,
    line text NOT NULL,
    "timestamp" timestamp with time zone NOT NULL,
    created_at timestamp with time zone NOT NULL
);
CREATE TABLE public.jobs (
    id text NOT NULL,
    pipeline_id text NOT NULL,
    status text NOT NULL,
    node_executions jsonb NOT NULL,
    inputs jsonb DEFAULT '[]'::jsonb NOT NULL,
    origin jsonb NOT NULL,
    agent_app_id text,
    created_at timestamp with time zone NOT NULL,
    updated_at timestamp with time zone NOT NULL,
    started_at timestamp with time zone,
    finished_at timestamp with time zone,
    nodes jsonb NOT NULL,
    version bigint DEFAULT 0 NOT NULL,
    stream_id text
);
CREATE TABLE public.organizations (
    id text NOT NULL,
    name text NOT NULL,
    description text,
    is_active boolean DEFAULT true NOT NULL,
    created_at timestamp with time zone NOT NULL,
    updated_at timestamp with time zone NOT NULL,
    version bigint DEFAULT 0 NOT NULL
);
CREATE TABLE public.password_resets (
    id text NOT NULL,
    user_id text NOT NULL,
    token_hash text NOT NULL,
    created_at timestamp with time zone NOT NULL,
    expires_at timestamp with time zone NOT NULL,
    used_at timestamp with time zone
);
CREATE TABLE public.pipeline_triggers (
    id text NOT NULL,
    pipeline_id text NOT NULL,
    name text NOT NULL,
    kind text NOT NULL,
    source jsonb NOT NULL,
    inputs jsonb DEFAULT '[]'::jsonb NOT NULL,
    enabled boolean DEFAULT true NOT NULL,
    next_fire_at timestamp with time zone,
    last_fired_at timestamp with time zone,
    last_status text,
    webhook_secret_enc bytea,
    created_at timestamp with time zone NOT NULL,
    updated_at timestamp with time zone NOT NULL,
    version bigint DEFAULT 0 NOT NULL,
    CONSTRAINT pipeline_triggers_kind_check CHECK ((kind = ANY (ARRAY['cron'::text, 'webhook'::text])))
);
CREATE TABLE public.pipelines (
    id text NOT NULL,
    project_id text NOT NULL,
    name text NOT NULL,
    nodes jsonb NOT NULL,
    created_at timestamp with time zone NOT NULL,
    updated_at timestamp with time zone NOT NULL,
    version bigint DEFAULT 0 NOT NULL
);
CREATE TABLE public.project_secrets (
    id text NOT NULL,
    project_id text NOT NULL,
    name text NOT NULL,
    description text DEFAULT ''::text NOT NULL,
    encrypted_value bytea NOT NULL,
    created_at timestamp with time zone NOT NULL,
    updated_at timestamp with time zone NOT NULL
);
CREATE TABLE public.projects (
    id text NOT NULL,
    name text NOT NULL,
    description text,
    organization_id text NOT NULL,
    is_active boolean DEFAULT true NOT NULL,
    created_at timestamp with time zone NOT NULL,
    updated_at timestamp with time zone NOT NULL,
    version bigint DEFAULT 0 NOT NULL
);
CREATE TABLE public.role_permissions (
    role_id text NOT NULL,
    permission text NOT NULL
);
CREATE TABLE public.roles (
    id text NOT NULL,
    key text,
    name text NOT NULL,
    description text DEFAULT ''::text NOT NULL,
    scope_kind text NOT NULL,
    owner_org_id text,
    builtin boolean DEFAULT false NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL,
    kind text DEFAULT 'member'::text NOT NULL,
    version bigint DEFAULT 0 NOT NULL,
    CONSTRAINT roles_admin_builtin CHECK (((kind <> 'admin'::text) OR builtin)),
    CONSTRAINT roles_kind_check CHECK ((kind = ANY (ARRAY['admin'::text, 'member'::text, 'agent'::text]))),
    CONSTRAINT roles_owned_custom CHECK (((owner_org_id IS NULL) OR (NOT builtin))),
    CONSTRAINT roles_owned_scope CHECK (((owner_org_id IS NULL) OR (scope_kind = ANY (ARRAY['organization'::text, 'project'::text])))),
    CONSTRAINT roles_scope_kind_check CHECK ((scope_kind = ANY (ARRAY['system'::text, 'organization'::text, 'project'::text])))
);
CREATE TABLE public.sessions (
    id text NOT NULL,
    token_hash text CONSTRAINT sessions_token_not_null NOT NULL,
    user_id text NOT NULL,
    created_at timestamp with time zone NOT NULL,
    expires_at timestamp with time zone NOT NULL,
    last_active_at timestamp with time zone NOT NULL,
    user_agent text,
    ip_address text
);
CREATE TABLE public.trigger_deliveries (
    trigger_id text NOT NULL,
    delivery_id text NOT NULL,
    received_at timestamp with time zone NOT NULL
);
CREATE TABLE public.users (
    id text NOT NULL,
    username text NOT NULL,
    password_hash text NOT NULL,
    is_active boolean DEFAULT true NOT NULL,
    email text,
    created_at timestamp with time zone NOT NULL,
    updated_at timestamp with time zone NOT NULL,
    version bigint DEFAULT 0 NOT NULL,
    display_name text
);
ALTER TABLE ONLY public.agents
    ADD CONSTRAINT agents_pkey PRIMARY KEY (app_id);
ALTER TABLE ONLY public.app_secrets
    ADD CONSTRAINT app_secrets_app_id_label_key UNIQUE (app_id, label);
ALTER TABLE ONLY public.app_secrets
    ADD CONSTRAINT app_secrets_pkey PRIMARY KEY (id);
ALTER TABLE ONLY public.app_tokens
    ADD CONSTRAINT app_tokens_pkey PRIMARY KEY (id);
ALTER TABLE ONLY public.apps
    ADD CONSTRAINT apps_organization_id_name_key UNIQUE (organization_id, name);
ALTER TABLE ONLY public.apps
    ADD CONSTRAINT apps_pkey PRIMARY KEY (id);
ALTER TABLE ONLY public.audit_log
    ADD CONSTRAINT audit_log_pkey PRIMARY KEY (id);
ALTER TABLE ONLY public.authz_version
    ADD CONSTRAINT authz_version_pkey PRIMARY KEY (id);
ALTER TABLE ONLY public.grants
    ADD CONSTRAINT grants_pkey PRIMARY KEY (id);
ALTER TABLE ONLY public.grants
    ADD CONSTRAINT grants_principal_kind_principal_id_role_id_scope_kind_scope_key UNIQUE (principal_kind, principal_id, role_id, scope_kind, scope_id);
ALTER TABLE ONLY public.job_logs
    ADD CONSTRAINT job_logs_pkey PRIMARY KEY (id);
ALTER TABLE ONLY public.jobs
    ADD CONSTRAINT jobs_pkey PRIMARY KEY (id);
ALTER TABLE ONLY public.organizations
    ADD CONSTRAINT organizations_pkey PRIMARY KEY (id);
ALTER TABLE ONLY public.password_resets
    ADD CONSTRAINT password_resets_pkey PRIMARY KEY (id);
ALTER TABLE ONLY public.pipeline_triggers
    ADD CONSTRAINT pipeline_triggers_pipeline_id_name_key UNIQUE (pipeline_id, name);
ALTER TABLE ONLY public.pipeline_triggers
    ADD CONSTRAINT pipeline_triggers_pkey PRIMARY KEY (id);
ALTER TABLE ONLY public.pipelines
    ADD CONSTRAINT pipelines_pkey PRIMARY KEY (id);
ALTER TABLE ONLY public.project_secrets
    ADD CONSTRAINT project_secrets_pkey PRIMARY KEY (id);
ALTER TABLE ONLY public.project_secrets
    ADD CONSTRAINT project_secrets_project_id_name_key UNIQUE (project_id, name);
ALTER TABLE ONLY public.projects
    ADD CONSTRAINT projects_pkey PRIMARY KEY (id);
ALTER TABLE ONLY public.role_permissions
    ADD CONSTRAINT role_permissions_pkey PRIMARY KEY (role_id, permission);
ALTER TABLE ONLY public.roles
    ADD CONSTRAINT roles_key_key UNIQUE (key);
ALTER TABLE ONLY public.roles
    ADD CONSTRAINT roles_pkey PRIMARY KEY (id);
ALTER TABLE ONLY public.sessions
    ADD CONSTRAINT sessions_pkey PRIMARY KEY (id);
ALTER TABLE ONLY public.trigger_deliveries
    ADD CONSTRAINT trigger_deliveries_pkey PRIMARY KEY (trigger_id, delivery_id);
ALTER TABLE ONLY public.users
    ADD CONSTRAINT users_pkey PRIMARY KEY (id);
CREATE INDEX app_secrets_app_id_idx ON public.app_secrets USING btree (app_id);
CREATE INDEX app_tokens_app_id_idx ON public.app_tokens USING btree (app_id);
CREATE INDEX app_tokens_expires_at_idx ON public.app_tokens USING btree (expires_at);
CREATE INDEX app_tokens_secret_id_idx ON public.app_tokens USING btree (secret_id);
CREATE UNIQUE INDEX app_tokens_token_hash_key ON public.app_tokens USING btree (token_hash);
CREATE INDEX apps_organization_id_idx ON public.apps USING btree (organization_id);
CREATE UNIQUE INDEX apps_trigger_runner_idx ON public.apps USING btree (organization_id) WHERE (kind = 'trigger_runner'::text);
CREATE INDEX audit_log_action_idx ON public.audit_log USING btree (action);
CREATE INDEX audit_log_occurred_at_idx ON public.audit_log USING btree (occurred_at);
CREATE INDEX audit_log_principal_idx ON public.audit_log USING btree (principal_kind, principal_id);
CREATE INDEX audit_log_resource_idx ON public.audit_log USING btree (resource_kind, resource_id);
CREATE INDEX grants_principal_idx ON public.grants USING btree (principal_kind, principal_id);
CREATE INDEX grants_role_idx ON public.grants USING btree (role_id);
CREATE INDEX grants_scope_idx ON public.grants USING btree (scope_kind, scope_id);
CREATE INDEX job_logs_job_id_node_id_timestamp_idx ON public.job_logs USING btree (job_id, node_id, "timestamp");
CREATE INDEX job_logs_job_id_timestamp_idx ON public.job_logs USING btree (job_id, "timestamp");
CREATE INDEX jobs_agent_app_id_created_at_idx ON public.jobs USING btree (agent_app_id, created_at DESC);
CREATE INDEX jobs_created_at_idx ON public.jobs USING btree (created_at DESC);
CREATE INDEX jobs_pipeline_id_idx ON public.jobs USING btree (pipeline_id);
CREATE INDEX jobs_pool_idx ON public.jobs USING btree (created_at) WHERE ((status = 'pending'::text) AND (agent_app_id IS NULL));
CREATE INDEX jobs_status_idx ON public.jobs USING btree (status);
CREATE INDEX organizations_created_at_idx ON public.organizations USING btree (created_at DESC);
CREATE INDEX organizations_name_idx ON public.organizations USING btree (name);
CREATE UNIQUE INDEX password_resets_token_hash_key ON public.password_resets USING btree (token_hash);
CREATE INDEX password_resets_user_id_idx ON public.password_resets USING btree (user_id);
CREATE INDEX pipeline_triggers_due_idx ON public.pipeline_triggers USING btree (next_fire_at) WHERE (enabled AND (next_fire_at IS NOT NULL));
CREATE INDEX pipeline_triggers_pipeline_id_idx ON public.pipeline_triggers USING btree (pipeline_id);
CREATE INDEX pipelines_created_at_idx ON public.pipelines USING btree (created_at DESC);
CREATE INDEX pipelines_project_id_idx ON public.pipelines USING btree (project_id);
CREATE INDEX project_secrets_project_id_idx ON public.project_secrets USING btree (project_id);
CREATE INDEX projects_created_at_idx ON public.projects USING btree (created_at DESC);
CREATE INDEX projects_organization_id_idx ON public.projects USING btree (organization_id);
CREATE UNIQUE INDEX roles_name_key ON public.roles USING btree (owner_org_id, lower(name)) NULLS NOT DISTINCT;
CREATE INDEX sessions_expires_at_idx ON public.sessions USING btree (expires_at);
CREATE UNIQUE INDEX sessions_token_hash_key ON public.sessions USING btree (token_hash);
CREATE INDEX sessions_user_id_idx ON public.sessions USING btree (user_id);
CREATE INDEX users_created_at_idx ON public.users USING btree (created_at DESC);
CREATE UNIQUE INDEX users_email_key ON public.users USING btree (email) WHERE (email IS NOT NULL);
CREATE UNIQUE INDEX users_username_key ON public.users USING btree (username);
CREATE TRIGGER apps_delete_grants AFTER DELETE ON public.apps FOR EACH ROW EXECUTE FUNCTION public.delete_grants_for_principal('app');
CREATE TRIGGER grants_bump_authz_version AFTER INSERT OR DELETE OR UPDATE OR TRUNCATE ON public.grants FOR EACH STATEMENT EXECUTE FUNCTION public.bump_authz_version();
CREATE TRIGGER grants_check_references BEFORE INSERT OR UPDATE OF principal_kind, principal_id, scope_kind, scope_id, role_id ON public.grants FOR EACH ROW EXECUTE FUNCTION public.check_grant_references();
CREATE TRIGGER organizations_delete_grants AFTER DELETE ON public.organizations FOR EACH ROW EXECUTE FUNCTION public.delete_grants_for_scope('organization');
CREATE TRIGGER projects_delete_grants AFTER DELETE ON public.projects FOR EACH ROW EXECUTE FUNCTION public.delete_grants_for_scope('project');
CREATE TRIGGER role_permissions_bump_authz_version AFTER INSERT OR DELETE OR UPDATE OR TRUNCATE ON public.role_permissions FOR EACH STATEMENT EXECUTE FUNCTION public.bump_authz_version();
CREATE TRIGGER roles_bump_authz_version AFTER INSERT OR DELETE OR UPDATE OR TRUNCATE ON public.roles FOR EACH STATEMENT EXECUTE FUNCTION public.bump_authz_version();
CREATE TRIGGER roles_check_change BEFORE UPDATE OF kind, scope_kind, owner_org_id ON public.roles FOR EACH ROW EXECUTE FUNCTION public.check_role_change();
CREATE TRIGGER roles_check_name BEFORE INSERT OR UPDATE OF name, owner_org_id ON public.roles FOR EACH ROW EXECUTE FUNCTION public.check_role_name();
CREATE TRIGGER users_delete_grants AFTER DELETE ON public.users FOR EACH ROW EXECUTE FUNCTION public.delete_grants_for_principal('user');
ALTER TABLE ONLY public.agents
    ADD CONSTRAINT agents_app_id_fkey FOREIGN KEY (app_id) REFERENCES public.apps(id) ON DELETE CASCADE;
ALTER TABLE ONLY public.app_secrets
    ADD CONSTRAINT app_secrets_app_id_fkey FOREIGN KEY (app_id) REFERENCES public.apps(id) ON DELETE CASCADE;
ALTER TABLE ONLY public.app_tokens
    ADD CONSTRAINT app_tokens_app_id_fkey FOREIGN KEY (app_id) REFERENCES public.apps(id) ON DELETE CASCADE;
ALTER TABLE ONLY public.app_tokens
    ADD CONSTRAINT app_tokens_secret_id_fkey FOREIGN KEY (secret_id) REFERENCES public.app_secrets(id) ON DELETE CASCADE;
ALTER TABLE ONLY public.apps
    ADD CONSTRAINT apps_organization_id_fkey FOREIGN KEY (organization_id) REFERENCES public.organizations(id) ON DELETE CASCADE;
ALTER TABLE ONLY public.grants
    ADD CONSTRAINT grants_role_id_fkey FOREIGN KEY (role_id) REFERENCES public.roles(id) ON DELETE CASCADE;
ALTER TABLE ONLY public.job_logs
    ADD CONSTRAINT job_logs_job_id_fkey FOREIGN KEY (job_id) REFERENCES public.jobs(id) ON DELETE CASCADE;
ALTER TABLE ONLY public.jobs
    ADD CONSTRAINT jobs_agent_app_id_fkey FOREIGN KEY (agent_app_id) REFERENCES public.agents(app_id) ON DELETE SET NULL;
ALTER TABLE ONLY public.jobs
    ADD CONSTRAINT jobs_pipeline_id_fkey FOREIGN KEY (pipeline_id) REFERENCES public.pipelines(id) ON DELETE CASCADE;
ALTER TABLE ONLY public.password_resets
    ADD CONSTRAINT password_resets_user_id_fkey FOREIGN KEY (user_id) REFERENCES public.users(id) ON DELETE CASCADE;
ALTER TABLE ONLY public.pipeline_triggers
    ADD CONSTRAINT pipeline_triggers_pipeline_id_fkey FOREIGN KEY (pipeline_id) REFERENCES public.pipelines(id) ON DELETE CASCADE;
ALTER TABLE ONLY public.pipelines
    ADD CONSTRAINT pipelines_project_id_fkey FOREIGN KEY (project_id) REFERENCES public.projects(id) ON DELETE CASCADE;
ALTER TABLE ONLY public.project_secrets
    ADD CONSTRAINT project_secrets_project_id_fkey FOREIGN KEY (project_id) REFERENCES public.projects(id) ON DELETE CASCADE;
ALTER TABLE ONLY public.projects
    ADD CONSTRAINT projects_organization_id_fkey FOREIGN KEY (organization_id) REFERENCES public.organizations(id) ON DELETE CASCADE;
ALTER TABLE ONLY public.role_permissions
    ADD CONSTRAINT role_permissions_role_id_fkey FOREIGN KEY (role_id) REFERENCES public.roles(id) ON DELETE CASCADE;
ALTER TABLE ONLY public.roles
    ADD CONSTRAINT roles_owner_org_id_fkey FOREIGN KEY (owner_org_id) REFERENCES public.organizations(id) ON DELETE CASCADE;
ALTER TABLE ONLY public.sessions
    ADD CONSTRAINT sessions_user_id_fkey FOREIGN KEY (user_id) REFERENCES public.users(id) ON DELETE CASCADE;
ALTER TABLE ONLY public.trigger_deliveries
    ADD CONSTRAINT trigger_deliveries_trigger_id_fkey FOREIGN KEY (trigger_id) REFERENCES public.pipeline_triggers(id) ON DELETE CASCADE;
INSERT INTO public.authz_version VALUES (true, 9);
INSERT INTO public.roles VALUES ('organization-member', 'organization-member', 'Organization Member', 'Belongs to the organization: sees it exists, nothing more.', 'organization', NULL, true, '2026-11-03 22:41:42.829515+00', '2026-11-03 22:41:42.829515+00', 'member', 0);
INSERT INTO public.roles VALUES ('organization-viewer', 'organization-viewer', 'Organization Viewer', 'Read every project and run in the organization, change nothing.', 'organization', NULL, true, '2026-11-03 22:41:42.829515+00', '2026-11-03 22:41:42.829515+00', 'member', 0);
INSERT INTO public.roles VALUES ('project-viewer', 'project-viewer', 'Project Viewer', 'Read a project, its pipelines and its runs, change nothing.', 'project', NULL, true, '2026-11-03 22:41:42.829515+00', '2026-11-03 22:41:42.829515+00', 'member', 0);
INSERT INTO public.roles VALUES ('project-developer', 'project-developer', 'Project Developer', 'Build in a project: create, edit and run its pipelines, and cancel their runs.', 'project', NULL, true, '2026-11-03 22:41:42.829515+00', '2026-11-03 22:41:42.829515+00', 'member', 0);
INSERT INTO public.roles VALUES ('system-admin', 'system-admin', 'System Admin', 'Global super-user: full control over every scope.', 'system', NULL, true, '2026-11-03 22:41:42.829515+00', '2026-11-03 22:41:42.829515+00', 'admin', 0);
INSERT INTO public.roles VALUES ('organization-admin', 'organization-admin', 'Organization Admin', 'Owner of an organization and everything beneath it.', 'organization', NULL, true, '2026-11-03 22:41:42.829515+00', '2026-11-03 22:41:42.829515+00', 'admin', 0);
INSERT INTO public.roles VALUES ('project-admin', 'project-admin', 'Project Admin', 'Owner of a project and everything beneath it.', 'project', NULL, true, '2026-11-03 22:41:42.829515+00', '2026-11-03 22:41:42.829515+00', 'admin', 0);
INSERT INTO public.roles VALUES ('organization-agent', 'organization-agent', 'Organization Agent', 'Machine app scoped to an organization: pull and run its jobs.', 'organization', NULL, true, '2026-11-03 22:41:42.829515+00', '2026-11-03 22:41:42.829515+00', 'agent', 0);
INSERT INTO public.roles VALUES ('project-agent', 'project-agent', 'Project Agent', 'Machine app scoped to a project: pull and run its jobs.', 'project', NULL, true, '2026-11-03 22:41:42.829515+00', '2026-11-03 22:41:42.829515+00', 'agent', 0);
INSERT INTO public.roles VALUES ('organization-trigger-runner', 'organization-trigger-runner', 'Organization Trigger Runner', 'Machine app that fires the triggers of an organization: run its pipelines.', 'organization', NULL, true, '2026-11-03 22:41:42.829515+00', '2026-11-03 22:41:42.829515+00', 'agent', 0);
INSERT INTO public.roles VALUES ('organization-creator', 'organization-creator', 'Organization Creator', 'Create new organizations, and nothing else.', 'system', NULL, true, '2026-11-03 22:41:43.048073+00', '2026-11-03 22:41:43.048073+00', 'member', 0);
INSERT INTO public.role_permissions VALUES ('system-admin', '*');
INSERT INTO public.role_permissions VALUES ('organization-admin', '*');
INSERT INTO public.role_permissions VALUES ('project-admin', '*');
INSERT INTO public.role_permissions VALUES ('organization-agent', 'readPipeline');
INSERT INTO public.role_permissions VALUES ('organization-agent', 'executeJob');
INSERT INTO public.role_permissions VALUES ('organization-agent', 'writeJobStatus');
INSERT INTO public.role_permissions VALUES ('organization-agent', 'appendJobLog');
INSERT INTO public.role_permissions VALUES ('project-agent', 'readPipeline');
INSERT INTO public.role_permissions VALUES ('project-agent', 'executeJob');
INSERT INTO public.role_permissions VALUES ('project-agent', 'writeJobStatus');
INSERT INTO public.role_permissions VALUES ('project-agent', 'appendJobLog');
INSERT INTO public.role_permissions VALUES ('organization-trigger-runner', 'runPipeline');
INSERT INTO public.role_permissions VALUES ('organization-member', 'readOrganization');
INSERT INTO public.role_permissions VALUES ('organization-viewer', 'readOrganization');
INSERT INTO public.role_permissions VALUES ('organization-viewer', 'listOrganizationMembers');
INSERT INTO public.role_permissions VALUES ('organization-viewer', 'listProjectsByOrganization');
INSERT INTO public.role_permissions VALUES ('organization-viewer', 'listPipelinesByOrganization');
INSERT INTO public.role_permissions VALUES ('organization-viewer', 'listJobsByOrganization');
INSERT INTO public.role_permissions VALUES ('organization-viewer', 'readProject');
INSERT INTO public.role_permissions VALUES ('organization-viewer', 'readPipeline');
INSERT INTO public.role_permissions VALUES ('organization-viewer', 'listPipelinesByProject');
INSERT INTO public.role_permissions VALUES ('organization-viewer', 'readJob');
INSERT INTO public.role_permissions VALUES ('organization-viewer', 'listJobsByProject');
INSERT INTO public.role_permissions VALUES ('organization-viewer', 'listJobsByPipeline');
INSERT INTO public.role_permissions VALUES ('organization-viewer', 'readJobLogs');
INSERT INTO public.role_permissions VALUES ('project-viewer', 'readProject');
INSERT INTO public.role_permissions VALUES ('project-viewer', 'listProjectMembers');
INSERT INTO public.role_permissions VALUES ('project-viewer', 'readPipeline');
INSERT INTO public.role_permissions VALUES ('project-viewer', 'listPipelinesByProject');
INSERT INTO public.role_permissions VALUES ('project-viewer', 'readJob');
INSERT INTO public.role_permissions VALUES ('project-viewer', 'listJobsByProject');
INSERT INTO public.role_permissions VALUES ('project-viewer', 'listJobsByPipeline');
INSERT INTO public.role_permissions VALUES ('project-viewer', 'readJobLogs');
INSERT INTO public.role_permissions VALUES ('project-developer', 'readProject');
INSERT INTO public.role_permissions VALUES ('project-developer', 'listProjectMembers');
INSERT INTO public.role_permissions VALUES ('project-developer', 'readPipeline');
INSERT INTO public.role_permissions VALUES ('project-developer', 'listPipelinesByProject');
INSERT INTO public.role_permissions VALUES ('project-developer', 'readJob');
INSERT INTO public.role_permissions VALUES ('project-developer', 'listJobsByProject');
INSERT INTO public.role_permissions VALUES ('project-developer', 'listJobsByPipeline');
INSERT INTO public.role_permissions VALUES ('project-developer', 'readJobLogs');
INSERT INTO public.role_permissions VALUES ('project-developer', 'createPipeline');
INSERT INTO public.role_permissions VALUES ('project-developer', 'updatePipeline');
INSERT INTO public.role_permissions VALUES ('project-developer', 'deletePipeline');
INSERT INTO public.role_permissions VALUES ('project-developer', 'runPipeline');
INSERT INTO public.role_permissions VALUES ('project-developer', 'listSecrets');
INSERT INTO public.role_permissions VALUES ('project-developer', 'manageTriggers');
INSERT INTO public.role_permissions VALUES ('organization-viewer', 'listProjectMembers');
INSERT INTO public.role_permissions VALUES ('project-developer', 'updateJob');
INSERT INTO public.role_permissions VALUES ('organization-creator', 'createOrganization');
