-- The operator of the instance and its privacy contact: one row. An empty text is no value.
CREATE TABLE legal_settings (
    id boolean PRIMARY KEY DEFAULT true CHECK (id),
    operator_name text NOT NULL DEFAULT '',
    operator_address text NOT NULL DEFAULT '',
    operator_registration text NOT NULL DEFAULT '',
    publication_director text NOT NULL DEFAULT '',
    operator_contact text NOT NULL DEFAULT '',
    operator_host text NOT NULL DEFAULT '',
    privacy_contact text NOT NULL DEFAULT ''
);

INSERT INTO legal_settings DEFAULT VALUES;

-- The legal documents that are on: one row for each kind at most. A kind with no row is off.
CREATE TABLE legal_documents (
    kind text PRIMARY KEY CHECK (
        kind IN ('legal-notice', 'privacy', 'cookies', 'terms', 'terms-of-sale', 'dpa', 'subprocessors')
    ),
    title text NOT NULL DEFAULT '',
    url text,
    markdown text,
    version text NOT NULL DEFAULT '',
    effective_date date,
    acceptance_required boolean NOT NULL DEFAULT false,
    changes text NOT NULL DEFAULT '',
    updated_at timestamptz NOT NULL DEFAULT now(),
    CHECK ((url IS NULL) <> (markdown IS NULL))
);
