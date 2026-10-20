-- The organization viewer reads every project of its organization, so it holds
-- every read that a project viewer holds on one project.
INSERT INTO role_permissions (role_id, permission)
SELECT 'organization-viewer', 'listProjectMembers'
WHERE EXISTS (SELECT 1 FROM roles WHERE id = 'organization-viewer')
ON CONFLICT DO NOTHING;
