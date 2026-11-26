-- The SMTP relay of the instance, which an instance admin sets at runtime: at most one row. With
-- no row, the server uses its default mailer (SCYLLA_SMTP_URL, else a log of the mails). The
-- password is sealed with the key of the secrets of the server.
CREATE TABLE mail_settings (
    id boolean PRIMARY KEY DEFAULT true CHECK (id),
    host text NOT NULL,
    port integer NOT NULL CHECK (port BETWEEN 1 AND 65535),
    security text NOT NULL CHECK (security IN ('tls', 'starttls', 'none')),
    username text NOT NULL DEFAULT '',
    password bytea,
    from_address text NOT NULL,
    from_name text NOT NULL DEFAULT '',
    updated_at timestamptz NOT NULL DEFAULT now()
);
