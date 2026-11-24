CREATE TABLE instance_admins (
    user_id uuid PRIMARY KEY REFERENCES users (id) ON DELETE CASCADE,
    created_at timestamptz NOT NULL DEFAULT now()
);
