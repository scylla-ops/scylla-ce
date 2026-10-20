//! The fire and `FireTriggerNow` through the engine, on the trigger lab.

use super::*;
use crate::application::trigger::tests::{Lab, cron, lab, pipeline_id};
use crate::application::trigger::{RecordTriggerFire, ResolveTriggerRun, SetTriggerEnabled};
use crate::domain::caller::{CallerContext, ServiceIdentity};
use crate::domain::errors::DomainError;
use crate::domain::job::JobOrigin;
use crate::domain::permission::Permission;
use crate::test_support::authz::RecordingPermissionService;
use crate::test_support::stubs::alice;

fn runner(lab: &Lab) -> CallerContext {
    CallerContext::App(lab.apps.apps.lock().unwrap()[0].id().clone())
}

fn firer() -> CallerContext {
    CallerContext::Service(ServiceIdentity::trigger_firer())
}

#[tokio::test]
async fn a_fire_runs_as_the_runner_app_then_records_the_outcome_as_the_firer() {
    let permissions = Arc::new(RecordingPermissionService::new());
    let lab = lab(permissions.clone());
    let trigger = lab.create(cron()).await.unwrap().trigger;
    let before = permissions.checks().len();

    let job = lab.firer().fire(trigger.id(), None, None).await.unwrap();

    assert_eq!(
        permissions.checks()[before..],
        [
            (runner(&lab), Permission::RunPipeline(pipeline_id())),
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
    let lab = lab(permissions.clone());
    let trigger = lab.create(cron()).await.unwrap().trigger;
    lab.actions
        .run(
            &*lab.uc,
            &alice(),
            SetTriggerEnabled {
                id: trigger.id().clone(),
                enabled: false,
            },
        )
        .await
        .unwrap();
    let before = permissions.checks().len();

    let err = lab
        .firer()
        .fire(trigger.id(), None, None)
        .await
        .unwrap_err();

    assert!(matches!(err, DomainError::BusinessRule(_)));
    assert_eq!(permissions.checks().len(), before);
    assert!(lab.jobs.rows().is_empty());
    assert_eq!(lab.last_fire(trigger.id()), None);
}

#[tokio::test]
async fn a_failed_run_is_recorded_as_an_error() {
    let lab = lab(Arc::new(RecordingPermissionService::new()));
    let trigger = lab.seed();

    let err = lab
        .firer()
        .fire(trigger.id(), None, None)
        .await
        .unwrap_err();

    assert!(matches!(err, DomainError::Internal(_)), "no runner app");
    assert_eq!(lab.last_fire(trigger.id()).as_deref(), Some("error"));
}

#[tokio::test]
async fn a_fire_record_does_not_undo_a_disable_that_committed_during_the_run() {
    let lab = lab(Arc::new(RecordingPermissionService::new()));
    let id = lab.create(cron()).await.unwrap().trigger.id().clone();
    let resolve = ResolveTriggerRun { id: id.clone() };
    lab.actions.run(&*lab.uc, &firer(), resolve).await.unwrap();
    let disable = SetTriggerEnabled {
        id: id.clone(),
        enabled: false,
    };
    lab.actions.run(&*lab.uc, &alice(), disable).await.unwrap();

    let record = RecordTriggerFire {
        id: id.clone(),
        status: "ok",
    };
    lab.actions.run(&*lab.uc, &firer(), record).await.unwrap();

    assert!(!lab.stored(&id).is_enabled());
    assert_eq!(lab.last_fire(&id).as_deref(), Some("ok"));
}

#[tokio::test]
async fn a_fire_now_checks_run_on_the_trigger_then_fires_it() {
    let permissions = Arc::new(RecordingPermissionService::new());
    let lab = lab(permissions.clone());
    let trigger = lab.create(cron()).await.unwrap().trigger;
    let firing: Arc<dyn TriggerFiring> = lab.firer();
    let uc = TriggerFireUseCases::new(firing);
    let before = permissions.checks().len();

    let job = lab
        .actions
        .run(
            &uc,
            &alice(),
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
    let lab = lab(Arc::new(RecordingPermissionService::new()));
    let trigger = lab.seed();

    let err = lab
        .actions
        .run(
            &*lab.uc,
            &alice(),
            ResolveTriggerRun {
                id: trigger.id().clone(),
            },
        )
        .await
        .err()
        .unwrap();

    assert!(matches!(err, DomainError::Forbidden(_)));
}
