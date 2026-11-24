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

CREATE TABLE external_identities (
    provider_id uuid NOT NULL REFERENCES identity_providers (id) ON DELETE CASCADE,
    subject text NOT NULL,
    user_id uuid NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    email text,
    linked_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (provider_id, subject)
);

CREATE INDEX external_identities_user_id_idx ON external_identities (user_id);

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
