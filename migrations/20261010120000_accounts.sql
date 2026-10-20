-- The name that people read. It is not unique.
ALTER TABLE users ADD COLUMN display_name TEXT NULL;

-- A reset link of a user. The table keeps the SHA-256 of the token, in
-- lowercase hex, as for a session. A link is single-use: used_at is set when
-- the password changes through it.
CREATE TABLE password_resets (
    id         TEXT        PRIMARY KEY,
    user_id    TEXT        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    token_hash TEXT        NOT NULL,
    created_at TIMESTAMPTZ NOT NULL,
    expires_at TIMESTAMPTZ NOT NULL,
    used_at    TIMESTAMPTZ NULL
);

CREATE UNIQUE INDEX password_resets_token_hash_key ON password_resets (token_hash);
CREATE INDEX password_resets_user_id_idx ON password_resets (user_id);
