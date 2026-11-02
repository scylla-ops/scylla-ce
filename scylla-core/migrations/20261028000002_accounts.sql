CREATE TABLE users (
    id uuid PRIMARY KEY REFERENCES actors (id),
    display_name text NOT NULL,
    status text NOT NULL CHECK (status IN ('pending', 'active', 'disabled')),
    created_at timestamptz NOT NULL DEFAULT now(),
    last_sign_in_at timestamptz
);

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
