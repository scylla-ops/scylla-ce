-- The places of the nodes on the canvas of a pipeline. They are apart from `pipelines.nodes`:
-- the version of a pipeline and the snapshot of a job do not see them.
CREATE TABLE pipeline_layouts (
    pipeline_id uuid PRIMARY KEY REFERENCES pipelines (id) ON DELETE CASCADE,
    start jsonb,
    nodes jsonb NOT NULL DEFAULT '{}',
    updated_at timestamptz NOT NULL DEFAULT now()
);
