-- The facts for the notifications: an outbox that one task reads, in the transaction of the write.
CREATE TABLE notification_events (
    id uuid PRIMARY KEY,
    kind text NOT NULL,
    actor_id uuid REFERENCES actors (id),
    workspace_id uuid REFERENCES workspaces (id) ON DELETE CASCADE,
    project_id uuid REFERENCES projects (id) ON DELETE CASCADE,
    subject_kind text,
    subject_id uuid,
    params jsonb NOT NULL DEFAULT '{}',
    dedupe_key text,
    group_key text,
    attempts integer NOT NULL DEFAULT 0,
    next_attempt_at timestamptz NOT NULL DEFAULT now(),
    dispatched_at timestamptz,
    created_at timestamptz NOT NULL DEFAULT now()
);
CREATE UNIQUE INDEX notification_events_dedupe_idx
    ON notification_events (dedupe_key) WHERE dedupe_key IS NOT NULL;
CREATE INDEX notification_events_pending_idx
    ON notification_events (next_attempt_at) WHERE dispatched_at IS NULL;
CREATE INDEX notification_events_subject_idx
    ON notification_events (subject_kind, subject_id) WHERE dispatched_at IS NULL;
CREATE INDEX notification_events_actor_id_idx ON notification_events (actor_id);
CREATE INDEX notification_events_workspace_id_idx ON notification_events (workspace_id);
CREATE INDEX notification_events_project_id_idx ON notification_events (project_id);

-- The inbox of a user: one row for each person and each group of facts. A row holds a kind and its
-- parameters, never a sentence.
CREATE TABLE notifications (
    id uuid PRIMARY KEY,
    recipient_id uuid NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    kind text NOT NULL,
    reason text NOT NULL,
    actor_id uuid REFERENCES actors (id),
    workspace_id uuid REFERENCES workspaces (id) ON DELETE CASCADE,
    project_id uuid REFERENCES projects (id) ON DELETE CASCADE,
    subject_kind text,
    subject_id uuid,
    params jsonb NOT NULL DEFAULT '{}',
    group_key text NOT NULL,
    occurrences integer NOT NULL DEFAULT 1,
    needs_action boolean NOT NULL DEFAULT false,
    first_at timestamptz NOT NULL DEFAULT now(),
    last_at timestamptz NOT NULL DEFAULT now(),
    read_at timestamptz,
    done_at timestamptz,
    closed_at timestamptz
);
CREATE UNIQUE INDEX notifications_open_group_idx
    ON notifications (recipient_id, group_key) WHERE closed_at IS NULL;
CREATE INDEX notifications_unread_idx
    ON notifications (recipient_id) WHERE read_at IS NULL AND done_at IS NULL;
CREATE INDEX notifications_list_idx ON notifications (recipient_id, last_at DESC);
CREATE INDEX notifications_subject_idx ON notifications (subject_kind, subject_id);
CREATE INDEX notifications_workspace_id_idx ON notifications (workspace_id);
CREATE INDEX notifications_project_id_idx ON notifications (project_id);
CREATE INDEX notifications_actor_id_idx ON notifications (actor_id);
