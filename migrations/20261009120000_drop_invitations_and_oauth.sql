-- The Community Edition has no invitations and no OAuth sign-in. The
-- Enterprise edition adds them back, with its own tables.

DROP TRIGGER organization_invites_check_role ON organization_invites;
DROP FUNCTION check_invite_role();

CREATE OR REPLACE FUNCTION check_role_change() RETURNS TRIGGER AS $$
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
    ) THEN
        RAISE EXCEPTION 'role % is given where its new kind, scope or owner does not allow it', NEW.id
            USING ERRCODE = 'foreign_key_violation';
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

DROP TABLE organization_invites;
DROP TABLE user_oauth_identities;

-- The permission no longer exists. A role keeps its other permissions. The
-- audit_log rows of old checks keep the key and the resource kind 'invitation':
-- they are history, and no code reads them.
DELETE FROM role_permissions WHERE permission = 'manageInvitations';
