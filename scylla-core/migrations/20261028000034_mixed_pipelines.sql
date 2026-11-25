-- The editor pipelines next to the file pipelines of a project of files. A file of the default
-- branch that takes the key of an editor pipeline marks it (key_taken_at): the file wins, and the
-- editor pipeline sends no report. A workspace admin lets the push, pull request and tag
-- triggers of an editor pipeline start runs in a project of files (git_events_allowed_at and
-- git_events_allowed_by). Each column is nullable: an older server does not write them.
ALTER TABLE pipelines
    ADD COLUMN key_taken_at timestamptz,
    ADD COLUMN git_events_allowed_at timestamptz,
    ADD COLUMN git_events_allowed_by uuid REFERENCES actors (id);

CREATE INDEX pipelines_git_events_allowed_by_idx ON pipelines (git_events_allowed_by);
