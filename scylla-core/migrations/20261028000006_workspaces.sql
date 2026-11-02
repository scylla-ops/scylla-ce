CREATE TABLE workspaces (
    id uuid PRIMARY KEY,
    owner_id uuid NOT NULL REFERENCES actors (id),
    name text NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX workspaces_owner_id_idx ON workspaces (owner_id);

INSERT INTO workspaces (id, owner_id, name)
SELECT uuidv7(), id, 'Personal' FROM actors WHERE kind = 'user';

ALTER TABLE projects ADD COLUMN workspace_id uuid REFERENCES workspaces (id);

UPDATE projects
SET workspace_id = workspaces.id
FROM actors, workspaces
WHERE actors.id = projects.owner_id
  AND workspaces.owner_id = CASE WHEN actors.kind = 'user' THEN actors.id ELSE actors.owner_id END;

DELETE FROM projects WHERE workspace_id IS NULL;

ALTER TABLE projects ALTER COLUMN workspace_id SET NOT NULL;
DROP INDEX projects_owner_id_idx;
ALTER TABLE projects DROP COLUMN owner_id;
CREATE INDEX projects_workspace_id_idx ON projects (workspace_id);

ALTER TABLE agents ADD COLUMN workspace_id uuid REFERENCES workspaces (id);

UPDATE agents
SET workspace_id = workspaces.id
FROM actors, workspaces
WHERE actors.id = agents.id AND workspaces.owner_id = actors.owner_id;

DELETE FROM agent_keys WHERE agent_id IN (SELECT id FROM agents WHERE workspace_id IS NULL);
DELETE FROM agents WHERE workspace_id IS NULL;

ALTER TABLE agents ALTER COLUMN workspace_id SET NOT NULL;
CREATE INDEX agents_workspace_id_idx ON agents (workspace_id);

DROP INDEX actors_owner_id_idx;
ALTER TABLE actors DROP COLUMN owner_id;
