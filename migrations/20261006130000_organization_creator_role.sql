-- Lets a user create organizations, and nothing else. A system administrator
-- grants it at the system scope; the sign-up of an edition may grant it to each
-- new account. The creator of an organization becomes its admin.
INSERT INTO roles (id, key, name, description, scope_kind, kind, builtin) VALUES
    ('organization-creator', 'organization-creator', 'Organization Creator',
     'Create new organizations, and nothing else.', 'system', 'member', TRUE)
ON CONFLICT DO NOTHING;

INSERT INTO role_permissions (role_id, permission)
SELECT 'organization-creator', 'createOrganization'
WHERE EXISTS (SELECT 1 FROM roles WHERE id = 'organization-creator' AND builtin)
ON CONFLICT DO NOTHING;
