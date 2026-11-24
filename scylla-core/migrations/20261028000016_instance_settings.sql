CREATE TABLE instance_settings (
    id boolean PRIMARY KEY DEFAULT true CHECK (id),
    personal_tokens_enabled boolean NOT NULL DEFAULT true
);

INSERT INTO instance_settings DEFAULT VALUES;
