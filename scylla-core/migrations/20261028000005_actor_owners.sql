ALTER TABLE actors ADD COLUMN owner_id uuid REFERENCES actors (id);

CREATE INDEX actors_owner_id_idx ON actors (owner_id);

UPDATE actors SET owner_id = agents.registered_by
FROM agents
WHERE agents.id = actors.id;

ALTER TABLE agents DROP COLUMN registered_by;
