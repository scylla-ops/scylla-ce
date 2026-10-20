-- The trigger runner is an App of kind 'trigger_runner', one per organization at most. An
-- existing App named 'trigger-runner' is the runner only if it holds the runner grant.
ALTER TABLE apps ADD COLUMN kind TEXT NOT NULL DEFAULT 'standard'
    CHECK (kind IN ('standard', 'trigger_runner'));

UPDATE apps SET kind = 'trigger_runner'
WHERE name = 'trigger-runner'
  AND EXISTS (
      SELECT 1 FROM grants g
      WHERE g.principal_kind = 'app' AND g.principal_id = apps.id
        AND g.role_id = 'organization-trigger-runner'
        AND g.scope_kind = 'organization' AND g.scope_id = apps.organization_id
  );

CREATE UNIQUE INDEX apps_trigger_runner_idx ON apps (organization_id) WHERE kind = 'trigger_runner';
