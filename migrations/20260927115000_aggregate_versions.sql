-- Optimistic concurrency for pipelines, organizations, triggers and users: an update or a
-- delete names the version it read, and the write goes through only if the row still carries it.
ALTER TABLE pipelines ADD COLUMN version BIGINT NOT NULL DEFAULT 0;
ALTER TABLE organizations ADD COLUMN version BIGINT NOT NULL DEFAULT 0;
ALTER TABLE pipeline_triggers ADD COLUMN version BIGINT NOT NULL DEFAULT 0;
ALTER TABLE users ADD COLUMN version BIGINT NOT NULL DEFAULT 0;
