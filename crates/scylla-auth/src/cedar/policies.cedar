// scylla — Cedar authorization policies (V1). Default-deny; only the explicit
// permits below grant access. Compiled into the binary via include_str! and
// validated against schema.cedarschema at startup.

// ── System admin ────────────────────────────────────────────────────────
// There is no static admin policy: `system-admin` is now a GRANT on the System
// scope (the tenancy root), linked as a role-template instance like any other
// grant. Because every resource is `in` System, that single grant confers full
// control over everything. See `crates/scylla-auth/src/authz/grant/mod.rs` (SYSTEM scope).

// ── Internal services ──────────────────────────────────────────────────
// In-process subsystems (recorder, bootstrap, the server drivers) bypass user
// RBAC. Explicit, auditable replacement for the old silent recorder bypass.
// Tighten later to per-service action allowlists once service identities are
// on the wire.
@id("service")
permit (
    principal,
    action,
    resource
) when {
    principal is Scylla::Service
};

// ── Self access (ABAC) ─────────────────────────────────────────────────
// A user owns its own User entity: it may read and update its own profile and
// list the orgs/projects it belongs to. `deleteUser` is intentionally excluded
// — self-deletion is destructive and stays an admin / dedicated-flow action.
// Without this, only global admins could fetch their own org/project list —
// non-admins were denied listUserOrganizations even for themselves.
@id("self")
permit (
    principal,
    action in [
        Scylla::Action::"readUser",
        Scylla::Action::"updateUser",
        Scylla::Action::"listUserOrganizations",
        Scylla::Action::"listUserProjects"
    ],
    resource
) when {
    principal is Scylla::User && principal == resource
};

// ── No membership policies ─────────────────────────────────────────────
// There used to be an `org-member` and a `project-member` policy here, granting
// rights for merely belonging somewhere. Both are gone: every capability now
// comes from a grant, so what belonging used to confer is the
// `organization-member` builtin role, editable by the tenant like any other.
