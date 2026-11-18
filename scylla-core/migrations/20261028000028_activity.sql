-- The activity feed of a workspace: one row for each change of its jobs, triggers, secrets,
-- members, Git connections and agents, in the transaction of the change. A row holds a kind and
-- its parameters, never a sentence, and shows only to the readers of its area. A row stays 30 days.
CREATE TABLE activity (
    id uuid PRIMARY KEY,
    workspace_id uuid NOT NULL REFERENCES workspaces (id) ON DELETE CASCADE,
    project_id uuid REFERENCES projects (id) ON DELETE CASCADE,
    area text NOT NULL,
    kind text NOT NULL,
    actor_id uuid REFERENCES actors (id),
    subject_kind text,
    subject_id uuid,
    params jsonb NOT NULL DEFAULT '{}',
    created_at timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX activity_list_idx ON activity (workspace_id, created_at DESC, id DESC);
CREATE INDEX activity_project_id_idx ON activity (project_id);
CREATE INDEX activity_actor_id_idx ON activity (actor_id);
CREATE INDEX activity_created_at_idx ON activity (created_at);
