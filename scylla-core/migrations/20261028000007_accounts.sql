CREATE TABLE identity_providers (
    id uuid PRIMARY KEY,
    slug text NOT NULL UNIQUE,
    kind text NOT NULL,
    display_name text NOT NULL,
    settings jsonb NOT NULL,
    client_secret bytea,
    allow_sign_up boolean NOT NULL DEFAULT false,
    enabled boolean NOT NULL DEFAULT true,
    created_at timestamptz NOT NULL DEFAULT now()
);

INSERT INTO identity_providers (id, slug, kind, display_name, settings, enabled)
SELECT uuidv7(),
       CASE WHEN rank = 1 THEN 'oidc' ELSE 'oidc-' || rank END,
       'oidc',
       issuer,
       jsonb_build_object('issuer', issuer, 'client_id', ''),
       false
FROM (SELECT issuer, row_number() OVER (ORDER BY issuer) AS rank
      FROM (SELECT DISTINCT issuer FROM users) AS issuers) AS ranked;

CREATE TABLE external_identities (
    provider_id uuid NOT NULL REFERENCES identity_providers (id) ON DELETE CASCADE,
    subject text NOT NULL,
    user_id uuid NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    email text,
    linked_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (provider_id, subject)
);

CREATE INDEX external_identities_user_id_idx ON external_identities (user_id);

INSERT INTO external_identities (provider_id, subject, user_id)
SELECT identity_providers.id, users.subject, users.id
FROM users
JOIN identity_providers ON identity_providers.settings ->> 'issuer' = users.issuer;

ALTER TABLE users DROP COLUMN issuer, DROP COLUMN subject;
ALTER TABLE users
    ADD COLUMN display_name text NOT NULL DEFAULT '',
    ADD COLUMN status text NOT NULL DEFAULT 'active'
        CHECK (status IN ('pending', 'active', 'disabled')),
    ADD COLUMN created_at timestamptz NOT NULL DEFAULT now(),
    ADD COLUMN last_sign_in_at timestamptz;

INSERT INTO users (id)
SELECT id FROM actors WHERE kind = 'user' AND id NOT IN (SELECT id FROM users);

ALTER TABLE users ALTER COLUMN display_name DROP DEFAULT, ALTER COLUMN status DROP DEFAULT;

CREATE TABLE user_emails (
    email text PRIMARY KEY,
    user_id uuid NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    verified_at timestamptz,
    is_primary boolean NOT NULL DEFAULT true,
    created_at timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX user_emails_user_id_idx ON user_emails (user_id);
CREATE UNIQUE INDEX user_emails_one_primary_idx ON user_emails (user_id) WHERE is_primary;

CREATE TABLE passwords (
    user_id uuid PRIMARY KEY REFERENCES users (id) ON DELETE CASCADE,
    hash text NOT NULL,
    failed_attempts integer NOT NULL DEFAULT 0,
    locked_until timestamptz,
    updated_at timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE sessions (
    id uuid PRIMARY KEY,
    user_id uuid NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    refresh_hash bytea NOT NULL,
    methods text[] NOT NULL,
    user_agent text,
    created_at timestamptz NOT NULL DEFAULT now(),
    last_used_at timestamptz NOT NULL DEFAULT now(),
    expires_at timestamptz NOT NULL,
    revoked_at timestamptz
);

CREATE INDEX sessions_user_id_idx ON sessions (user_id);

CREATE TABLE one_time_tokens (
    hash bytea PRIMARY KEY,
    purpose text NOT NULL,
    user_id uuid NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    email text NOT NULL,
    expires_at timestamptz NOT NULL,
    used_at timestamptz,
    created_at timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX one_time_tokens_user_id_idx ON one_time_tokens (user_id);

CREATE TABLE sign_in_flows (
    state_hash bytea PRIMARY KEY,
    provider_id uuid NOT NULL REFERENCES identity_providers (id) ON DELETE CASCADE,
    browser_hash bytea NOT NULL,
    pkce_verifier text NOT NULL,
    nonce text NOT NULL,
    link_user_id uuid REFERENCES users (id) ON DELETE CASCADE,
    return_to text NOT NULL,
    expires_at timestamptz NOT NULL
);

CREATE TABLE invitations (
    id uuid PRIMARY KEY,
    workspace_id uuid NOT NULL REFERENCES workspaces (id) ON DELETE CASCADE,
    email text NOT NULL,
    role text NOT NULL,
    token_hash bytea NOT NULL UNIQUE,
    invited_by uuid NOT NULL REFERENCES actors (id),
    expires_at timestamptz NOT NULL,
    accepted_by uuid REFERENCES users (id),
    accepted_at timestamptz,
    created_at timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX invitations_workspace_id_idx ON invitations (workspace_id);
CREATE INDEX invitations_email_idx ON invitations (email);

ALTER TABLE workspaces DROP CONSTRAINT workspaces_owner_id_fkey;
ALTER TABLE workspaces
    ADD CONSTRAINT workspaces_owner_id_fkey FOREIGN KEY (owner_id) REFERENCES users (id);
