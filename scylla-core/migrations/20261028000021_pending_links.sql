-- A link of an identity to an account that waits for the password of the account, for 10 minutes.
-- The cookie of the browser holds the token, and this table holds only its hash.
CREATE TABLE pending_links (
    token_hash bytea PRIMARY KEY,
    provider_id uuid NOT NULL REFERENCES identity_providers (id) ON DELETE CASCADE,
    subject text NOT NULL,
    user_id uuid NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    email text NOT NULL,
    display_name text,
    return_to text NOT NULL,
    expires_at timestamptz NOT NULL
);
