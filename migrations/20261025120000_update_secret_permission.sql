INSERT INTO role_permissions (role_id, permission)
SELECT role_id, 'updateSecret' FROM role_permissions WHERE permission = 'deleteSecret'
ON CONFLICT DO NOTHING;
