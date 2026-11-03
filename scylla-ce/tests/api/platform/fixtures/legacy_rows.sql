-- The rows of the old product in the import test. The test sets a real hash after the load.
INSERT INTO users (id, username, password_hash, is_active, email, display_name, created_at, updated_at, version) VALUES
 ('01hzzzzzzzzzzzzzzzzzzzzza1', 'alice', '$HASH$', true, 'Alice@Example.com', 'Alice', now(), now(), 1),
 ('01hzzzzzzzzzzzzzzzzzzzzzb2', 'bob', '$HASH$', false, 'bob@example.com', NULL, now(), now(), 1),
 ('01hzzzzzzzzzzzzzzzzzzzzzc3', 'carl', '$HASH$', true, NULL, 'Carl', now(), now(), 1);
INSERT INTO organizations (id, name, description, is_active, created_at, updated_at, version) VALUES
 ('01hzzzzzzzzzzzzzzzzzzzzzn1', 'Acme', NULL, true, now(), now(), 1),
 ('01hzzzzzzzzzzzzzzzzzzzzzn2', 'Orphan', NULL, true, now(), now(), 1);
INSERT INTO projects (id, name, description, organization_id, is_active, created_at, updated_at, version) VALUES
 ('01hzzzzzzzzzzzzzzzzzzzzzp1', 'Web', 'The site', '01hzzzzzzzzzzzzzzzzzzzzzn1', true, now(), now(), 1);
INSERT INTO grants (id, principal_kind, principal_id, role_id, scope_kind, scope_id, created_at) VALUES
 ('01hzzzzzzzzzzzzzzzzzzzzzg1', 'user', '01hzzzzzzzzzzzzzzzzzzzzza1', 'system-admin', 'system', 'system', now()),
 ('01hzzzzzzzzzzzzzzzzzzzzzg2', 'user', '01hzzzzzzzzzzzzzzzzzzzzza1', 'organization-admin', 'organization', '01hzzzzzzzzzzzzzzzzzzzzzn1', now()),
 ('01hzzzzzzzzzzzzzzzzzzzzzg3', 'user', '01hzzzzzzzzzzzzzzzzzzzzzb2', 'organization-viewer', 'organization', '01hzzzzzzzzzzzzzzzzzzzzzn1', now()),
 ('01hzzzzzzzzzzzzzzzzzzzzzg4', 'user', '01hzzzzzzzzzzzzzzzzzzzzzb2', 'project-developer', 'project', '01hzzzzzzzzzzzzzzzzzzzzzp1', now());
INSERT INTO project_secrets (id, project_id, name, description, encrypted_value, created_at, updated_at) VALUES
 ('01hzzzzzzzzzzzzzzzzzzzzzs1', '01hzzzzzzzzzzzzzzzzzzzzzp1', 'API_TOKEN', '', '\x00'::bytea, now(), now());
INSERT INTO pipelines (id, project_id, name, nodes, created_at, updated_at, version) VALUES
 ('01hzzzzzzzzzzzzzzzzzzzzzm1', '01hzzzzzzzzzzzzzzzzzzzzzp1', 'ci',
  '[{"id":"build","deps":[],"step":{"kind":"exec","command":"make","args":["all"]},"env":[],"retries":1,"timeout_seconds":120},
    {"id":"deploy","deps":["build"],"step":{"kind":"script","script":"echo $TOKEN","shell":"bash"},"env":[{"key":"TOKEN","source":{"secret":"API_TOKEN"}}],"working_dir":"site"}]',
  now(), now(), 1),
 ('01hzzzzzzzzzzzzzzzzzzzzzm2', '01hzzzzzzzzzzzzzzzzzzzzzp1', 'loop',
  '[{"id":"a","deps":["b"],"step":{"kind":"exec","command":"true","args":[]}},{"id":"b","deps":["a"],"step":{"kind":"exec","command":"true","args":[]}}]',
  now(), now(), 1);
