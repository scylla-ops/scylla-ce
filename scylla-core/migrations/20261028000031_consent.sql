-- The record of each choice of a browser on the trackers that need consent: the proof of a
-- consent. A withdrawal is a new record. The records of one browser share its consent id. A
-- record of a signed-in user names the user, and goes with the user. No IP address.
CREATE TABLE consent_records (
    id uuid PRIMARY KEY,
    consent_id uuid NOT NULL,
    user_id uuid REFERENCES users (id) ON DELETE CASCADE,
    banner_version text NOT NULL,
    banner_hash text NOT NULL,
    choices jsonb NOT NULL,
    action text NOT NULL CHECK (action IN ('accept_all', 'reject_all', 'custom', 'withdraw')),
    source text NOT NULL CHECK (source IN ('banner', 'preferences', 'account')),
    recorded_at timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX consent_records_consent_id_idx ON consent_records (consent_id);
CREATE INDEX consent_records_user_id_idx ON consent_records (user_id);
