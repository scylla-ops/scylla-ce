CREATE TABLE ci_identities (
    id uuid PRIMARY KEY,
    app_id uuid NOT NULL REFERENCES apps (id) ON DELETE CASCADE,
    provider text NOT NULL CHECK (provider IN ('github', 'gitlab')),
    issuer text NOT NULL,
    repository text NOT NULL,
    git_ref text,
    environment text,
    preset text NOT NULL CHECK (preset IN ('read_only', 'deploy', 'full', 'custom')),
    actions text[] NOT NULL,
    project_ids uuid[] NOT NULL,
    created_by uuid NOT NULL REFERENCES actors (id),
    created_at timestamptz NOT NULL DEFAULT now(),
    last_used_at timestamptz
);

CREATE INDEX ci_identities_app_id_idx ON ci_identities (app_id, created_at);
CREATE INDEX ci_identities_created_by_idx ON ci_identities (created_by);
