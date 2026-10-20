-- The Cedar policy set is built from roles, role_permissions and grants. Every
-- statement that changes them, cascades included, bumps this version in the
-- same transaction, and a control plane rebuilds its set when the version it
-- reads differs from the one it built.
CREATE TABLE authz_version (
    id      BOOLEAN PRIMARY KEY DEFAULT TRUE CHECK (id),
    version BIGINT NOT NULL
);
INSERT INTO authz_version (version) VALUES (0);

CREATE FUNCTION bump_authz_version() RETURNS TRIGGER AS $$
BEGIN
    UPDATE authz_version SET version = version + 1;
    RETURN NULL;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER grants_bump_authz_version
    AFTER INSERT OR UPDATE OR DELETE OR TRUNCATE ON grants
    FOR EACH STATEMENT EXECUTE FUNCTION bump_authz_version();
CREATE TRIGGER roles_bump_authz_version
    AFTER INSERT OR UPDATE OR DELETE OR TRUNCATE ON roles
    FOR EACH STATEMENT EXECUTE FUNCTION bump_authz_version();
CREATE TRIGGER role_permissions_bump_authz_version
    AFTER INSERT OR UPDATE OR DELETE OR TRUNCATE ON role_permissions
    FOR EACH STATEMENT EXECUTE FUNCTION bump_authz_version();
