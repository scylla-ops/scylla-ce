-- The number of workspaces that one user can create.
ALTER TABLE instance_settings
    ADD COLUMN workspace_limit integer NOT NULL DEFAULT 1 CHECK (workspace_limit >= 0);

-- The user whose limit counts a workspace. A personal workspace counts for no user.
ALTER TABLE workspaces ADD COLUMN counts_for uuid REFERENCES actors (id);

CREATE INDEX workspaces_counts_for ON workspaces (counts_for) WHERE counts_for IS NOT NULL;

-- Each user creates workspaces now, so the role of the workspace creators goes. A custom role is
-- then the only kind of row.
DELETE FROM roles WHERE id = '00000000-0000-0000-0000-000000000001';

ALTER TABLE roles DROP COLUMN builtin;
