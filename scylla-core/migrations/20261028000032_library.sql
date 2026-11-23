-- The library of a workspace: the fragments that its pipelines share, and their versions. A
-- version does not change after its insert. The links of the pipelines are in
-- `pipelines.fragment_links`.
CREATE TABLE fragments (
    id uuid PRIMARY KEY,
    workspace_id uuid NOT NULL REFERENCES workspaces (id) ON DELETE CASCADE,
    created_by uuid NOT NULL REFERENCES actors (id),
    name text NOT NULL,
    description text NOT NULL DEFAULT '',
    tags text[] NOT NULL DEFAULT '{}',
    created_at timestamptz NOT NULL DEFAULT now(),
    -- The package of an outside registry that the fragment comes from, or NULL.
    origin jsonb,
    CONSTRAINT fragments_name_key UNIQUE (workspace_id, name)
);

CREATE INDEX fragments_created_by_idx ON fragments (created_by);

CREATE TABLE fragment_versions (
    fragment_id uuid NOT NULL REFERENCES fragments (id) ON DELETE CASCADE,
    number integer NOT NULL CHECK (number >= 1),
    nodes jsonb NOT NULL,
    parameters jsonb NOT NULL DEFAULT '[]',
    changes text[] NOT NULL DEFAULT '{}',
    created_by uuid NOT NULL REFERENCES actors (id),
    published_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (fragment_id, number)
);

CREATE INDEX fragment_versions_created_by_idx ON fragment_versions (created_by);

-- The favorites of each reader.
CREATE TABLE fragment_favorites (
    fragment_id uuid NOT NULL REFERENCES fragments (id) ON DELETE CASCADE,
    actor_id uuid NOT NULL REFERENCES actors (id),
    PRIMARY KEY (fragment_id, actor_id)
);

CREATE INDEX fragment_favorites_actor_id_idx ON fragment_favorites (actor_id);
