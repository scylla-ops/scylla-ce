-- The acceptances of the terms of service: one row for each version that a user accepted, the
-- proof of the acceptance. The rows go with the account.
CREATE TABLE terms_acceptances (
    user_id uuid NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    kind text NOT NULL CHECK (kind IN ('terms')),
    version text NOT NULL,
    method text NOT NULL CHECK (method IN ('sign_up_form', 'acceptance_step', 're_acceptance')),
    accepted_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (user_id, kind, version)
);
