//! A self-contained aggregate (notes in a versioned in-memory store) drives the engine without
//! the core. The checks are about the pipeline, not about notes.

use crate::domain::caller::{CallerContext, ServiceIdentity};
use crate::domain::errors::{DomainError, DomainResult};
use crate::domain::ids::{OrganizationId, ProjectId, UserId};
use crate::domain::permission::Permission;
use crate::{
    Access, Action, ActionId, Actions, Around, Authorized, Authorizer, Command, Committed, Deleted,
    Describe, Done, Draft, Extension, Fetch, Fetched, Gate, Hooks, Kind, Listener, Next, Observer,
    Path, Persist, Policy, Prepare, Prepared, Proceed, Query, Run, StageKind, Wrap,
};
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tokio::sync::Barrier;

#[derive(Debug, Clone, PartialEq, Eq)]
struct Note {
    id: ProjectId,
    org: OrganizationId,
    title: String,
    version: u64,
}

struct CreateNote {
    org: OrganizationId,
    title: String,
}

struct RenameNote {
    id: ProjectId,
    title: String,
}

struct DeleteNote {
    id: ProjectId,
}

struct ReadNote {
    id: ProjectId,
}

struct MoveNote {
    id: ProjectId,
    org: OrganizationId,
}

struct CountNotes {
    org: OrganizationId,
}

struct WhoAmI;

impl Describe for CreateNote {
    fn access(&self) -> Access {
        Access::Requires(Permission::CreateProject(self.org.clone()))
    }
}

impl Command for CreateNote {
    type Staged = Draft<Note>;
    type Committed = Note;
}

impl Describe for RenameNote {
    fn access(&self) -> Access {
        Access::Requires(Permission::UpdateProject(self.id.clone()))
    }
}

impl Command for RenameNote {
    type Staged = Draft<Note>;
    type Committed = Note;
}

impl Describe for DeleteNote {
    fn access(&self) -> Access {
        Access::Requires(Permission::DeleteProject(self.id.clone()))
    }
}

impl Command for DeleteNote {
    type Staged = Note;
    type Committed = Deleted<Note>;
}

impl Describe for ReadNote {
    fn access(&self) -> Access {
        Access::Requires(Permission::ReadProject(self.id.clone()))
    }
}

impl Query for ReadNote {
    type Output = Note;
}

impl Describe for MoveNote {
    fn access(&self) -> Access {
        Access::RequiresAll(vec![
            Permission::UpdateProject(self.id.clone()),
            Permission::CreateProject(self.org.clone()),
        ])
    }
}

impl Command for MoveNote {
    type Staged = Draft<Note>;
    type Committed = Note;
}

impl Describe for CountNotes {
    fn access(&self) -> Access {
        Access::Public
    }
}

impl Query for CountNotes {
    type Output = usize;
}

impl Describe for WhoAmI {
    fn access(&self) -> Access {
        Access::Authenticated
    }
}

impl Query for WhoAmI {
    type Output = CallerContext;
}

#[derive(Default)]
struct Notes {
    rows: Mutex<HashMap<ProjectId, Note>>,
}

impl Notes {
    fn find(&self, id: &ProjectId) -> Option<Note> {
        self.rows.lock().unwrap().get(id).cloned()
    }

    fn load(&self, id: &ProjectId) -> DomainResult<Note> {
        self.find(id)
            .ok_or_else(|| DomainError::not_found("Note", id.as_str()))
    }

    fn count_in(&self, org: &OrganizationId) -> usize {
        self.rows
            .lock()
            .unwrap()
            .values()
            .filter(|n| n.org == *org)
            .count()
    }

    fn insert(&self, note: Note) -> Note {
        self.rows
            .lock()
            .unwrap()
            .insert(note.id.clone(), note.clone());
        note
    }

    fn update(&self, mut note: Note) -> DomainResult<Note> {
        let mut rows = self.rows.lock().unwrap();
        Self::unchanged(&rows, &note)?;
        note.version += 1;
        rows.insert(note.id.clone(), note.clone());
        Ok(note)
    }

    fn remove(&self, note: &Note) -> DomainResult<()> {
        let mut rows = self.rows.lock().unwrap();
        Self::unchanged(&rows, note)?;
        rows.remove(&note.id);
        Ok(())
    }

    fn unchanged(rows: &HashMap<ProjectId, Note>, note: &Note) -> DomainResult<()> {
        let stored = rows
            .get(&note.id)
            .ok_or_else(|| DomainError::not_found("Note", note.id.as_str()))?;
        if stored.version != note.version {
            return Err(DomainError::stale("Note", note.id.as_str()));
        }
        Ok(())
    }
}

#[async_trait]
impl Run<Prepare<CreateNote>> for Notes {
    async fn run(&self, input: Authorized<CreateNote>) -> DomainResult<Prepared<CreateNote>> {
        let cmd = input.command();
        let note = Note {
            id: ProjectId::generate(),
            org: cmd.org.clone(),
            title: cmd.title.clone(),
            version: 0,
        };
        Ok(input.prepared(Draft::new(note)))
    }
}

#[async_trait]
impl Run<Prepare<RenameNote>> for Notes {
    async fn run(&self, input: Authorized<RenameNote>) -> DomainResult<Prepared<RenameNote>> {
        let cmd = input.command();
        let mut note = self.load(&cmd.id)?;
        note.title.clone_from(&cmd.title);
        Ok(input.prepared(Draft::new(note)))
    }
}

#[async_trait]
impl Run<Prepare<DeleteNote>> for Notes {
    async fn run(&self, input: Authorized<DeleteNote>) -> DomainResult<Prepared<DeleteNote>> {
        let note = self.load(&input.command().id)?;
        Ok(input.prepared(note))
    }
}

#[async_trait]
impl Run<Fetch<ReadNote>> for Notes {
    async fn run(&self, input: Authorized<ReadNote>) -> DomainResult<Fetched<ReadNote>> {
        let note = self.load(&input.command().id)?;
        Ok(input.fetched(note))
    }
}

#[async_trait]
impl Run<Prepare<MoveNote>> for Notes {
    async fn run(&self, input: Authorized<MoveNote>) -> DomainResult<Prepared<MoveNote>> {
        let cmd = input.command();
        let mut note = self.load(&cmd.id)?;
        note.org = cmd.org.clone();
        Ok(input.prepared(Draft::new(note)))
    }
}

#[async_trait]
impl Run<Fetch<CountNotes>> for Notes {
    async fn run(&self, input: Authorized<CountNotes>) -> DomainResult<Fetched<CountNotes>> {
        let count = self.count_in(&input.command().org);
        Ok(input.fetched(count))
    }
}

#[async_trait]
impl Run<Fetch<WhoAmI>> for Notes {
    async fn run(&self, input: Authorized<WhoAmI>) -> DomainResult<Fetched<WhoAmI>> {
        let caller = input.caller().clone();
        Ok(input.fetched(caller))
    }
}

#[async_trait]
impl Run<Persist<CreateNote>> for Notes {
    async fn run(&self, input: Prepared<CreateNote>) -> DomainResult<Committed<CreateNote>> {
        input
            .commit(async |draft| Ok(self.insert(draft.into_inner())))
            .await
    }
}

#[async_trait]
impl Run<Persist<RenameNote>> for Notes {
    async fn run(&self, input: Prepared<RenameNote>) -> DomainResult<Committed<RenameNote>> {
        input
            .commit(async |draft| self.update(draft.into_inner()))
            .await
    }
}

#[async_trait]
impl Run<Persist<MoveNote>> for Notes {
    async fn run(&self, input: Prepared<MoveNote>) -> DomainResult<Committed<MoveNote>> {
        input
            .commit(async |draft| self.update(draft.into_inner()))
            .await
    }
}

#[async_trait]
impl Run<Persist<DeleteNote>> for Notes {
    async fn run(&self, input: Prepared<DeleteNote>) -> DomainResult<Committed<DeleteNote>> {
        input
            .commit(async |note| {
                self.remove(&note)?;
                Ok(Deleted::new(note))
            })
            .await
    }
}

struct Roster {
    admins: Vec<(CallerContext, OrganizationId)>,
    notes: Arc<Notes>,
    asked: Arc<Mutex<Vec<Permission>>>,
}

#[async_trait]
impl Authorizer for Roster {
    async fn authorize(&self, caller: &CallerContext, permission: Permission) -> DomainResult<()> {
        self.asked.lock().unwrap().push(permission.clone());
        if matches!(caller, CallerContext::Service(_)) {
            return Ok(());
        }
        let refused = || DomainError::forbidden(format!("{caller} may not {}", permission.key()));
        let org = match &permission {
            Permission::CreateProject(org) => org.clone(),
            Permission::ReadProject(id)
            | Permission::UpdateProject(id)
            | Permission::DeleteProject(id) => {
                let Some(note) = self.notes.find(id) else {
                    return Err(refused());
                };
                note.org
            }
            _ => return Err(refused()),
        };
        if self.admins.iter().any(|(c, o)| c == caller && *o == org) {
            Ok(())
        } else {
            Err(refused())
        }
    }
}

#[derive(Default)]
struct Journal {
    entries: Mutex<Vec<(ActionId, StageKind, bool)>>,
}

impl Journal {
    fn entries(&self) -> Vec<(ActionId, StageKind, bool)> {
        self.entries.lock().unwrap().clone()
    }

    fn trail(&self, action: &ActionId) -> Vec<(StageKind, bool)> {
        self.entries()
            .into_iter()
            .filter(|(id, _, _)| id == action)
            .map(|(_, stage, ok)| (stage, ok))
            .collect()
    }
}

#[async_trait]
impl Observer for Journal {
    async fn observe(
        &self,
        stage: StageKind,
        action: &dyn Action,
        outcome: Result<(), &DomainError>,
    ) {
        self.entries
            .lock()
            .unwrap()
            .push((action.id().clone(), stage, outcome.is_ok()));
    }
}

impl Extension for Journal {
    fn register(self: &Arc<Self>, hooks: &mut Hooks) {
        for stage in StageKind::ALL {
            hooks.observe(stage, self.clone());
        }
    }
}

struct Quota {
    limit: usize,
    used: Mutex<HashMap<OrganizationId, usize>>,
}

impl Quota {
    fn new(limit: usize) -> Self {
        Self {
            limit,
            used: Mutex::new(HashMap::new()),
        }
    }

    fn used_in(&self, org: &OrganizationId) -> usize {
        self.used.lock().unwrap().get(org).copied().unwrap_or(0)
    }

    fn adjust(&self, org: &OrganizationId, delta: isize) {
        let mut used = self.used.lock().unwrap();
        let slot = used.entry(org.clone()).or_insert(0);
        *slot = slot.saturating_add_signed(delta);
    }
}

#[async_trait]
impl Policy for Quota {
    async fn enforce(&self, _: StageKind, action: &dyn Action) -> DomainResult<()> {
        let Some(org) = action.access().permissions().iter().find_map(|p| match p {
            Permission::CreateProject(org) => Some(org),
            _ => None,
        }) else {
            return Ok(());
        };
        let current = self.used_in(org);
        if current >= self.limit {
            return Err(DomainError::quota_exceeded(format!(
                "project quota reached for this organization ({current}/{})",
                self.limit
            )));
        }
        Ok(())
    }
}

#[async_trait]
impl Observer for Quota {
    async fn observe(&self, _: StageKind, action: &dyn Action, outcome: Result<(), &DomainError>) {
        if outcome.is_err() {
            return;
        }
        match action.access().permissions() {
            [Permission::CreateProject(org)] => self.adjust(org, 1),
            [Permission::DeleteProject(_)] => {
                if let Some(done) = action.downcast_ref::<Committed<DeleteNote>>() {
                    self.adjust(&done.outcome().last_state().org, -1);
                }
            }
            _ => {}
        }
    }
}

impl Extension for Quota {
    fn register(self: &Arc<Self>, hooks: &mut Hooks) {
        hooks
            .policy(StageKind::Prepare, self.clone())
            .observe(StageKind::Persist, self.clone());
    }
}

struct Protect;

#[async_trait]
impl Gate<Persist<DeleteNote>> for Protect {
    async fn check(&self, input: &Prepared<DeleteNote>) -> DomainResult<()> {
        if input.staged().title.starts_with("prod-") {
            return Err(DomainError::business_rule(format!(
                "{} is protected",
                input.staged().title
            )));
        }
        Ok(())
    }
}

impl Extension for Protect {
    fn register(self: &Arc<Self>, hooks: &mut Hooks) {
        hooks.gate::<Persist<DeleteNote>>(self.clone());
    }
}

#[derive(Default)]
struct Timing {
    stages: Mutex<Vec<StageKind>>,
}

#[async_trait]
impl Around for Timing {
    async fn around(&self, stage: StageKind, next: Proceed<'_>) -> DomainResult<Done> {
        let result = next.run().await;
        self.stages.lock().unwrap().push(stage);
        result
    }
}

impl Extension for Timing {
    fn register(self: &Arc<Self>, hooks: &mut Hooks) {
        for stage in StageKind::ALL {
            hooks.around(stage, self.clone());
        }
    }
}

struct DryRun;

#[async_trait]
impl Wrap<Persist<CreateNote>> for DryRun {
    async fn wrap(
        &self,
        input: Prepared<CreateNote>,
        _: Next<'_, Persist<CreateNote>>,
    ) -> DomainResult<Committed<CreateNote>> {
        input.commit(async |draft| Ok(draft.into_inner())).await
    }
}

struct Meet(Arc<Barrier>);

#[async_trait]
impl Wrap<Persist<RenameNote>> for Meet {
    async fn wrap(
        &self,
        input: Prepared<RenameNote>,
        next: Next<'_, Persist<RenameNote>>,
    ) -> DomainResult<Committed<RenameNote>> {
        self.0.wait().await;
        next.run(input).await
    }
}

#[derive(Default)]
struct Witness {
    seen: Mutex<Vec<(CallerContext, Access)>>,
}

#[async_trait]
impl Listener<Persist<CreateNote>> for Witness {
    async fn listen(&self, output: &Committed<CreateNote>) {
        self.seen
            .lock()
            .unwrap()
            .push((output.caller().clone(), output.access().clone()));
    }
}

struct Cache(Mutex<HashMap<ProjectId, Note>>);

#[async_trait]
impl Wrap<Fetch<ReadNote>> for Cache {
    async fn wrap(
        &self,
        input: Authorized<ReadNote>,
        next: Next<'_, Fetch<ReadNote>>,
    ) -> DomainResult<Fetched<ReadNote>> {
        let cached = self.0.lock().unwrap().get(&input.command().id).cloned();
        if let Some(note) = cached {
            return Ok(input.fetched(note));
        }
        let fetched = next.run(input).await?;
        self.0
            .lock()
            .unwrap()
            .insert(fetched.output().id.clone(), fetched.output().clone());
        Ok(fetched)
    }
}

#[derive(Default)]
struct Seen(Mutex<Vec<Access>>);

#[async_trait]
impl Policy for Seen {
    async fn enforce(&self, _: StageKind, action: &dyn Action) -> DomainResult<()> {
        self.0.lock().unwrap().push(action.access().clone());
        Ok(())
    }
}

struct Lab {
    actions: Actions,
    notes: Arc<Notes>,
    asked: Arc<Mutex<Vec<Permission>>>,
    journal: Arc<Journal>,
    quota: Arc<Quota>,
    timing: Arc<Timing>,
}

impl Lab {
    fn asked(&self) -> Vec<Permission> {
        std::mem::take(&mut *self.asked.lock().unwrap())
    }

    async fn run<A, K>(&self, caller: &CallerContext, action: A) -> DomainResult<A::Output>
    where
        A: Path<K, Notes>,
        K: Kind,
    {
        self.actions.run(&*self.notes, caller, action).await
    }
}

fn lab(limit: usize) -> Lab {
    lab_with(limit, |_| {})
}

fn lab_with(limit: usize, extra: impl FnOnce(&mut Hooks)) -> Lab {
    let notes = Arc::new(Notes::default());
    let journal = Arc::new(Journal::default());
    let quota = Arc::new(Quota::new(limit));
    let timing = Arc::new(Timing::default());
    let mut hooks = Hooks::new()
        .with(&quota)
        .with(&Arc::new(Protect))
        .with(&journal)
        .with(&timing);
    extra(&mut hooks);
    let asked = Arc::new(Mutex::new(Vec::new()));
    let roster = Roster {
        admins: vec![(alice(), acme())],
        notes: notes.clone(),
        asked: asked.clone(),
    };
    Lab {
        actions: Actions::new(Arc::new(roster), Arc::new(hooks)),
        notes,
        asked,
        journal,
        quota,
        timing,
    }
}

fn alice() -> CallerContext {
    CallerContext::User(UserId::new("alice"))
}

fn bob() -> CallerContext {
    CallerContext::User(UserId::new("bob"))
}

fn acme() -> OrganizationId {
    OrganizationId::new("acme")
}

fn globex() -> OrganizationId {
    OrganizationId::new("globex")
}

fn create(title: &str) -> CreateNote {
    CreateNote {
        org: acme(),
        title: title.to_owned(),
    }
}

fn rename(id: &ProjectId, title: &str) -> RenameNote {
    RenameNote {
        id: id.clone(),
        title: title.to_owned(),
    }
}

fn delete(id: &ProjectId) -> DeleteNote {
    DeleteNote { id: id.clone() }
}

fn read(id: &ProjectId) -> ReadNote {
    ReadNote { id: id.clone() }
}

#[tokio::test]
async fn one_command_crosses_the_three_stages_in_order() {
    let lab = lab(10);
    lab.run(&alice(), create("one")).await.unwrap();

    let entries = lab.journal.entries();
    let action = &entries[0].0;
    assert_eq!(
        lab.journal.trail(action),
        vec![
            (StageKind::Authorize, true),
            (StageKind::Prepare, true),
            (StageKind::Persist, true)
        ]
    );
    assert_eq!(
        lab.timing.stages.lock().unwrap().clone(),
        [StageKind::Authorize, StageKind::Prepare, StageKind::Persist]
    );
}

#[tokio::test]
async fn one_query_crosses_two_stages_and_takes_the_same_hooks() {
    let lab = lab(10);
    let note = lab.run(&alice(), create("one")).await.unwrap();

    let seen = lab.run(&alice(), read(&note.id)).await.unwrap();

    assert_eq!(seen, note);
    let entries = lab.journal.entries();
    assert_eq!(
        lab.journal.trail(&entries[3].0),
        vec![(StageKind::Authorize, true), (StageKind::Fetch, true)]
    );
    assert_eq!(
        lab.timing.stages.lock().unwrap()[3..],
        [StageKind::Authorize, StageKind::Fetch]
    );
    let err = lab.run(&bob(), read(&note.id)).await.unwrap_err();
    assert!(matches!(err, DomainError::Forbidden(_)));
}

#[tokio::test]
async fn a_wrap_on_fetch_serves_the_second_read_from_its_cache() {
    let lab = lab_with(10, |hooks| {
        hooks.wrap::<Fetch<ReadNote>>(Arc::new(Cache(Mutex::default())));
    });
    let note = lab.run(&alice(), create("one")).await.unwrap();

    let first = lab.run(&alice(), read(&note.id)).await.unwrap();
    lab.run(&alice(), rename(&note.id, "two")).await.unwrap();
    let second = lab.run(&alice(), read(&note.id)).await.unwrap();

    assert_eq!(first.title, "one");
    assert_eq!(second.title, "one");
    assert_eq!(lab.notes.find(&note.id).unwrap().title, "two");
}

#[tokio::test]
async fn the_committed_event_names_the_caller_and_the_access() {
    let witness = Arc::new(Witness::default());
    let lab = lab_with(10, |hooks| {
        hooks.listen::<Persist<CreateNote>>(witness.clone());
    });
    let note = lab.run(&alice(), create("one")).await.unwrap();

    assert_eq!(note.title, "one");
    assert_eq!(lab.notes.count_in(&acme()), 1);
    assert_eq!(
        witness.seen.lock().unwrap().clone(),
        vec![(alice(), Access::Requires(Permission::CreateProject(acme())))]
    );
}

#[tokio::test]
async fn a_policy_veto_stops_the_chain_and_is_observed() {
    let lab = lab(2);
    lab.run(&alice(), create("one")).await.unwrap();
    let second = lab.run(&alice(), create("two")).await.unwrap();

    let err = lab.run(&alice(), create("three")).await.unwrap_err();
    assert!(matches!(
        &err,
        DomainError::QuotaExceeded(m) if m == "project quota reached for this organization (2/2)"
    ));
    assert_eq!(lab.notes.count_in(&acme()), 2);
    let vetoed = lab.journal.entries().last().unwrap().clone();
    assert_eq!((vetoed.1, vetoed.2), (StageKind::Prepare, false));

    lab.run(&alice(), delete(&second.id)).await.unwrap();
    assert_eq!(lab.quota.used_in(&acme()), 1);
    lab.run(&alice(), create("three")).await.unwrap();
}

#[tokio::test]
async fn a_gate_veto_on_persist_leaves_the_row() {
    let lab = lab(10);
    let prod = lab.run(&alice(), create("prod-db")).await.unwrap();

    let err = lab.run(&alice(), delete(&prod.id)).await.unwrap_err();

    assert!(matches!(err, DomainError::BusinessRule(_)));
    assert!(lab.notes.find(&prod.id).is_some());
    assert_eq!(lab.quota.used_in(&acme()), 1);
}

#[tokio::test]
async fn a_forbidden_caller_is_journaled_at_authorize_only() {
    let lab = lab(10);
    let err = lab.run(&bob(), create("mine")).await.unwrap_err();

    assert!(matches!(err, DomainError::Forbidden(_)));
    let entries = lab.journal.entries();
    assert_eq!(entries.len(), 1);
    assert_eq!((entries[0].1, entries[0].2), (StageKind::Authorize, false));
    assert_eq!(lab.notes.count_in(&acme()), 0);
    assert_eq!(lab.quota.used_in(&acme()), 0);
}

#[tokio::test]
async fn an_around_registered_once_covers_every_command() {
    let lab = lab(10);
    let created = lab.run(&alice(), create("one")).await.unwrap();
    lab.run(&alice(), rename(&created.id, "two")).await.unwrap();

    let write = [StageKind::Authorize, StageKind::Prepare, StageKind::Persist];
    assert_eq!(
        lab.timing.stages.lock().unwrap().clone(),
        [write, write].concat()
    );
}

#[tokio::test]
async fn a_wrap_may_skip_the_stage_for_a_simulation() {
    let lab = lab_with(10, |hooks| {
        hooks.wrap::<Persist<CreateNote>>(Arc::new(DryRun));
    });
    let done = lab.run(&alice(), create("ghost")).await.unwrap();

    assert_eq!(done.title, "ghost");
    assert_eq!(lab.notes.count_in(&acme()), 0);
    assert_eq!(lab.quota.used_in(&acme()), 1);
}

#[tokio::test]
async fn two_writes_staged_from_the_same_read_commit_once() {
    let barrier = Arc::new(Barrier::new(2));
    let lab = lab_with(10, |hooks| {
        hooks.wrap::<Persist<RenameNote>>(Arc::new(Meet(barrier.clone())));
    });
    let created = lab.run(&alice(), create("one")).await.unwrap();
    let id = created.id.clone();
    let alice = alice();

    let (a, b) = tokio::join!(
        lab.run(&alice, rename(&id, "a")),
        lab.run(&alice, rename(&id, "b")),
    );
    let (won, lost) = match (a, b) {
        (Ok(won), Err(lost)) | (Err(lost), Ok(won)) => (won, lost),
        (Ok(_), Ok(_)) => panic!("both renames committed"),
        (Err(a), Err(b)) => panic!("both renames failed: {a}, {b}"),
    };

    assert!(matches!(lost, DomainError::Stale(_)));
    let stored = lab.notes.find(&id).unwrap();
    assert_eq!(stored.title, won.title);
    assert_eq!(stored.version, 1);
}

#[tokio::test]
async fn a_delete_of_an_unknown_note_is_refused_before_prepare() {
    let lab = lab(10);
    let missing = ProjectId::new("missing");

    let err = lab.run(&alice(), delete(&missing)).await.unwrap_err();
    assert!(matches!(err, DomainError::Forbidden(_)));
    assert_eq!(lab.journal.entries().len(), 1);

    let system = CallerContext::Service(ServiceIdentity::recorder());
    let err = lab.run(&system, delete(&missing)).await.unwrap_err();
    assert!(matches!(err, DomainError::NotFound(_)));
    let entries = lab.journal.entries();
    assert_eq!(entries.len(), 3);
    assert_eq!((entries[1].1, entries[1].2), (StageKind::Authorize, true));
    assert_eq!((entries[2].1, entries[2].2), (StageKind::Prepare, false));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_run_can_move_to_another_task() {
    let lab = Arc::new(lab(10));
    let spawned = lab.clone();
    let done = tokio::spawn(async move { spawned.run(&alice(), create("one")).await })
        .await
        .unwrap()
        .unwrap();

    assert_eq!(lab.notes.find(&done.id).unwrap().title, "one");
}

#[tokio::test]
async fn a_public_query_runs_without_the_authorizer() {
    let lab = lab(10);
    lab.run(&alice(), create("one")).await.unwrap();
    lab.asked();

    let count = lab
        .run(&CallerContext::Anonymous, CountNotes { org: acme() })
        .await
        .unwrap();

    assert_eq!(count, 1);
    assert!(lab.asked().is_empty());
}

#[tokio::test]
async fn an_authenticated_query_refuses_anonymous_only() {
    let lab = lab(10);

    let err = lab
        .run(&CallerContext::Anonymous, WhoAmI)
        .await
        .unwrap_err();
    assert!(matches!(err, DomainError::Forbidden(_)));
    let entries = lab.journal.entries();
    assert_eq!(entries.len(), 1);
    assert_eq!((entries[0].1, entries[0].2), (StageKind::Authorize, false));

    assert_eq!(lab.run(&bob(), WhoAmI).await.unwrap(), bob());
    let system = CallerContext::Service(ServiceIdentity::recorder());
    assert_eq!(lab.run(&system, WhoAmI).await.unwrap(), system);
    assert!(lab.asked().is_empty());
}

#[tokio::test]
async fn every_permission_of_an_action_must_pass_and_the_first_refusal_stops() {
    let lab = lab(10);
    let note = lab.run(&alice(), create("one")).await.unwrap();
    let update = Permission::UpdateProject(note.id.clone());
    let move_to = |org| MoveNote {
        id: note.id.clone(),
        org,
    };
    lab.asked();

    let err = lab.run(&bob(), move_to(acme())).await.unwrap_err();
    assert!(matches!(err, DomainError::Forbidden(_)));
    assert_eq!(lab.asked(), vec![update.clone()]);

    let err = lab.run(&alice(), move_to(globex())).await.unwrap_err();
    assert!(matches!(err, DomainError::Forbidden(_)));
    assert_eq!(
        lab.asked(),
        vec![update.clone(), Permission::CreateProject(globex())]
    );
    assert_eq!(lab.notes.find(&note.id).unwrap().org, acme());

    let moved = lab.run(&alice(), move_to(acme())).await.unwrap();
    assert_eq!(moved.version, 1);
    assert_eq!(lab.asked(), vec![update, Permission::CreateProject(acme())]);
}

#[tokio::test]
async fn the_hooks_run_the_same_way_for_every_access() {
    let seen = Arc::new(Seen::default());
    let lab = lab_with(10, |hooks| {
        hooks.policy(StageKind::Authorize, seen.clone());
    });
    let note = lab.run(&alice(), create("one")).await.unwrap();
    let before = lab.journal.entries().len();
    lab.timing.stages.lock().unwrap().clear();
    seen.0.lock().unwrap().clear();

    lab.run(&CallerContext::Anonymous, CountNotes { org: acme() })
        .await
        .unwrap();
    lab.run(&bob(), WhoAmI).await.unwrap();
    lab.run(&alice(), read(&note.id)).await.unwrap();

    let entries = lab.journal.entries();
    let read = [(StageKind::Authorize, true), (StageKind::Fetch, true)];
    for action in entries[before..].iter().step_by(2) {
        assert_eq!(lab.journal.trail(&action.0), read);
    }
    assert_eq!(entries.len() - before, 6);
    let read = [StageKind::Authorize, StageKind::Fetch];
    assert_eq!(
        lab.timing.stages.lock().unwrap().clone(),
        [read, read, read].concat()
    );
    assert_eq!(
        seen.0.lock().unwrap().clone(),
        [
            Access::Public,
            Access::Authenticated,
            Access::Requires(Permission::ReadProject(note.id.clone()))
        ]
    );
}
