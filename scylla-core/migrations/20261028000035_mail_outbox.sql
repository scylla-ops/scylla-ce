-- The outbox of the mails. A change puts its mail here in its transaction, and one task of the
-- server sends it later, again after a transient error, for at most 24 hours. A sent or dropped
-- mail keeps no content: only its kind, its masked recipient, its state, its attempts and its
-- times.
CREATE TABLE mail_outbox (
    id uuid PRIMARY KEY,
    kind text NOT NULL,
    recipient text NOT NULL,
    content jsonb,
    state text NOT NULL DEFAULT 'pending' CHECK (state IN ('pending', 'sent', 'dropped')),
    attempts integer NOT NULL DEFAULT 0 CHECK (attempts >= 0),
    next_attempt_at timestamptz,
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now(),
    CHECK ((state = 'pending') = (content IS NOT NULL)),
    CHECK ((state = 'pending') = (next_attempt_at IS NOT NULL))
);

CREATE INDEX mail_outbox_due_idx ON mail_outbox (next_attempt_at) WHERE state = 'pending';

CREATE INDEX mail_outbox_done_idx ON mail_outbox (updated_at) WHERE state <> 'pending';
