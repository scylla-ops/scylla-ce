-- Role display names are unique across the catalog. The builtin, or else the
-- oldest custom role, keeps a shared name; the later ones get their id appended.
UPDATE roles SET name = btrim(name) WHERE name <> btrim(name);

UPDATE roles r
SET name = CASE WHEN octet_length(r.name) > 226 THEN left(r.name, 56) ELSE r.name END
    || ' (' || r.id || ')'
WHERE NOT r.builtin
  AND EXISTS (
      SELECT 1 FROM roles o
      WHERE o.name = r.name AND o.id <> r.id AND (o.builtin OR o.id < r.id)
  );

CREATE UNIQUE INDEX roles_name_key ON roles (name);
