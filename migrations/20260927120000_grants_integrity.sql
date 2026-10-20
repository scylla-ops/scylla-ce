-- The principal and the scope of a grant are polymorphic, so no foreign key
-- holds them. The delete triggers of the init migration complete the cascade;
-- the trigger below completes the check on insert. FOR KEY SHARE locks the
-- referenced row as a foreign key does: a concurrent delete waits, and its
-- trigger then removes the new grant.

DELETE FROM grants g
WHERE (g.principal_kind = 'user' AND NOT EXISTS (SELECT 1 FROM users u WHERE u.id = g.principal_id))
   OR (g.principal_kind = 'app' AND NOT EXISTS (SELECT 1 FROM apps a WHERE a.id = g.principal_id))
   OR (g.scope_kind = 'organization' AND NOT EXISTS (SELECT 1 FROM organizations o WHERE o.id = g.scope_id))
   OR (g.scope_kind = 'project' AND NOT EXISTS (SELECT 1 FROM projects p WHERE p.id = g.scope_id));

-- An app acts only in the organization that owns it.
DELETE FROM grants g
USING apps a
WHERE g.principal_kind = 'app'
  AND a.id = g.principal_id
  AND ((g.scope_kind = 'organization' AND g.scope_id <> a.organization_id)
    OR (g.scope_kind = 'project' AND NOT EXISTS (
          SELECT 1 FROM projects p WHERE p.id = g.scope_id AND p.organization_id = a.organization_id)));

-- The agent roles are for apps only, and an invitation makes a user grant.
DELETE FROM grants
WHERE principal_kind = 'user'
  AND role_id IN ('organization-agent', 'project-agent', 'organization-trigger-runner');

UPDATE organization_invites
SET status = 'revoked'
WHERE status = 'pending'
  AND role_name IN ('organization-agent', 'project-agent', 'organization-trigger-runner');

CREATE FUNCTION check_grant_references() RETURNS TRIGGER AS $$
BEGIN
    IF NEW.principal_kind = 'user' THEN
        PERFORM 1 FROM users WHERE id = NEW.principal_id FOR KEY SHARE;
    ELSE
        PERFORM 1 FROM apps WHERE id = NEW.principal_id FOR KEY SHARE;
    END IF;
    IF NOT FOUND THEN
        RAISE EXCEPTION 'grant principal %:% does not exist', NEW.principal_kind, NEW.principal_id
            USING ERRCODE = 'foreign_key_violation';
    END IF;

    IF NEW.scope_kind = 'organization' THEN
        PERFORM 1 FROM organizations WHERE id = NEW.scope_id FOR KEY SHARE;
    ELSIF NEW.scope_kind = 'project' THEN
        PERFORM 1 FROM projects WHERE id = NEW.scope_id FOR KEY SHARE;
    ELSE
        RETURN NEW;
    END IF;
    IF NOT FOUND THEN
        RAISE EXCEPTION 'grant scope %:% does not exist', NEW.scope_kind, NEW.scope_id
            USING ERRCODE = 'foreign_key_violation';
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER grants_check_references BEFORE INSERT ON grants
    FOR EACH ROW EXECUTE FUNCTION check_grant_references();
