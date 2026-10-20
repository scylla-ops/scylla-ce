-- A role with no owner is a platform role: the builtins and the custom roles of
-- the system administrators. A role with an owner belongs to that organization:
-- it is organization or project scoped, and only that organization sees and
-- grants it. From here grants.role_id and organization_invites.role_name
-- reference roles; the grant cleanup comment of the init migration no longer
-- applies.

ALTER TABLE roles
    ADD COLUMN kind TEXT NOT NULL DEFAULT 'member' CHECK (kind IN ('admin', 'member', 'agent')),
    ADD COLUMN version BIGINT NOT NULL DEFAULT 0;
UPDATE roles SET kind = 'admin'
WHERE id IN ('system-admin', 'organization-admin', 'project-admin');
UPDATE roles SET kind = 'agent'
WHERE id IN ('organization-agent', 'project-agent', 'organization-trigger-runner');

ALTER TABLE roles
    ADD CONSTRAINT roles_owned_scope CHECK (owner_org_id IS NULL OR scope_kind IN ('organization', 'project')),
    ADD CONSTRAINT roles_owned_custom CHECK (owner_org_id IS NULL OR NOT builtin),
    ADD CONSTRAINT roles_admin_builtin CHECK (kind <> 'admin' OR builtin);

DROP INDEX roles_name_key;
DROP INDEX roles_owner_org_idx;
CREATE UNIQUE INDEX roles_name_key ON roles (owner_org_id, name) NULLS NOT DISTINCT;

-- An organization sees the platform roles next to its own, so a name is either
-- a platform name or an organization name. Two organizations may share one.
CREATE FUNCTION check_role_name() RETURNS TRIGGER AS $$
BEGIN
    IF EXISTS (
        SELECT 1 FROM roles r
        WHERE r.name = NEW.name
          AND r.id <> NEW.id
          AND (r.owner_org_id IS NULL) <> (NEW.owner_org_id IS NULL)
    ) THEN
        RAISE EXCEPTION 'role name % is taken', NEW.name
            USING ERRCODE = 'unique_violation', CONSTRAINT = 'roles_name_key';
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER roles_check_name BEFORE INSERT OR UPDATE OF name ON roles
    FOR EACH ROW EXECUTE FUNCTION check_role_name();

-- The one rule for a role given to a principal at a scope, for grants and
-- invitations alike. The use cases refuse first, with a better message.
CREATE FUNCTION check_role_binding(role TEXT, principal TEXT, scope TEXT, organization TEXT)
RETURNS VOID AS $$
DECLARE
    r RECORD;
BEGIN
    SELECT scope_kind, owner_org_id, kind INTO r FROM roles WHERE id = role FOR KEY SHARE;
    IF NOT FOUND
       OR r.scope_kind <> scope
       OR (r.kind = 'agent' AND principal = 'user')
       OR (r.owner_org_id IS NOT NULL AND r.owner_org_id IS DISTINCT FROM organization) THEN
        RAISE EXCEPTION 'role % cannot be given here', role
            USING ERRCODE = 'foreign_key_violation';
    END IF;
END;
$$ LANGUAGE plpgsql;

CREATE OR REPLACE FUNCTION check_grant_references() RETURNS TRIGGER AS $$
DECLARE
    organization TEXT;
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
        SELECT id INTO organization FROM organizations WHERE id = NEW.scope_id FOR KEY SHARE;
    ELSIF NEW.scope_kind = 'project' THEN
        SELECT organization_id INTO organization FROM projects WHERE id = NEW.scope_id FOR KEY SHARE;
    END IF;
    IF NEW.scope_kind <> 'system' AND NOT FOUND THEN
        RAISE EXCEPTION 'grant scope %:% does not exist', NEW.scope_kind, NEW.scope_id
            USING ERRCODE = 'foreign_key_violation';
    END IF;

    PERFORM check_role_binding(NEW.role_id, NEW.principal_kind, NEW.scope_kind, organization);
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE FUNCTION check_invite_role() RETURNS TRIGGER AS $$
BEGIN
    IF NEW.role_name IS NOT NULL THEN
        PERFORM check_role_binding(NEW.role_name, 'user', 'organization', NEW.organization_id);
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER organization_invites_check_role BEFORE INSERT ON organization_invites
    FOR EACH ROW EXECUTE FUNCTION check_invite_role();

DELETE FROM grants g WHERE NOT EXISTS (SELECT 1 FROM roles r WHERE r.id = g.role_id);
DELETE FROM grants g
USING roles r
WHERE r.id = g.role_id
  AND r.owner_org_id IS NOT NULL
  AND r.owner_org_id IS DISTINCT FROM CASE g.scope_kind
        WHEN 'organization' THEN g.scope_id
        WHEN 'project' THEN (SELECT p.organization_id FROM projects p WHERE p.id = g.scope_id)
      END;
DELETE FROM organization_invites i
WHERE i.role_name IS NOT NULL
  AND NOT EXISTS (SELECT 1 FROM roles r WHERE r.id = i.role_name);

-- CASCADE, not RESTRICT: on an organization delete the foreign key triggers run
-- before organizations_delete_grants, while grants of its roles still exist.
-- DeleteRole refuses a role that is still granted or offered.
ALTER TABLE grants
    ADD CONSTRAINT grants_role_id_fkey FOREIGN KEY (role_id) REFERENCES roles (id) ON DELETE CASCADE;
CREATE INDEX grants_role_idx ON grants (role_id);
ALTER TABLE organization_invites
    ADD CONSTRAINT organization_invites_role_name_fkey
        FOREIGN KEY (role_name) REFERENCES roles (id) ON DELETE CASCADE;
CREATE INDEX organization_invites_role_idx ON organization_invites (role_name);
