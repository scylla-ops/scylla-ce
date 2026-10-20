-- A role name is unique per owner without regard to case. An organization role
-- never takes the name of a platform role. A new platform role may take the name
-- of an organization role: that organization then sees both, each in its group.
DROP INDEX roles_name_key;
CREATE UNIQUE INDEX roles_name_key ON roles (owner_org_id, lower(name)) NULLS NOT DISTINCT;

CREATE OR REPLACE FUNCTION check_role_name() RETURNS TRIGGER AS $$
BEGIN
    IF NEW.owner_org_id IS NOT NULL AND EXISTS (
        SELECT 1 FROM roles r
        WHERE r.owner_org_id IS NULL
          AND lower(r.name) = lower(NEW.name)
    ) THEN
        RAISE EXCEPTION 'role name % is taken', NEW.name
            USING ERRCODE = 'unique_violation', CONSTRAINT = 'roles_name_key';
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

DROP TRIGGER roles_check_name ON roles;
CREATE TRIGGER roles_check_name BEFORE INSERT OR UPDATE OF name, owner_org_id ON roles
    FOR EACH ROW EXECUTE FUNCTION check_role_name();

-- The binding checks also run when a statement changes a grant, an invitation or
-- a role that is given. No code path does that today; the database refuses it all
-- the same.
DROP TRIGGER grants_check_references ON grants;
CREATE TRIGGER grants_check_references
    BEFORE INSERT OR UPDATE OF principal_kind, principal_id, scope_kind, scope_id, role_id ON grants
    FOR EACH ROW EXECUTE FUNCTION check_grant_references();

DROP TRIGGER organization_invites_check_role ON organization_invites;
CREATE TRIGGER organization_invites_check_role
    BEFORE INSERT OR UPDATE OF role_name, organization_id ON organization_invites
    FOR EACH ROW EXECUTE FUNCTION check_invite_role();

CREATE FUNCTION check_role_change() RETURNS TRIGGER AS $$
BEGIN
    IF EXISTS (
        SELECT 1 FROM grants g
        LEFT JOIN projects p ON g.scope_kind = 'project' AND p.id = g.scope_id
        WHERE g.role_id = NEW.id
          AND (g.scope_kind <> NEW.scope_kind
               OR (NEW.kind = 'agent' AND g.principal_kind = 'user')
               OR (NEW.owner_org_id IS NOT NULL
                   AND NEW.owner_org_id IS DISTINCT FROM CASE g.scope_kind
                       WHEN 'organization' THEN g.scope_id
                       WHEN 'project' THEN p.organization_id
                   END))
    ) OR EXISTS (
        SELECT 1 FROM organization_invites i
        WHERE i.role_name = NEW.id
          AND i.status = 'pending'
          AND (NEW.scope_kind <> 'organization'
               OR NEW.kind = 'agent'
               OR (NEW.owner_org_id IS NOT NULL AND NEW.owner_org_id IS DISTINCT FROM i.organization_id))
    ) THEN
        RAISE EXCEPTION 'role % is given where its new kind, scope or owner does not allow it', NEW.id
            USING ERRCODE = 'foreign_key_violation';
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER roles_check_change BEFORE UPDATE OF kind, scope_kind, owner_org_id ON roles
    FOR EACH ROW EXECUTE FUNCTION check_role_change();
