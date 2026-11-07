//! The tests of the conditional nodes of a pipeline: the switch, the merge and the ports of the
//! needs, their checks at the save, and their round trip through the reads.

use scylla_proto::pipelines::v1::{env_var, node, Container, EnvVar, Merge, Node, Test};
use scylla_testkit::{
    case, exec_node, literal, merge_node, output, rule, switch_node, text, var, TestApp,
};
use sqlx::PgPool;

use super::pipeline_layouts::layout;
use crate::{app, assert_invalid_field};

/// Creates a user and a project in its personal workspace. Returns the user and the project id.
async fn project(app: &TestApp) -> (scylla_domain::identity::actors::ActorId, String) {
    let owner = app.user().await;
    let project = app.create_project(owner, "alpha").await.unwrap();
    (owner, text(&project.id).to_owned())
}

/// Returns a pipeline with a switch on a variable and on an output, a merge, and an `error`
/// need.
fn conditional() -> Vec<Node> {
    let rules = vec![
        rule(var("BRANCH"), Test::Equals, Some(literal("main"))),
        rule(
            output("build", "VERSION"),
            Test::NumberGe,
            Some(literal("2")),
        ),
        rule(var("SCYLLA_JOB_ID"), Test::NotEmpty, None),
    ];
    vec![
        exec_node("build", &[]),
        exec_node("test", &["build"]),
        switch_node("is-main", &["test"], vec![case("true", rules)], "false"),
        exec_node("deploy", &["build", "is-main:true"]),
        exec_node("notify", &["test:error"]),
        merge_node("done", &["deploy", "is-main:false"]),
    ]
}

/// Returns the needs of a node as `node` or `node:port`.
fn needs(node: &Node) -> Vec<String> {
    let need = |need: &scylla_proto::pipelines::v1::Need| match need.port.as_str() {
        "" => need.node.clone(),
        port => format!("{}:{port}", need.node),
    };
    node.needs.iter().map(need).collect()
}

/// A pipeline with a switch, a merge and an `error` need keeps its needs with their ports and
/// its switch, in `GetPipeline`, after `UpdatePipeline`, and in `GetJob`.
#[sqlx::test(migrations = false)]
async fn create_pipeline_keeps_the_conditional_nodes(pool: PgPool) {
    // given
    let app = app(pool).await;
    let (owner, project) = project(&app).await;
    let sent = conditional();

    // when
    let created = app
        .create_pipeline(owner, &project, sent.clone())
        .await
        .unwrap();
    let id = text(&created.id).to_owned();
    let read = app.get_pipeline(owner, &id).await.unwrap();
    let updated = app
        .update_pipeline(owner, &id, 1, None, Some(sent.clone()))
        .await;
    let job = app.run_pipeline(owner, &id).await.unwrap();
    let job = app.get_job(owner, text(&job.id)).await.unwrap();

    // then
    for nodes in [&read.nodes, &updated.unwrap().nodes, &job.nodes] {
        let steps: Vec<_> = nodes.iter().map(|node| &node.step).collect();
        let expected: Vec<_> = sent.iter().map(|node| &node.step).collect();
        assert_eq!(steps, expected);
        assert_eq!(needs(&nodes[3]), ["build", "is-main:true"]);
        assert_eq!(needs(&nodes[4]), ["test:error"]);
    }
    assert!(matches!(
        read.nodes[5].step,
        Some(node::Step::Merge(Merge {}))
    ));
    assert!(job.runs.iter().all(|run| run.ports.is_empty()));
}

/// Each check of the conditional nodes refuses its value with its path, in `CreatePipeline` and
/// in `UpdatePipeline`.
#[sqlx::test(migrations = false)]
async fn create_pipeline_checks_the_conditional_nodes(pool: PgPool) {
    // given
    let app = app(pool).await;
    let (owner, project) = project(&app).await;
    let at = "nodes[2].switch.cases[0].rules[0]";
    let with_rule = |left, test, right| {
        let mut nodes = conditional();
        let cases = vec![case("true", vec![rule(left, test, right)])];
        nodes[2] = switch_node("is-main", &["test"], cases, "false");
        nodes
    };
    let changed = |index: usize, change: &dyn Fn(&mut Node)| {
        let mut nodes = conditional();
        change(&mut nodes[index]);
        nodes
    };
    let cases: Vec<(Vec<Node>, String)> = vec![
        (
            changed(3, &|node| node.needs[1].port = "maybe".to_owned()),
            "nodes[3].needs[1].port".to_owned(),
        ),
        (
            changed(4, &|node| node.needs[0].port = "done".to_owned()),
            "nodes[4].needs[0].port".to_owned(),
        ),
        (
            changed(4, &|node| {
                node.needs[0].node = "done".to_owned();
                node.needs[0].port = "true".to_owned();
            }),
            "nodes[4].needs[0].port".to_owned(),
        ),
        (
            changed(5, &|node| node.needs = vec![node.needs[0].clone(); 2]),
            "nodes[5].needs[1]".to_owned(),
        ),
        (
            changed(5, &|node| node.needs.clear()),
            "nodes[5].needs".to_owned(),
        ),
        (
            changed(5, &|node| {
                node.env = vec![EnvVar {
                    name: "A".to_owned(),
                    value: Some(env_var::Value::Literal("1".to_owned())),
                }]
            }),
            "nodes[5].env".to_owned(),
        ),
        (
            changed(2, &|node| node.retries = 1),
            "nodes[2].retries".to_owned(),
        ),
        (
            changed(2, &|node| {
                node.container = Some(Container {
                    image: "alpine".to_owned(),
                })
            }),
            "nodes[2].container".to_owned(),
        ),
        (
            changed(2, &|node| {
                if let Some(node::Step::Switch(switch)) = &mut node.step {
                    switch.cases.clear();
                }
            }),
            "nodes[2].switch.cases".to_owned(),
        ),
        (
            changed(2, &|node| {
                if let Some(node::Step::Switch(switch)) = &mut node.step {
                    switch.otherwise = "true".to_owned();
                }
            }),
            "nodes[2].switch.otherwise".to_owned(),
        ),
        (
            with_rule(var("BRANCH"), Test::Equals, None),
            format!("{at}.right"),
        ),
        (
            with_rule(var("BRANCH"), Test::Empty, Some(literal("x"))),
            format!("{at}.right"),
        ),
        (
            with_rule(var("BRANCH"), Test::NumberGt, Some(literal("ten"))),
            format!("{at}.right.literal"),
        ),
        (
            with_rule(var("1A"), Test::NotEmpty, None),
            format!("{at}.left.var"),
        ),
        (
            with_rule(var("A"), Test::Unspecified, None),
            format!("{at}.test"),
        ),
        (
            with_rule(output("notify", "KEY"), Test::NotEmpty, None),
            format!("{at}.left.output.node"),
        ),
        (
            with_rule(output("build", "bad-key"), Test::NotEmpty, None),
            format!("{at}.left.output.key"),
        ),
    ];

    // when
    let created = app
        .create_pipeline(owner, &project, conditional())
        .await
        .unwrap();
    let id = text(&created.id).to_owned();
    let mut refusals = Vec::new();
    for (nodes, path) in cases {
        let update = app
            .update_pipeline(owner, &id, 1, None, Some(nodes.clone()))
            .await;
        refusals.push((
            app.create_pipeline(owner, &project, nodes).await,
            update,
            path,
        ));
    }

    // then
    for (create, update, path) in refusals {
        assert_invalid_field(&create.unwrap_err(), &path);
        assert_invalid_field(&update.unwrap_err(), &path);
    }
}

/// A switch and a merge keep their places like each node: at the creation and in a save of the
/// layout.
#[sqlx::test(migrations = false)]
async fn save_pipeline_layout_keeps_the_places_of_a_switch_and_a_merge(pool: PgPool) {
    // given
    let app = app(pool).await;
    let (owner, project) = project(&app).await;
    let places = layout(&[("is-main", 10, 20), ("done", 30, 40)]);
    let created = app.create_pipeline_with_layout(owner, &project, conditional(), places);
    let id = text(&created.await.unwrap().id).to_owned();

    // when
    let saved = app.save_pipeline_layout(owner, &id, Some(layout(&[("notify", 50, 60)])));
    saved.await.unwrap();
    let read = app.pipeline_layout(owner, &id).await.unwrap().unwrap();

    // then
    let mut ids: Vec<_> = read.nodes.keys().map(String::as_str).collect();
    ids.sort_unstable();
    assert_eq!(ids, ["done", "is-main", "notify"]);
    assert_eq!((read.nodes["is-main"].x, read.nodes["done"].y), (10, 40));
}
