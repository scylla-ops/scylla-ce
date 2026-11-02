CREATE TABLE revoked_tokens (
    revocation_id bytea PRIMARY KEY,
    revoked_at timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE assertion_nonces (
    nonce text PRIMARY KEY,
    expires_at timestamptz NOT NULL
);
