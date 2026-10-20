-- A job runs the nodes it was created with, and every write of a job names the
-- version it read. A job created before this migration gets the nodes of its
-- pipeline as they are now; the dispatcher reads them only for a pending job.
ALTER TABLE jobs ADD COLUMN nodes JSONB;
UPDATE jobs j SET nodes = p.nodes FROM pipelines p WHERE p.id = j.pipeline_id;
ALTER TABLE jobs ALTER COLUMN nodes SET NOT NULL;
ALTER TABLE jobs ADD COLUMN version BIGINT NOT NULL DEFAULT 0;

-- A job placed on an agent names the agent stream that took it, and only a
-- release of that stream returns the job to the pool. The streams of the earlier
-- dispatcher are gone after the restart, so each pending job goes back to the pool.
ALTER TABLE jobs ADD COLUMN stream_id TEXT;
UPDATE jobs SET agent_app_id = NULL WHERE status = 'pending';
CREATE INDEX jobs_pool_idx ON jobs (created_at) WHERE status = 'pending' AND agent_app_id IS NULL;

-- Who may run the pipelines of a project may cancel their jobs.
INSERT INTO role_permissions (role_id, permission)
SELECT 'project-developer', 'updateJob'
WHERE EXISTS (SELECT 1 FROM roles WHERE id = 'project-developer')
ON CONFLICT DO NOTHING;

UPDATE roles
SET description = 'Build in a project: create, edit and run its pipelines, and cancel their runs.'
WHERE id = 'project-developer'
  AND description = 'Build in a project: create, edit and run its pipelines.';
