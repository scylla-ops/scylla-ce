-- Every domain string is checked again when it is read, so stored rows must
-- follow the text rules: one-line fields hold no control character, a username
-- holds no '@', a role name is not blank, and every limit counts UTF-8 bytes.
DO $$
DECLARE
    target RECORD;
BEGIN
    FOR target IN
        SELECT * FROM (VALUES
            ('organizations', 'name'),
            ('projects', 'name'),
            ('pipelines', 'name'),
            ('pipeline_triggers', 'name'),
            ('apps', 'name'),
            ('app_secrets', 'label'),
            ('users', 'username'),
            ('roles', 'name')
        ) AS t (tbl, col)
    LOOP
        EXECUTE format(
            'UPDATE %1$I SET %2$I = COALESCE(NULLIF(btrim(regexp_replace(%2$I, %3$L, '' '', ''g'')), ''''), id) WHERE %2$I ~ %3$L',
            target.tbl, target.col, '[\x01-\x1f\x7f-\x9f]+'
        );
    END LOOP;
END
$$;

UPDATE users SET email = NULL WHERE email ~ '[\x01-\x1f\x7f-\x9f]';

UPDATE users SET username = replace(username, '@', '_at_') WHERE username LIKE '%@%';

UPDATE roles
SET name = CASE WHEN builtin THEN initcap(replace(key, '-', ' ')) ELSE id END
WHERE btrim(name) = '';

UPDATE roles SET name = left(name, 63) WHERE octet_length(name) > 255;

UPDATE roles SET description = left(description, 256) WHERE octet_length(description) > 1024;

UPDATE project_secrets SET description = left(description, 256) WHERE octet_length(description) > 1024;

-- A signature header that is not an HTTP header name never arrived, so such a
-- trigger could not verify a request: it falls back to the default header.
UPDATE pipeline_triggers
SET source = source - 'signature_header'
WHERE kind = 'webhook'
  AND source ->> 'signature_header' !~ '^[A-Za-z0-9!#$%&''*+.^_`|~-]{1,255}$';

-- RFC 6901 allows only '~0' and '~1'. The resolver read any other '~' as a
-- literal '~', which '~0' spells, so each pointer keeps what it resolves to.
UPDATE pipeline_triggers t
SET inputs = (
    SELECT jsonb_agg(
        CASE
            WHEN e.input -> 'source' ? 'json_pointer' THEN jsonb_set(
                e.input,
                '{source,json_pointer}',
                to_jsonb(regexp_replace(e.input -> 'source' ->> 'json_pointer', '~(?![01])', '~0', 'g'))
            )
            ELSE e.input
        END
        ORDER BY e.ord
    )
    FROM jsonb_array_elements(t.inputs) WITH ORDINALITY AS e (input, ord)
)
WHERE EXISTS (
    SELECT 1
    FROM jsonb_array_elements(t.inputs) AS i (input)
    WHERE i.input -> 'source' ->> 'json_pointer' ~ '~(?![01])'
);

-- The agent refuses an env var key longer than 255 bytes, so each run with
-- such a key fails: the trigger input or the node env var is removed.
UPDATE pipeline_triggers t
SET inputs = COALESCE((
    SELECT jsonb_agg(e.input ORDER BY e.ord)
    FROM jsonb_array_elements(t.inputs) WITH ORDINALITY AS e (input, ord)
    WHERE octet_length(e.input ->> 'key') <= 255
), '[]')
WHERE EXISTS (
    SELECT 1
    FROM jsonb_array_elements(t.inputs) AS i (input)
    WHERE octet_length(i.input ->> 'key') > 255
);

UPDATE pipelines p
SET nodes = (
    SELECT jsonb_agg(
        jsonb_set(n.node, '{env}', COALESCE((
            SELECT jsonb_agg(e.var ORDER BY e.ord)
            FROM jsonb_array_elements(n.node -> 'env') WITH ORDINALITY AS e (var, ord)
            WHERE octet_length(e.var ->> 'key') <= 255
        ), '[]'))
        ORDER BY n.ord
    )
    FROM jsonb_array_elements(p.nodes) WITH ORDINALITY AS n (node, ord)
)
WHERE EXISTS (
    SELECT 1
    FROM jsonb_array_elements(p.nodes) AS n (node), jsonb_array_elements(n.node -> 'env') AS e (var)
    WHERE octet_length(e.var ->> 'key') > 255
);
