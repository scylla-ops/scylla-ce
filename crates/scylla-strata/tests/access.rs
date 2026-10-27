//! Tests the access rules of the `#[authorize]` slot of `scylla_strata::Access`: a small module
//! of use cases, one for each kind of access, and a journal of its hooks.

#![allow(clippy::unused_async_trait_impl)]

use async_trait::async_trait;
use scylla_auth::authz::PermissionService;
use scylla_core::domain::access::Access as Rule;
use scylla_core::grpc::mappers::domain_error_to_status;
use scylla_core::test_support::stubs::ScopesByAgent;
use scylla_domain::domain::caller::{CallerContext, ServiceIdentity};
use scylla_domain::domain::errors::{DomainError, DomainResult};
use scylla_domain::domain::ids::{AppId, OrganizationId, ProjectId, UserId};
use scylla_domain::domain::permission::Permission;
use scylla_strata::{AccessModel, CallerOf, CedarPermissions, Refusal, ScyllaCtx};
use std::sync::{Arc, Mutex};
use strata::{Contextual, Edition};
use tonic::Code;

#[derive(Default)]
struct Asked {
    refused: Vec<Permission>,
    checks: Mutex<Vec<Permission>>,
}

impl Asked {
    fn checks(&self) -> Vec<Permission> {
        std::mem::take(&mut *self.checks.lock().unwrap())
    }
}

#[async_trait]
impl PermissionService for Asked {
    async fn check(&self, _: &CallerContext, permission: Permission) -> DomainResult<()> {
        self.checks.lock().unwrap().push(permission.clone());
        if self.refused.contains(&permission) {
            return Err(DomainError::forbidden(format!(
                "{} is not granted",
                permission.key()
            )));
        }
        Ok(())
    }
}

#[strata::module(section = "notes")]
pub trait Notes: scylla_strata::Access {
    #[component]
    type Trail = Mutex<Vec<String>>;

    fn note(&self, line: String) {
        self.trail().lock().unwrap().push(line);
    }

    #[usecase(read, access = Rule::Public)]
    async fn count_notes(&self) -> DomainResult<usize> {
        self.note("core count_notes".to_owned());
        Ok(1)
    }

    #[usecase(read, access = Rule::Authenticated)]
    async fn who_am_i(&self) -> DomainResult<CallerContext> {
        self.note("core who_am_i".to_owned());
        Ok(self.ctx().caller().clone())
    }

    #[usecase(write, access = Rule::Requires(Permission::CreateProject(org.clone())))]
    async fn create_note(&self, org: OrganizationId) -> DomainResult<()> {
        self.note(format!("core create_note {org}"));
        Ok(())
    }

    #[usecase(write, access = Rule::RequiresAll(vec![
        Permission::UpdateProject(id.clone()),
        Permission::CreateProject(org.clone()),
    ]))]
    async fn move_note(&self, id: ProjectId, org: OrganizationId) -> DomainResult<()> {
        self.note(format!("core move_note {id} {org}"));
        Ok(())
    }
}

#[strata::module(section = "journal")]
pub trait Journal: Notes {
    #[before(Notes::*)]
    async fn open(&self, usecase: &'static str) -> DomainResult<()> {
        self.note(format!("before {usecase}"));
        Ok(())
    }

    #[after(Notes::*)]
    async fn journal(&self, usecase: &'static str, outcome: Result<(), &DomainError>) {
        let outcome = outcome.map_or_else(ToString::to_string, |()| "ok".to_owned());
        self.note(format!("after {usecase}: {outcome}"));
    }
}

#[strata::edition(modules(scylla_strata::Access, Notes, Journal))]
pub struct NotesLab;

#[strata::edition(extends = NotesLab, overrides(scylla_strata::Access))]
pub struct TestNotes;

#[strata::overrides]
impl scylla_strata::Access for TestNotes {
    type Permissions = CedarPermissions;
}

async fn notes(refused: Vec<Permission>) -> (TestNotes, Arc<Asked>) {
    let asked = Arc::new(Asked {
        refused,
        ..Asked::default()
    });
    let app = TestNotes::builder(&strata::Config::new())
        .provide(AccessModel {
            permissions: asked.clone(),
            visibility: Arc::new(ScopesByAgent::everything()),
        })
        .build()
        .await
        .unwrap();
    (app, asked)
}

fn trail(app: &TestNotes) -> Vec<String> {
    std::mem::take(&mut *app.trail().lock().unwrap())
}

fn on(caller: CallerContext) -> ScyllaCtx {
    ScyllaCtx {
        caller,
        ..ScyllaCtx::default()
    }
}

fn bob() -> CallerContext {
    CallerContext::User(UserId::new("bob"))
}

fn acme() -> OrganizationId {
    OrganizationId::new("acme")
}

#[tokio::test]
async fn a_public_use_case_runs_without_the_permission_service() {
    let (app, asked) = notes(Vec::new()).await;
    let bad_token = ScyllaCtx {
        refused: Some(Refusal::Unauthenticated(
            "Invalid or expired token".to_owned(),
        )),
        ..ScyllaCtx::default()
    };

    for ctx in [ScyllaCtx::default(), bad_token] {
        assert_eq!(NotesRpc::count_notes(&app.scoped(&ctx)).await.unwrap(), 1);
        assert_eq!(
            trail(&app),
            [
                "before count_notes",
                "core count_notes",
                "after count_notes: ok"
            ]
        );
    }
    assert_eq!(asked.checks(), Vec::<Permission>::new());
}

#[tokio::test]
async fn an_authenticated_use_case_refuses_anonymous_only() {
    let (app, asked) = notes(Vec::new()).await;

    let err = NotesRpc::who_am_i(&app.scoped(&ScyllaCtx::default()))
        .await
        .unwrap_err();
    assert!(
        matches!(&err, DomainError::Forbidden(m) if m == "Anonymous caller is not permitted"),
        "{err}"
    );
    assert_eq!(
        trail(&app),
        ["after who_am_i: Forbidden: Anonymous caller is not permitted"]
    );

    let system = CallerContext::Service(ServiceIdentity::recorder());
    let app_caller = CallerContext::App(AppId::new("ci"));
    for caller in [bob(), app_caller, system] {
        let me = NotesRpc::who_am_i(&app.scoped(&on(caller.clone())))
            .await
            .unwrap();
        assert_eq!(me, caller);
        assert_eq!(
            trail(&app),
            ["before who_am_i", "core who_am_i", "after who_am_i: ok"]
        );
    }
    assert_eq!(asked.checks(), Vec::<Permission>::new());
}

#[tokio::test]
async fn every_permission_of_an_action_must_pass_and_the_first_refusal_stops() {
    let note = ProjectId::new("note-1");
    let globex = OrganizationId::new("globex");
    let update = Permission::UpdateProject(note.clone());
    let create_in_globex = Permission::CreateProject(globex.clone());

    let (app, asked) = notes(vec![update.clone(), create_in_globex.clone()]).await;
    let err = NotesRpc::move_note(&app.scoped(&on(bob())), note.clone(), globex.clone())
        .await
        .unwrap_err();
    let status = domain_error_to_status(err);
    assert_eq!(status.code(), Code::PermissionDenied);
    assert_eq!(status.message(), format!("{} is not granted", update.key()));
    assert_eq!(asked.checks(), vec![update.clone()]);
    assert!(!trail(&app).iter().any(|line| line.starts_with("core")));

    let (app, asked) = notes(vec![create_in_globex.clone()]).await;
    let err = NotesRpc::move_note(&app.scoped(&on(bob())), note.clone(), globex)
        .await
        .unwrap_err();
    let status = domain_error_to_status(err);
    assert_eq!(status.code(), Code::PermissionDenied);
    assert_eq!(
        status.message(),
        format!("{} is not granted", create_in_globex.key())
    );
    assert_eq!(asked.checks(), [update.clone(), create_in_globex]);
    assert!(!trail(&app).iter().any(|line| line.starts_with("core")));

    NotesRpc::move_note(&app.scoped(&on(bob())), note.clone(), acme())
        .await
        .unwrap();
    assert_eq!(asked.checks(), [update, Permission::CreateProject(acme())]);
    assert_eq!(
        trail(&app),
        [
            "before move_note".to_owned(),
            format!("core move_note {note} acme"),
            "after move_note: ok".to_owned()
        ]
    );
}

#[tokio::test]
async fn a_refused_caller_is_journaled_at_authorize_only() {
    let create = Permission::CreateProject(acme());
    let (app, asked) = notes(vec![create.clone()]).await;

    let err = NotesRpc::create_note(&app.scoped(&on(bob())), acme())
        .await
        .unwrap_err();

    assert!(matches!(err, DomainError::Forbidden(_)), "{err}");
    assert_eq!(
        trail(&app),
        [format!(
            "after create_note: Forbidden: {} is not granted",
            create.key()
        )]
    );
    assert_eq!(asked.checks(), [create]);
}
