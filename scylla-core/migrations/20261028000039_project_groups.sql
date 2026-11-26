-- The groups of the projects of a workspace. The members of the workspace share the groups, the
-- order of the groups and the order of the projects in each group.
CREATE TABLE project_groups (
    id uuid PRIMARY KEY,
    workspace_id uuid NOT NULL REFERENCES workspaces (id) ON DELETE CASCADE,
    created_by uuid NOT NULL REFERENCES actors (id),
    name text NOT NULL,
    position integer NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX project_groups_workspace_id_idx ON project_groups (workspace_id);
CREATE INDEX project_groups_created_by_idx ON project_groups (created_by);

-- The group of a project and its place in the group, or in the projects with no group. A project
-- with no place comes after the others of its group, oldest first.
ALTER TABLE projects
    ADD COLUMN group_id uuid REFERENCES project_groups (id) ON DELETE SET NULL,
    ADD COLUMN position integer;

CREATE INDEX projects_group_id_idx ON projects (group_id);
