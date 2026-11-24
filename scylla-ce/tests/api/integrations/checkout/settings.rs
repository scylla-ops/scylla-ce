//! The tests of the checkout settings of an editor pipeline, and of the features of an agent.

use std::time::Duration;

use scylla_proto::pipelines::v1::git_ref::Kind;
use scylla_proto::pipelines::v1::{CheckoutSettings, PipelineFileFormat};
use scylla_testkit::{exec_node, text, FakeProvider};
use sqlx::PgPool;

use super::{app_with, github, linked};

/// Returns checkout settings.
fn settings(enabled: bool, depth: u32, submodules: bool, lfs: bool) -> CheckoutSettings {
    CheckoutSettings {
        enabled,
        depth,
        submodules,
        lfs,
    }
}

/// A pipeline keeps the checkout settings of its creation and of a change, and a pipeline with
/// none has the default. The export writes them.
#[sqlx::test(migrations = false)]
async fn a_pipeline_keeps_its_checkout_settings(pool: PgPool) {
    // given
    let app = crate::app(pool).await;
    let owner = app.user().await;
    let project = app.create_project(owner, "alpha").await.unwrap();
    let project = text(&project.id).to_owned();
    let nodes = || vec![exec_node("build", &[])];
    let full = settings(true, 0, true, true);

    // when
    let plain = app.create_pipeline(owner, &project, nodes()).await.unwrap();
    let created = app
        .create_pipeline_with_checkout(owner, &project, nodes(), full)
        .await
        .unwrap();
    let id = text(&created.id).to_owned();
    let read = app.get_pipeline(owner, &id).await.unwrap();
    let off = settings(false, 1, false, false);
    let changed = app
        .update_pipeline_checkout(owner, &id, created.version, off)
        .await
        .unwrap();
    let renamed = app
        .update_pipeline(owner, &id, changed.version, Some("deploy"), None)
        .await
        .unwrap();
    let export = app.export_pipeline(owner, &id, PipelineFileFormat::Toml);

    // then
    assert_eq!(plain.checkout, Some(settings(true, 1, false, false)));
    assert_eq!(created.checkout, Some(full));
    assert_eq!(read.checkout, Some(full));
    assert_eq!(changed.checkout, Some(off));
    assert_eq!(renamed.checkout, Some(off));
    let content = export.await.unwrap().content;
    assert!(content.contains("enabled = false"), "{content}");
}

/// A job of a pipeline with the checkout off has no checkout, and an agent with no `checkout`
/// gets it. A job of a pipeline with settings gets them in its dispatch.
#[sqlx::test(migrations = false)]
async fn the_dispatch_follows_the_checkout_settings(pool: PgPool) {
    // given
    let fake = FakeProvider::start().await;
    github(&fake);
    let app = app_with(pool, &fake).await;
    let off = linked(&app, "github", Some(settings(false, 1, false, false))).await;
    let deep = linked(&app, "github", Some(settings(true, 0, true, true))).await;
    let main = || Kind::Branch("main".to_owned());
    let job = app.run_pipeline_on(off.owner, &off.pipeline, main());
    let job = job.await.unwrap();
    app.run_pipeline_on(deep.owner, &deep.pipeline, main())
        .await
        .unwrap();

    // when
    let mut old = app.agent_with(off.owner, off.workspace, &[]).await;
    let plain = old.dispatch_in(Duration::from_secs(10)).await;
    let (full, _up) = app.first_dispatch(deep.owner, deep.workspace).await;

    // then
    assert!(job.source.is_some() && job.checkout.is_none());
    assert!(plain.expect("a dispatch").checkout.is_none());
    let checkout = full.checkout.expect("a checkout");
    assert_eq!(
        (checkout.depth, checkout.submodules, checkout.lfs),
        (0, true, true)
    );
}

/// The hello of an agent stores its features, with no empty and no double one.
#[sqlx::test(migrations = false)]
async fn a_hello_stores_the_features_of_the_agent(pool: PgPool) {
    // given
    let app = crate::app(pool).await;
    let owner = app.user().await;
    let workspace = app.workspace_of(owner).await;
    let features = ["checkout", " checkout ", "", "lfs"];

    // when
    let agent = app.agent_with(owner, workspace, &features).await;
    let read = app.get_app(owner, &agent.key.app).await.unwrap();

    // then
    let host = read.app.expect("an app").host.expect("a host");
    assert_eq!(host.features, ["checkout", "lfs"]);
}
