-- Optimistic concurrency for projects: an update or a delete names the version it read,
-- and the write goes through only if the row still carries it.
ALTER TABLE projects ADD COLUMN version BIGINT NOT NULL DEFAULT 0;
