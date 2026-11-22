-- The acceptances of the terms of service: one row for each version that a user accepted, the
-- proof of the acceptance. The rows of a deleted account stay as the proof, with the time of the
-- deletion in `account_deleted_at` and the SHA-256 hash of the primary email of the account in
-- lowercase in `email_hash`. The retention task deletes them some time after the deletion (5 years
-- by default).
CREATE TABLE terms_acceptances (
    user_id uuid NOT NULL REFERENCES actors (id),
    kind text NOT NULL CHECK (kind IN ('terms')),
    version text NOT NULL,
    method text NOT NULL CHECK (method IN ('sign_up_form', 'acceptance_step', 're_acceptance')),
    accepted_at timestamptz NOT NULL DEFAULT now(),
    account_deleted_at timestamptz,
    email_hash bytea,
    PRIMARY KEY (user_id, kind, version)
);
