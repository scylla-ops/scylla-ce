//! Tests the Triggers module of Scylla, written with strata: the CE application tests of the slice
//! (the trigger actions, the fire, the cron passes, the webhook ingress) on stub ports, and the
//! gRPC adapter over a real tonic server.

use async_trait::async_trait;
use scylla_auth::authz::{Grant, PermissionService};
use scylla_core::application::trigger::{
    ClaimDueTriggers, CreateTrigger, CreatedTrigger, CronSchedule, DEFAULT_SIGNATURE_HEADER,
    DeleteTrigger, FireTriggerNow, GetTrigger, IngestOutcome, IngestWebhook, ListPipelineTriggers,
    NextFire, RecordTriggerFire, ResolveTriggerRun, ScheduleCronTriggers, SetTriggerEnabled,
    TriggerDeliveryRepository, TriggerRepository, UpdateTrigger,
};
use scylla_core::application::{
    AppRepository, JobRepository, PipelineRepository, PrincipalAccess, ProjectRepository,
    SecretCipher, SecretResolver,
};
use scylla_core::test_support::authz::{DenyingPermissionService, RecordingPermissionService};
use scylla_core::test_support::pipelines::PipelineBuilder;
use scylla_core::test_support::projects::ProjectBuilder;
use scylla_core::test_support::stubs::{
    EchoResolver, OnePipeline, OneProject, ScopesByAgent, StubJobs, StubRegistry, StubTails, alice,
    dispatcher,
};
use scylla_domain::domain::agent::Agent;
use scylla_domain::domain::app::{App, AppCredential, AppKind, AppName, TRIGGER_RUNNER_APP_NAME};
use scylla_domain::domain::caller::{CallerContext, ServiceIdentity};
use scylla_domain::domain::clock;
use scylla_domain::domain::errors::{DomainError, DomainResult};
use scylla_domain::domain::ids::{AppId, OrganizationId, PipelineId, ProjectId, TriggerId};
use scylla_domain::domain::job::{Job, JobOrigin};
use scylla_domain::domain::permission::Permission;
use scylla_domain::domain::pipeline::EnvKey;
use scylla_domain::domain::trigger::{
    CronSpec, FireObservation, Trigger, TriggerInput, TriggerKind, TriggerName, TriggerSource,
    WebhookSpec,
};
use scylla_proto::common::v1::TriggerId as PbTriggerId;
use scylla_proto::trigger::v1::{FireTriggerNowRequest, GetTriggerRequest};
use scylla_strata::grpc::{TriggerServiceClient, TriggerServiceServer};
use scylla_strata::{
    AccessModel, CedarPermissions, JobDispatch, ScyllaCtx, TriggerWebhookBase, Triggers,
    TriggersRpc,
};
use sqlx::types::chrono::{DateTime, Utc};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use strata::{Contextual, Edition};
use strata_tonic::Grpc;
use tonic::Code;
use tonic::transport::server::TcpIncoming;
use tonic::transport::{Channel, Server};

type Duration = <DateTime<Utc> as std::ops::Sub>::Output;

#[strata::edition(modules(
    scylla_strata::Access,
    scylla_strata::Pipelines,
    scylla_strata::Triggers
))]
pub struct TestBase;

#[strata::edition(extends = TestBase, overrides(scylla_strata::Access))]
pub struct TestCe;

#[strata::overrides]
impl scylla_strata::Access for TestCe {
    type Permissions = CedarPermissions;
}

/// The last fire is kept apart from the rows, as the store keeps it apart from the edits.
#[derive(Default)]
struct StubTriggers {
    rows: Mutex<HashMap<TriggerId, (Trigger, Option<Vec<u8>>)>>,
    fires: Mutex<HashMap<TriggerId, FireObservation>>,
}

#[async_trait]
impl TriggerRepository for StubTriggers {
    async fn create(&self, trigger: &Trigger, enc: Option<&[u8]>) -> DomainResult<Trigger> {
        self.rows.lock().unwrap().insert(
            trigger.id().clone(),
            (trigger.clone(), enc.map(<[u8]>::to_vec)),
        );
        Ok(trigger.clone())
    }
    async fn find_by_id(&self, id: &TriggerId) -> DomainResult<Trigger> {
        self.rows
            .lock()
            .unwrap()
            .get(id)
            .map(|(t, _)| t.clone())
            .ok_or_else(|| DomainError::not_found("Trigger", id))
    }
    async fn webhook_secret(&self, id: &TriggerId) -> DomainResult<Option<Vec<u8>>> {
        Ok(self
            .rows
            .lock()
            .unwrap()
            .get(id)
            .and_then(|(_, enc)| enc.clone()))
    }
    async fn update(&self, trigger: &Trigger) -> DomainResult<Trigger> {
        let mut rows = self.rows.lock().unwrap();
        let (stored, _) = rows
            .get_mut(trigger.id())
            .ok_or_else(|| DomainError::not_found("Trigger", trigger.id()))?;
        *stored = trigger.clone();
        Ok(trigger.clone())
    }
    async fn delete(&self, trigger: &Trigger) -> DomainResult<()> {
        self.rows.lock().unwrap().remove(trigger.id());
        Ok(())
    }
    async fn list_by_pipeline(&self, pipeline_id: &PipelineId) -> DomainResult<Vec<Trigger>> {
        Ok(self
            .rows
            .lock()
            .unwrap()
            .values()
            .filter(|(t, _)| t.pipeline_id() == pipeline_id)
            .map(|(t, _)| t.clone())
            .collect())
    }
    async fn record_fire(&self, id: &TriggerId, observation: &FireObservation) -> DomainResult<()> {
        if self.rows.lock().unwrap().contains_key(id) {
            self.fires
                .lock()
                .unwrap()
                .insert(id.clone(), observation.clone());
        }
        Ok(())
    }
    async fn seed_cron(&self, compute_next: &NextFire<'_>) -> DomainResult<Vec<Trigger>> {
        self.advance(compute_next, |t| t.next_fire_at().is_none())
    }
    async fn claim_due_cron(
        &self,
        now: DateTime<Utc>,
        _: i64,
        compute_next: &NextFire<'_>,
    ) -> DomainResult<Vec<Trigger>> {
        self.advance(compute_next, |t| {
            t.next_fire_at().is_some_and(|at| at <= now)
        })
    }
}

impl StubTriggers {
    fn advance(
        &self,
        compute_next: &NextFire<'_>,
        selected: impl Fn(&Trigger) -> bool,
    ) -> DomainResult<Vec<Trigger>> {
        let mut advanced = Vec::new();
        for (trigger, _) in self.rows.lock().unwrap().values_mut() {
            let cron = trigger.source().kind() == TriggerKind::Cron;
            if !(trigger.is_enabled() && cron && selected(trigger)) {
                continue;
            }
            let read = trigger.clone();
            if let Ok(next) = compute_next(&read) {
                trigger.set_next_fire_at(Some(next));
                advanced.push(read);
            }
        }
        Ok(advanced)
    }
}

#[derive(Default)]
struct StubApps {
    apps: Mutex<Vec<App>>,
    grants: Mutex<Vec<Grant>>,
}

#[async_trait]
impl AppRepository for StubApps {
    async fn create_app(&self, _: &App, _: &AppCredential) -> DomainResult<()> {
        unreachable!("the runner app is provisioned with its grant")
    }
    async fn provision_agent(
        &self,
        _: &App,
        _: &AppCredential,
        _: &Agent,
        _: &Grant,
    ) -> DomainResult<()> {
        unreachable!("no agent in a trigger action")
    }
    async fn provision(&self, app: &App, grant: &Grant) -> DomainResult<()> {
        let mut apps = self.apps.lock().unwrap();
        if apps
            .iter()
            .any(|a| a.organization_id() == app.organization_id() && a.name() == app.name())
        {
            return Err(DomainError::conflict("App name already exists"));
        }
        apps.push(app.clone());
        self.grants.lock().unwrap().push(grant.clone());
        Ok(())
    }
    async fn find_by_id(&self, id: &AppId) -> DomainResult<App> {
        Err(DomainError::not_found("App", id))
    }
    async fn find_trigger_runner(
        &self,
        organization_id: &OrganizationId,
    ) -> DomainResult<Option<AppId>> {
        Ok(self
            .apps
            .lock()
            .unwrap()
            .iter()
            .find(|a| a.organization_id() == organization_id && a.is_trigger_runner())
            .map(|a| a.id().clone()))
    }
    async fn list_by_organization(
        &self,
        organization_id: &OrganizationId,
    ) -> DomainResult<Vec<App>> {
        Ok(self
            .apps
            .lock()
            .unwrap()
            .iter()
            .filter(|a| a.organization_id() == organization_id)
            .cloned()
            .collect())
    }
    async fn set_active(&self, _: &AppId, _: bool) -> DomainResult<()> {
        unreachable!("no app update in a trigger action")
    }
    async fn delete(&self, _: &AppId) -> DomainResult<()> {
        unreachable!("no app delete in a trigger action")
    }
    async fn list_access(&self, _: &AppId) -> DomainResult<Vec<PrincipalAccess>> {
        unreachable!("no app access in a trigger action")
    }
}

struct StubCipher;

impl SecretCipher for StubCipher {
    fn encrypt(&self, plaintext: &str) -> DomainResult<Vec<u8>> {
        Ok(plaintext.as_bytes().iter().rev().copied().collect())
    }
    fn decrypt(&self, ciphertext: &[u8]) -> DomainResult<String> {
        Ok(String::from_utf8(ciphertext.iter().rev().copied().collect()).unwrap())
    }
}

struct StubSchedule;

impl CronSchedule for StubSchedule {
    fn next_after(&self, _: &str, after: DateTime<Utc>) -> DomainResult<DateTime<Utc>> {
        Ok(after + Duration::hours(1))
    }
}

/// Grants everything but `runPipeline`: a trigger manager who may not run the pipeline.
#[derive(Default)]
struct NoRunPermissionService {
    inner: RecordingPermissionService,
}

#[async_trait]
impl PermissionService for NoRunPermissionService {
    async fn check(&self, caller: &CallerContext, permission: Permission) -> DomainResult<()> {
        let refused = permission.key() == "runPipeline";
        self.inner.check(caller, permission).await?;
        if refused {
            return Err(DomainError::forbidden("no run rights"));
        }
        Ok(())
    }
}

/// Grants each permission to each caller except `user:eve`.
struct AllButEve;

#[async_trait]
impl PermissionService for AllButEve {
    async fn check(&self, caller: &CallerContext, permission: Permission) -> DomainResult<()> {
        match caller {
            CallerContext::User(id) if id.as_str() == "eve" => Err(DomainError::forbidden(
                format!("{} is not granted", permission.key()),
            )),
            _ => Ok(()),
        }
    }
}

struct Lab {
    app: TestCe,
    triggers: Arc<StubTriggers>,
    apps: Arc<StubApps>,
    jobs: Arc<StubJobs>,
    deliveries: Arc<StubDeliveries>,
}

fn ctx(caller: CallerContext) -> ScyllaCtx {
    ScyllaCtx {
        caller,
        ..ScyllaCtx::default()
    }
}

fn firer() -> CallerContext {
    CallerContext::Service(ServiceIdentity::trigger_firer())
}

fn cron_scheduler() -> CallerContext {
    CallerContext::Service(ServiceIdentity::cron_scheduler())
}

impl Lab {
    async fn create(&self, source: TriggerSource) -> DomainResult<CreatedTrigger> {
        TriggersRpc::create_trigger(&self.app.scoped(&ctx(alice())), create(source)).await
    }

    async fn update(&self, id: &TriggerId) -> DomainResult<Trigger> {
        let cmd = UpdateTrigger {
            id: id.clone(),
            name: TriggerName::new("hourly").unwrap(),
            source: cron(),
            inputs: Vec::new(),
        };
        TriggersRpc::update_trigger(&self.app.scoped(&ctx(alice())), cmd).await
    }

    async fn delete(&self, id: &TriggerId) -> DomainResult<()> {
        let cmd = DeleteTrigger { id: id.clone() };
        TriggersRpc::delete_trigger(&self.app.scoped(&ctx(alice())), cmd).await
    }

    async fn enable(&self, id: &TriggerId, enabled: bool) -> DomainResult<Trigger> {
        let cmd = SetTriggerEnabled {
            id: id.clone(),
            enabled,
        };
        TriggersRpc::set_trigger_enabled(&self.app.scoped(&ctx(alice())), cmd).await
    }

    async fn fire(&self, id: &TriggerId) -> DomainResult<Job> {
        Triggers::fire(&self.app, id).await
    }

    fn stored(&self, id: &TriggerId) -> Trigger {
        self.triggers.rows.lock().unwrap()[id].0.clone()
    }

    fn last_fire(&self, id: &TriggerId) -> Option<String> {
        self.triggers
            .fires
            .lock()
            .unwrap()
            .get(id)
            .map(|o| o.status.clone())
    }

    fn seed(&self) -> Trigger {
        let trigger = Trigger::create(
            pipeline_id(),
            TriggerName::new("nightly").unwrap(),
            cron(),
            Vec::new(),
        )
        .unwrap();
        self.triggers
            .rows
            .lock()
            .unwrap()
            .insert(trigger.id().clone(), (trigger.clone(), None));
        trigger
    }

    fn due(&self) -> Trigger {
        let mut trigger = self.seed();
        trigger.set_next_fire_at(Some(clock::now() - Duration::minutes(1)));
        self.triggers
            .rows
            .lock()
            .unwrap()
            .insert(trigger.id().clone(), (trigger.clone(), None));
        trigger
    }

    fn runner(&self) -> CallerContext {
        CallerContext::App(self.apps.apps.lock().unwrap()[0].id().clone())
    }

    /// The pass of the CE cron scheduler, on the use cases of the module: seed, claim, then one
    /// fire for each claimed trigger. A failed fire does not stop the pass.
    async fn tick(&self) -> (usize, Vec<TriggerId>) {
        let scheduler = ctx(cron_scheduler());
        let view = self.app.scoped(&scheduler);
        TriggersRpc::schedule_cron_triggers(&view, ScheduleCronTriggers)
            .await
            .unwrap();
        let claim = ClaimDueTriggers {
            now: clock::now(),
            limit: 100,
        };
        let due = TriggersRpc::claim_due_triggers(&view, claim).await.unwrap();
        let mut fired = 0;
        let ids: Vec<TriggerId> = due.iter().map(|t| t.id().clone()).collect();
        for trigger in due {
            if self.fire(trigger.id()).await.is_ok() {
                fired += 1;
            }
        }
        (fired, ids)
    }
}

async fn lab(permissions: Arc<dyn PermissionService>) -> Lab {
    let project = ProjectBuilder::for_org_id(organization_id(), "rocket")
        .id(ProjectId::new("proj-1"))
        .build();
    let pipeline = PipelineBuilder::for_project_id(project.id().clone())
        .id(pipeline_id())
        .build();
    let triggers = Arc::new(StubTriggers::default());
    let apps = Arc::new(StubApps::default());
    let jobs = Arc::new(StubJobs::default());
    let deliveries = Arc::new(StubDeliveries::default());
    let dispatch = dispatcher(
        Arc::new(StubRegistry::default()),
        jobs.clone(),
        Arc::new(StubTails::default()),
    );
    let config = strata::Config::new();
    let app = TestCe::builder(&config)
        .provide(AccessModel {
            permissions,
            visibility: Arc::new(ScopesByAgent::everything()),
        })
        .provide(JobDispatch(dispatch))
        .provide(TriggerWebhookBase(Some("https://hooks.example.com".into())))
        .provide_arc::<dyn TriggerRepository>(triggers.clone())
        .provide_arc::<dyn PipelineRepository>(Arc::new(OnePipeline(pipeline)))
        .provide_arc::<dyn ProjectRepository>(Arc::new(OneProject(project)))
        .provide_arc::<dyn AppRepository>(apps.clone())
        .provide_arc::<dyn SecretCipher>(Arc::new(StubCipher))
        .provide_arc::<dyn CronSchedule>(Arc::new(StubSchedule))
        .provide_arc::<dyn JobRepository>(jobs.clone())
        .provide_arc::<dyn SecretResolver>(Arc::new(EchoResolver))
        .provide_arc::<dyn TriggerDeliveryRepository>(deliveries.clone())
        .build()
        .await
        .unwrap();
    Lab {
        app,
        triggers,
        apps,
        jobs,
        deliveries,
    }
}

fn organization_id() -> OrganizationId {
    OrganizationId::new("acme")
}

fn pipeline_id() -> PipelineId {
    PipelineId::new("pl-1")
}

fn cron() -> TriggerSource {
    TriggerSource::Cron(CronSpec::new("0 3 * * *").unwrap())
}

fn webhook() -> TriggerSource {
    TriggerSource::Webhook(WebhookSpec::new(None).unwrap())
}

fn create(source: TriggerSource) -> CreateTrigger {
    CreateTrigger {
        pipeline_id: pipeline_id(),
        name: TriggerName::new("nightly").unwrap(),
        source,
        inputs: Vec::new(),
    }
}

#[tokio::test]
async fn a_create_checks_manage_then_run_and_provisions_the_runner_app_once() {
    let permissions = Arc::new(RecordingPermissionService::new());
    let lab = lab(permissions.clone()).await;

    let CreatedTrigger {
        trigger,
        webhook_secret: secret,
    } = lab.create(cron()).await.unwrap();
    lab.create(cron()).await.unwrap();

    assert_eq!(
        permissions.permissions(),
        vec![
            Permission::ManageTriggers(pipeline_id()),
            Permission::RunPipeline(pipeline_id()),
            Permission::ManageTriggers(pipeline_id()),
            Permission::RunPipeline(pipeline_id()),
        ]
    );
    assert!(secret.is_none());
    assert!(trigger.next_fire_at().is_some());
    assert_eq!(lab.triggers.rows.lock().unwrap().len(), 2);
    let apps = lab.apps.apps.lock().unwrap();
    assert_eq!(apps.len(), 1);
    assert!(apps[0].is_trigger_runner());
    assert_eq!(apps[0].name().as_str(), TRIGGER_RUNNER_APP_NAME);
    assert_eq!(apps[0].organization_id(), &organization_id());
}

#[tokio::test]
async fn a_legacy_app_holding_the_runner_name_fails_the_create_with_a_conflict() {
    let lab = lab(Arc::new(RecordingPermissionService::new())).await;
    let legacy = App::from_persistence(
        AppId::new("legacy"),
        organization_id(),
        AppName::new(TRIGGER_RUNNER_APP_NAME).unwrap(),
        AppKind::Standard,
        true,
        clock::now(),
        clock::now(),
    );
    lab.apps.apps.lock().unwrap().push(legacy);

    let err = lab.create(cron()).await.err().unwrap();

    assert!(matches!(err, DomainError::Conflict(_)), "{err:?}");
    assert!(lab.triggers.rows.lock().unwrap().is_empty());
    assert_eq!(lab.apps.apps.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn a_webhook_create_answers_the_plaintext_once_and_stores_it_encrypted() {
    let lab = lab(Arc::new(RecordingPermissionService::new())).await;

    let CreatedTrigger {
        trigger,
        webhook_secret: secret,
    } = lab.create(webhook()).await.unwrap();

    let secret = secret.expect("a webhook trigger has a secret");
    assert_eq!(secret.len(), 64);
    assert!(trigger.next_fire_at().is_none());
    assert_eq!(
        lab.triggers.webhook_secret(trigger.id()).await.unwrap(),
        Some(StubCipher.encrypt(&secret).unwrap())
    );
}

#[tokio::test]
async fn a_denied_caller_writes_nothing() {
    let lab = lab(Arc::new(DenyingPermissionService::new())).await;

    let err = lab.create(cron()).await.err().unwrap();

    assert!(matches!(err, DomainError::Forbidden(_)));
    assert!(lab.triggers.rows.lock().unwrap().is_empty());
    assert!(lab.apps.apps.lock().unwrap().is_empty());
}

#[tokio::test]
async fn a_manager_without_run_rights_cannot_create_a_trigger() {
    let permissions = Arc::new(NoRunPermissionService::default());
    let lab = lab(permissions.clone()).await;

    let err = lab.create(cron()).await.err().unwrap();

    assert!(matches!(err, DomainError::Forbidden(_)));
    assert_eq!(
        permissions.inner.permissions(),
        vec![
            Permission::ManageTriggers(pipeline_id()),
            Permission::RunPipeline(pipeline_id()),
        ]
    );
    assert!(lab.triggers.rows.lock().unwrap().is_empty());
    assert!(lab.apps.apps.lock().unwrap().is_empty());
}

#[tokio::test]
async fn a_create_on_an_unknown_pipeline_is_not_found_and_writes_nothing() {
    let lab = lab(Arc::new(RecordingPermissionService::new())).await;
    let cmd = CreateTrigger {
        pipeline_id: PipelineId::new("ghost"),
        ..create(cron())
    };

    let err = TriggersRpc::create_trigger(&lab.app.scoped(&ctx(alice())), cmd)
        .await
        .err()
        .unwrap();

    assert!(matches!(err, DomainError::NotFound(_)));
    assert!(lab.triggers.rows.lock().unwrap().is_empty());
    assert!(lab.apps.apps.lock().unwrap().is_empty());
}

#[tokio::test]
async fn a_pipeline_list_checks_manage_triggers() {
    let permissions = Arc::new(RecordingPermissionService::new());
    let lab = lab(permissions.clone()).await;
    let seeded = lab.seed();

    let triggers = TriggersRpc::list_pipeline_triggers(
        &lab.app.scoped(&ctx(alice())),
        ListPipelineTriggers {
            pipeline_id: pipeline_id(),
        },
    )
    .await
    .unwrap();

    assert_eq!(
        permissions.permissions(),
        vec![Permission::ManageTriggers(pipeline_id())]
    );
    assert_eq!(triggers.len(), 1);
    assert_eq!(triggers[0].id(), seeded.id());
}

#[tokio::test]
async fn a_get_checks_manage_on_the_trigger() {
    let permissions = Arc::new(RecordingPermissionService::new());
    let lab = lab(permissions.clone()).await;
    let seeded = lab.seed();

    let trigger = TriggersRpc::get_trigger(
        &lab.app.scoped(&ctx(alice())),
        GetTrigger {
            id: seeded.id().clone(),
        },
    )
    .await
    .unwrap();

    assert_eq!(trigger.id(), seeded.id());
    assert_eq!(
        permissions.permissions(),
        vec![Permission::ManageTrigger(seeded.id().clone())]
    );
}

#[tokio::test]
async fn an_update_checks_manage_and_run_on_the_trigger() {
    let permissions = Arc::new(RecordingPermissionService::new());
    let lab = lab(permissions.clone()).await;
    let seeded = lab.seed();

    let updated = lab.update(seeded.id()).await.unwrap();

    assert_eq!(
        permissions.permissions(),
        vec![
            Permission::ManageTrigger(seeded.id().clone()),
            Permission::RunTriggerPipeline(seeded.id().clone()),
        ]
    );
    assert_eq!(updated.name().as_str(), "hourly");
    assert!(updated.next_fire_at().is_some());
}

#[tokio::test]
async fn a_manager_without_run_rights_cannot_update_a_trigger() {
    let lab = lab(Arc::new(NoRunPermissionService::default())).await;
    let seeded = lab.seed();

    let err = lab.update(seeded.id()).await.unwrap_err();

    assert!(matches!(err, DomainError::Forbidden(_)));
    assert_eq!(lab.stored(seeded.id()).name().as_str(), "nightly");
}

#[tokio::test]
async fn a_disable_then_a_delete_check_manage_on_the_trigger() {
    let permissions = Arc::new(RecordingPermissionService::new());
    let lab = lab(permissions.clone()).await;
    let seeded = lab.seed();

    let disabled = lab.enable(seeded.id(), false).await.unwrap();
    lab.delete(seeded.id()).await.unwrap();

    assert!(!disabled.is_enabled());
    assert_eq!(
        permissions.permissions(),
        vec![
            Permission::ManageTrigger(seeded.id().clone()),
            Permission::ManageTrigger(seeded.id().clone()),
        ]
    );
    assert!(lab.triggers.rows.lock().unwrap().is_empty());
}

#[tokio::test]
async fn a_denied_delete_never_reads_or_removes() {
    let lab = lab(Arc::new(DenyingPermissionService::new())).await;
    let seeded = lab.seed();

    let err = lab.delete(seeded.id()).await.unwrap_err();

    assert!(matches!(err, DomainError::Forbidden(_)));
    assert!(lab.triggers.rows.lock().unwrap().contains_key(seeded.id()));
}

#[tokio::test]
async fn an_allowed_action_on_an_unknown_trigger_is_not_found() {
    let lab = lab(Arc::new(RecordingPermissionService::new())).await;

    let err = lab.delete(&TriggerId::new("missing")).await.unwrap_err();

    assert!(matches!(err, DomainError::NotFound(_)));
}

#[tokio::test]
async fn a_tick_seeds_the_unscheduled_then_fires_every_due_trigger_with_no_permission_asked() {
    let permissions = Arc::new(RecordingPermissionService::new());
    let lab = lab(permissions.clone()).await;
    let created = lab.create(cron()).await.unwrap().trigger;
    lab.triggers
        .rows
        .lock()
        .unwrap()
        .get_mut(created.id())
        .unwrap()
        .0
        .set_next_fire_at(None);
    let fresh = lab.seed();
    let a = lab.due();
    let b = lab.due();
    let before = permissions.checks().len();

    let scheduler = ctx(cron_scheduler());
    let view = lab.app.scoped(&scheduler);
    let seeded = TriggersRpc::schedule_cron_triggers(&view, ScheduleCronTriggers)
        .await
        .unwrap();
    let claim = ClaimDueTriggers {
        now: clock::now(),
        limit: 100,
    };
    let due = TriggersRpc::claim_due_triggers(&view, claim).await.unwrap();
    assert_eq!(permissions.checks().len(), before, "no permission asked");
    for trigger in &due {
        lab.fire(trigger.id()).await.unwrap();
    }

    assert_eq!(seeded.len(), 2);
    assert!(lab.stored(fresh.id()).next_fire_at().is_some(), "seeded");
    let mut fired: Vec<TriggerId> = due.iter().map(|t| t.id().clone()).collect();
    fired.sort_by_key(ToString::to_string);
    let mut expected = vec![a.id().clone(), b.id().clone()];
    expected.sort_by_key(ToString::to_string);
    assert_eq!(fired, expected);
    assert!(
        lab.stored(a.id()).next_fire_at().unwrap() > clock::now(),
        "advanced"
    );
    assert_eq!(lab.jobs.rows().len(), 2);
}

#[tokio::test]
async fn the_cron_worker_seeds_then_fires_each_due_trigger_as_the_scheduler_service() {
    let lab = lab(Arc::new(RecordingPermissionService::new())).await;
    lab.create(cron()).await.unwrap();
    let fresh = lab.seed();
    let a = lab.due();
    let b = lab.due();

    assert_eq!(scylla_strata::fire_cron_triggers(&lab.app).await, 2);
    assert_eq!(scylla_strata::fire_cron_triggers(&lab.app).await, 0);

    assert!(lab.stored(fresh.id()).next_fire_at().is_some(), "seeded");
    for id in [a.id(), b.id()] {
        assert_eq!(lab.last_fire(id).as_deref(), Some("ok"));
    }
    assert_eq!(lab.jobs.rows().len(), 2);
}

#[tokio::test]
async fn one_fire_failure_does_not_abort_the_pass() {
    let lab = lab(Arc::new(RecordingPermissionService::new())).await;
    lab.due();
    lab.due();

    let (fired, claimed) = lab.tick().await;

    assert_eq!(claimed.len(), 2);
    assert_eq!(fired, 0, "no runner app: each fire fails");
    for id in &claimed {
        assert_eq!(lab.last_fire(id).as_deref(), Some("error"));
    }
}

#[tokio::test]
async fn a_webhook_trigger_is_never_seeded_as_cron() {
    let lab = lab(Arc::new(RecordingPermissionService::new())).await;
    let hook = lab.create(webhook()).await.unwrap().trigger;

    lab.tick().await;

    assert!(lab.stored(hook.id()).next_fire_at().is_none());
}

#[tokio::test]
async fn the_cron_passes_refuse_a_caller_that_is_not_a_service() {
    let lab = lab(Arc::new(RecordingPermissionService::new())).await;
    let fresh = lab.seed();
    let user = ctx(alice());
    let scheduler = ctx(cron_scheduler());

    let seed = TriggersRpc::schedule_cron_triggers(&lab.app.scoped(&user), ScheduleCronTriggers)
        .await
        .unwrap_err();
    let claim = ClaimDueTriggers {
        now: clock::now(),
        limit: 10,
    };
    let claim = TriggersRpc::claim_due_triggers(&lab.app.scoped(&user), claim)
        .await
        .unwrap_err();
    let scheduled =
        TriggersRpc::schedule_cron_triggers(&lab.app.scoped(&scheduler), ScheduleCronTriggers)
            .await
            .unwrap();

    assert!(matches!(seed, DomainError::Forbidden(_)));
    assert!(matches!(claim, DomainError::Forbidden(_)));
    assert_eq!(scheduled.len(), 1);
    assert_eq!(scheduled[0].id(), fresh.id());
}

#[tokio::test]
async fn a_fire_runs_as_the_runner_app_then_records_the_outcome_as_the_firer() {
    let permissions = Arc::new(RecordingPermissionService::new());
    let lab = lab(permissions.clone()).await;
    let trigger = lab.create(cron()).await.unwrap().trigger;
    let before = permissions.checks().len();

    let job = lab.fire(trigger.id()).await.unwrap();

    assert_eq!(
        permissions.checks()[before..],
        [
            (lab.runner(), Permission::RunPipeline(pipeline_id())),
            (firer(), Permission::ManageTrigger(trigger.id().clone())),
        ]
    );
    assert_eq!(
        job.origin(),
        &JobOrigin::Cron {
            trigger_id: trigger.id().clone()
        }
    );
    assert_eq!(lab.jobs.rows().len(), 1);
    assert_eq!(lab.last_fire(trigger.id()).as_deref(), Some("ok"));
}

#[tokio::test]
async fn a_disabled_trigger_does_not_fire_and_records_nothing() {
    let permissions = Arc::new(RecordingPermissionService::new());
    let lab = lab(permissions.clone()).await;
    let trigger = lab.create(cron()).await.unwrap().trigger;
    lab.enable(trigger.id(), false).await.unwrap();
    let before = permissions.checks().len();

    let err = lab.fire(trigger.id()).await.unwrap_err();

    assert!(matches!(err, DomainError::BusinessRule(_)));
    assert_eq!(permissions.checks().len(), before);
    assert!(lab.jobs.rows().is_empty());
    assert_eq!(lab.last_fire(trigger.id()), None);
}

#[tokio::test]
async fn a_failed_run_is_recorded_as_an_error() {
    let lab = lab(Arc::new(RecordingPermissionService::new())).await;
    let trigger = lab.seed();

    let err = lab.fire(trigger.id()).await.unwrap_err();

    assert!(matches!(err, DomainError::Internal(_)), "no runner app");
    assert_eq!(lab.last_fire(trigger.id()).as_deref(), Some("error"));
}

#[tokio::test]
async fn a_fire_record_does_not_undo_a_disable_that_committed_during_the_run() {
    let lab = lab(Arc::new(RecordingPermissionService::new())).await;
    let id = lab.create(cron()).await.unwrap().trigger.id().clone();
    let service = ctx(firer());
    TriggersRpc::resolve_trigger_run(
        &lab.app.scoped(&service),
        ResolveTriggerRun { id: id.clone() },
    )
    .await
    .unwrap();
    lab.enable(&id, false).await.unwrap();

    let record = RecordTriggerFire {
        id: id.clone(),
        status: "ok",
    };
    TriggersRpc::record_trigger_fire(&lab.app.scoped(&service), record)
        .await
        .unwrap();

    assert!(!lab.stored(&id).is_enabled());
    assert_eq!(lab.last_fire(&id).as_deref(), Some("ok"));
}

#[tokio::test]
async fn a_fire_now_checks_run_on_the_trigger_then_fires_it() {
    let permissions = Arc::new(RecordingPermissionService::new());
    let lab = lab(permissions.clone()).await;
    let trigger = lab.create(cron()).await.unwrap().trigger;
    let before = permissions.checks().len();

    let job = TriggersRpc::fire_trigger_now(
        &lab.app.scoped(&ctx(alice())),
        FireTriggerNow {
            id: trigger.id().clone(),
        },
    )
    .await
    .unwrap();

    assert_eq!(
        permissions.checks()[before],
        (
            alice(),
            Permission::RunTriggerPipeline(trigger.id().clone())
        )
    );
    assert_eq!(lab.jobs.rows()[0].id(), job.id());
}

#[tokio::test]
async fn the_resolve_refuses_a_caller_that_is_not_a_service() {
    let lab = lab(Arc::new(RecordingPermissionService::new())).await;
    let trigger = lab.seed();

    let err = TriggersRpc::resolve_trigger_run(
        &lab.app.scoped(&ctx(alice())),
        ResolveTriggerRun {
            id: trigger.id().clone(),
        },
    )
    .await
    .err()
    .unwrap();

    assert!(matches!(err, DomainError::Forbidden(_)));
}

async fn serve(lab: Lab) -> TriggerServiceClient<Channel> {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let router = Server::builder().add_service(TriggerServiceServer::new(Grpc::new(lab.app)));
    tokio::spawn(router.serve_with_incoming(TcpIncoming::from(listener)));
    let channel = Channel::from_shared(format!("http://{addr}"))
        .unwrap()
        .connect()
        .await
        .unwrap();
    TriggerServiceClient::new(channel)
}

fn bearer<T>(message: T, bearer: Option<&str>) -> tonic::Request<T> {
    let mut request = tonic::Request::new(message);
    if let Some(bearer) = bearer {
        request
            .metadata_mut()
            .insert("authorization", bearer.parse().unwrap());
    }
    request
}

fn of(id: &TriggerId) -> Option<PbTriggerId> {
    Some(PbTriggerId {
        value: id.to_string(),
    })
}

#[tokio::test]
async fn grpc_serves_the_scylla_contract() {
    let lab = lab(Arc::new(AllButEve)).await;
    let trigger = lab.create(cron()).await.unwrap().trigger;
    let jobs = lab.jobs.clone();
    let mut client = serve(lab).await;

    let read = client
        .get_trigger(bearer(
            GetTriggerRequest {
                trigger_id: of(trigger.id()),
            },
            Some("Bearer user:ann"),
        ))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(read.trigger.unwrap().name, "nightly");

    let anonymous = client
        .get_trigger(bearer(GetTriggerRequest { trigger_id: None }, None))
        .await
        .unwrap_err();
    assert_eq!(anonymous.code(), Code::Unauthenticated, "{anonymous:?}");
    let denied = client
        .get_trigger(bearer(
            GetTriggerRequest {
                trigger_id: of(trigger.id()),
            },
            Some("Bearer user:eve"),
        ))
        .await
        .unwrap_err();
    assert_eq!(denied.code(), Code::PermissionDenied, "{denied:?}");

    let fired = client
        .fire_trigger_now(bearer(
            FireTriggerNowRequest {
                trigger_id: of(trigger.id()),
            },
            Some(&format!("Bearer {}", "user:ann")),
        ))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(fired.job_id.unwrap().value, jobs.rows()[0].id().to_string());
}

const KEY: &str = "key";
const MSG: &[u8] = b"The quick brown fox jumps over the lazy dog";
const SIG: &str = "f7bc83f430538424b13298e6aa6fb143ef4d59a14946175997479dbc2d1a3cd8";

type OnRecord = Box<dyn FnOnce() + Send>;

/// Records the dedupe keys. `on_record` runs once, at the next new record.
#[derive(Default)]
struct StubDeliveries {
    recorded: Mutex<Vec<String>>,
    on_record: Mutex<Option<OnRecord>>,
}

#[async_trait]
impl TriggerDeliveryRepository for StubDeliveries {
    async fn record_or_detect(
        &self,
        _: &TriggerId,
        delivery_id: &str,
        _: DateTime<Utc>,
    ) -> DomainResult<bool> {
        let is_new = {
            let mut seen = self.recorded.lock().unwrap();
            let is_new = !seen.iter().any(|d| d == delivery_id);
            if is_new {
                seen.push(delivery_id.to_string());
            }
            is_new
        };
        let hook = if is_new {
            self.on_record.lock().unwrap().take()
        } else {
            None
        };
        if let Some(hook) = hook {
            hook();
        }
        Ok(is_new)
    }
    async fn forget(&self, _: &TriggerId, delivery_id: &str) -> DomainResult<()> {
        self.recorded.lock().unwrap().retain(|d| d != delivery_id);
        Ok(())
    }
}

fn sign(body: &[u8]) -> String {
    use hmac::{Hmac, KeyInit, Mac};
    let mut mac = Hmac::<sha2::Sha256>::new_from_slice(KEY.as_bytes()).unwrap();
    mac.update(body);
    hex::encode(mac.finalize().into_bytes())
}

fn pointer(key: &str, at: &str) -> TriggerInput {
    TriggerInput::json_pointer(EnvKey::new(key).unwrap(), at).unwrap()
}

impl Lab {
    /// Stores a webhook trigger with the signing secret `KEY`.
    fn hook(&self, header: Option<&str>, inputs: Vec<TriggerInput>) -> Trigger {
        let trigger = Trigger::create(
            pipeline_id(),
            TriggerName::new("hook").unwrap(),
            TriggerSource::Webhook(WebhookSpec::new(header.map(str::to_owned)).unwrap()),
            inputs,
        )
        .unwrap();
        self.triggers.rows.lock().unwrap().insert(
            trigger.id().clone(),
            (trigger.clone(), Some(StubCipher.encrypt(KEY).unwrap())),
        );
        trigger
    }

    /// Provisions the runner app of the organization, with a cron trigger.
    async fn with_runner(&self) {
        self.create(cron()).await.unwrap();
    }

    async fn deliver(
        &self,
        id: &TriggerId,
        body: &[u8],
        signature: Option<&str>,
        delivery_id: Option<&str>,
        event: Option<&str>,
    ) -> DomainResult<IngestOutcome> {
        let headers = signature
            .map(|s| {
                let name = DEFAULT_SIGNATURE_HEADER.to_ascii_lowercase();
                HashMap::from([(name, s.to_string())])
            })
            .unwrap_or_default();
        let cmd = IngestWebhook {
            trigger_id: id.clone(),
            headers,
            delivery_id: delivery_id.map(str::to_string),
            event: event.map(str::to_string),
            body: body.to_vec(),
        };
        Triggers::ingest_webhook(&self.app, cmd).await
    }

    async fn ingest(
        &self,
        id: &TriggerId,
        signature: Option<&str>,
        delivery_id: Option<&str>,
        event: Option<&str>,
    ) -> DomainResult<IngestOutcome> {
        self.deliver(id, MSG, signature, delivery_id, event).await
    }

    fn recorded(&self) -> Vec<String> {
        self.deliveries.recorded.lock().unwrap().clone()
    }
}

#[tokio::test]
async fn signed_ping_answers_without_firing_or_recording() {
    let lab = lab(Arc::new(RecordingPermissionService::new())).await;
    lab.with_runner().await;
    let hook = lab.hook(None, Vec::new());

    let outcome = lab
        .ingest(hook.id(), Some(SIG), Some("d-1"), Some("ping"))
        .await
        .unwrap();

    assert!(matches!(outcome, IngestOutcome::Ping));
    assert!(lab.jobs.rows().is_empty());
    assert!(lab.recorded().is_empty());
}

#[tokio::test]
async fn ping_event_name_is_trimmed_and_case_insensitive() {
    let lab = lab(Arc::new(RecordingPermissionService::new())).await;
    let hook = lab.hook(None, Vec::new());

    let outcome = lab
        .ingest(hook.id(), Some(SIG), None, Some(" Ping "))
        .await
        .unwrap();

    assert!(matches!(outcome, IngestOutcome::Ping));
    assert!(lab.jobs.rows().is_empty());
}

#[tokio::test]
async fn mis_signed_or_unsigned_ping_is_rejected_before_the_event_check() {
    let lab = lab(Arc::new(RecordingPermissionService::new())).await;
    let hook = lab.hook(None, Vec::new());

    let wrong = lab
        .ingest(hook.id(), Some("sha256=00"), None, Some("ping"))
        .await
        .unwrap_err();
    let unsigned = lab
        .ingest(hook.id(), None, None, Some("ping"))
        .await
        .unwrap_err();

    assert!(matches!(wrong, DomainError::Unauthorized(_)), "{wrong:?}");
    assert!(
        matches!(unsigned, DomainError::Unauthorized(_)),
        "{unsigned:?}"
    );
    assert!(lab.recorded().is_empty());
}

#[tokio::test]
async fn other_events_and_no_event_still_fire() {
    let lab = lab(Arc::new(RecordingPermissionService::new())).await;
    lab.with_runner().await;
    let hook = lab.hook(None, Vec::new());

    let push = lab
        .ingest(hook.id(), Some(SIG), Some("d-1"), Some("push"))
        .await
        .unwrap();
    let bare = lab
        .ingest(hook.id(), Some(SIG), Some("d-2"), None)
        .await
        .unwrap();

    assert!(matches!(push, IngestOutcome::Fired(_)));
    assert!(matches!(bare, IngestOutcome::Fired(_)));
    assert_eq!(lab.jobs.rows().len(), 2);
}

#[tokio::test]
async fn a_repeated_delivery_is_a_duplicate_and_fires_once() {
    let lab = lab(Arc::new(RecordingPermissionService::new())).await;
    lab.with_runner().await;
    let hook = lab.hook(None, Vec::new());

    lab.ingest(hook.id(), Some(SIG), Some("d-1"), None)
        .await
        .unwrap();
    let again = lab
        .ingest(hook.id(), Some(SIG), Some(" d-1 "), None)
        .await
        .unwrap();

    assert!(matches!(again, IngestOutcome::Duplicate));
    assert_eq!(lab.jobs.rows().len(), 1);
    assert_eq!(lab.recorded(), vec!["d-1".to_string()]);
}

#[tokio::test]
async fn an_unknown_disabled_or_cron_trigger_is_not_found_without_asking_a_permission() {
    let permissions = Arc::new(RecordingPermissionService::new());
    let lab = lab(permissions.clone()).await;
    let disabled = lab.hook(None, Vec::new());
    lab.triggers
        .rows
        .lock()
        .unwrap()
        .get_mut(disabled.id())
        .unwrap()
        .0
        .disable();
    let nightly = lab.seed();

    for id in [&TriggerId::new("missing"), disabled.id(), nightly.id()] {
        let err = lab.ingest(id, Some(SIG), None, None).await.unwrap_err();
        assert!(matches!(err, DomainError::NotFound(_)), "{id}: {err:?}");
    }

    assert!(permissions.checks().is_empty());
    assert!(lab.recorded().is_empty());
}

#[tokio::test]
async fn a_failed_fire_forgets_the_delivery_so_a_retry_fires() {
    let lab = lab(Arc::new(RecordingPermissionService::new())).await;
    let hook = lab.hook(None, Vec::new());

    let err = lab
        .ingest(hook.id(), Some(SIG), Some("d-1"), None)
        .await
        .unwrap_err();
    assert!(
        matches!(err, DomainError::Internal(_)),
        "no runner app: {err:?}"
    );
    assert!(lab.recorded().is_empty());
    assert_eq!(lab.last_fire(hook.id()).as_deref(), Some("error"));

    lab.with_runner().await;
    let retry = lab
        .ingest(hook.id(), Some(SIG), Some("d-1"), None)
        .await
        .unwrap();
    assert!(matches!(retry, IngestOutcome::Fired(_)));
    assert_eq!(lab.jobs.rows().len(), 1);
}

#[tokio::test]
async fn a_trigger_disabled_while_the_delivery_is_in_flight_is_not_found() {
    let lab = lab(Arc::new(RecordingPermissionService::new())).await;
    lab.with_runner().await;
    let hook = lab.hook(None, Vec::new());
    let triggers = lab.triggers.clone();
    let id = hook.id().clone();
    *lab.deliveries.on_record.lock().unwrap() = Some(Box::new(move || {
        triggers
            .rows
            .lock()
            .unwrap()
            .get_mut(&id)
            .unwrap()
            .0
            .disable();
    }));

    let err = lab
        .ingest(hook.id(), Some(SIG), Some("d-1"), None)
        .await
        .unwrap_err();

    assert!(matches!(err, DomainError::NotFound(_)), "{err:?}");
    assert!(lab.recorded().is_empty());
    assert!(lab.jobs.rows().is_empty());
}

#[tokio::test]
async fn a_fire_refused_for_a_payload_value_stays_a_validation_error() {
    let lab = lab(Arc::new(RecordingPermissionService::new())).await;
    lab.with_runner().await;
    let hook = lab.hook(None, vec![pointer("S", "/s")]);
    let body = br#"{"s":"a\u0000b"}"#;

    let err = lab
        .deliver(hook.id(), body, Some(&sign(body)), Some("d-1"), None)
        .await
        .unwrap_err();

    assert!(matches!(err, DomainError::Validation(_)), "{err:?}");
    assert!(lab.recorded().is_empty());
    assert!(lab.jobs.rows().is_empty());
}

#[tokio::test]
async fn the_prefixed_and_bare_signature_are_one_delivery() {
    let lab = lab(Arc::new(RecordingPermissionService::new())).await;
    lab.with_runner().await;
    let hook = lab.hook(None, Vec::new());

    let first = lab.ingest(hook.id(), Some(SIG), None, None).await.unwrap();
    let prefixed = format!("SHA256={}", SIG.to_uppercase());
    let again = lab
        .ingest(hook.id(), Some(&prefixed), None, None)
        .await
        .unwrap();

    assert!(matches!(first, IngestOutcome::Fired(_)));
    assert!(matches!(again, IngestOutcome::Duplicate));
    assert_eq!(lab.jobs.rows().len(), 1);
    assert_eq!(lab.recorded(), vec![SIG.to_string()]);
}

#[tokio::test]
async fn a_webhook_fire_resolves_the_payload_inputs_and_keeps_the_delivery_id() {
    let permissions = Arc::new(RecordingPermissionService::new());
    let lab = lab(permissions.clone()).await;
    lab.with_runner().await;
    let hook = lab.hook(
        None,
        vec![pointer("GIT_COMMIT", "/after"), pointer("COUNT", "/n")],
    );
    let body = br#"{"after":"abc123","n":42}"#;
    let before = permissions.checks().len();

    let outcome = lab
        .deliver(
            hook.id(),
            body,
            Some(&sign(body)),
            Some("d-7"),
            Some("push"),
        )
        .await
        .unwrap();

    let IngestOutcome::Fired(job_id) = outcome else {
        panic!("the delivery must fire: {outcome:?}");
    };
    let job = lab
        .jobs
        .rows()
        .into_iter()
        .find(|j| j.id() == &job_id)
        .unwrap();
    assert_eq!(
        job.origin(),
        &JobOrigin::Webhook {
            trigger_id: hook.id().clone(),
            delivery_id: Some("d-7".to_string()),
        }
    );
    let inputs: Vec<(String, String)> = job
        .inputs()
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
    assert!(inputs.contains(&("GIT_COMMIT".to_string(), "abc123".to_string())));
    assert!(inputs.contains(&("COUNT".to_string(), "42".to_string())));
    assert_eq!(
        permissions.checks()[before..],
        [
            (lab.runner(), Permission::RunPipeline(pipeline_id())),
            (firer(), Permission::ManageTrigger(hook.id().clone())),
        ]
    );
    assert_eq!(lab.last_fire(hook.id()).as_deref(), Some("ok"));
}

#[tokio::test]
async fn a_custom_signature_header_is_read_under_its_lowercase_name() {
    let lab = lab(Arc::new(RecordingPermissionService::new())).await;
    lab.with_runner().await;
    let hook = lab.hook(Some("X-Hub-Signature-256"), Vec::new());

    let default_header = lab
        .ingest(hook.id(), Some(SIG), Some("d-1"), None)
        .await
        .unwrap_err();
    let cmd = IngestWebhook {
        trigger_id: hook.id().clone(),
        headers: HashMap::from([("x-hub-signature-256".to_string(), format!("sha256={SIG}"))]),
        delivery_id: Some("d-1".to_string()),
        event: None,
        body: MSG.to_vec(),
    };
    let custom = Triggers::ingest_webhook(&lab.app, cmd).await.unwrap();

    assert!(matches!(default_header, DomainError::Unauthorized(_)));
    assert!(matches!(custom, IngestOutcome::Fired(_)));
}
