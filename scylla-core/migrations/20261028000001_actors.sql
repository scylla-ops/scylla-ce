CREATE TABLE actors (
    id uuid PRIMARY KEY,
    kind text NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now()
);
