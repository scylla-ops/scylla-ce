//! The tests of the conditional nodes on a real agent and the real server of the Community
//! edition: a switch on a variable and on an output, the ports of `all_matches`, a merge, the
//! port `error` of a failure, and a failure that no need takes. Keep at most 4 tests here
//! (decision 102).

use std::time::Duration;

use scylla_agent::Settings;
use scylla_domain::identity::actors::ActorId;
use scylla_domain::projects::pipelines::PipelineHolder;
use scylla_proto::pipelines::v1::{
    env_var, node, EnvVar, Exec, Job, JobStatus, Node, NodeStatus, Test,
};
use scylla_testkit::{
    case, exec_node, literal, merge_node, output, rule, switch_node, text, var, TestApp,
};
use sqlx::PgPool;
use tokio::sync::oneshot;

/// Starts the server and an agent with the role runner, runs each list of nodes as a pipeline
/// after the end of the job before, and returns each job at its end.
async fn run(pool: PgPool, pipelines: Vec<Vec<Node>>) -> Vec<Job> {
    // Start the server, a project, and the agent.
    let app = TestApp::spawn(pool, scylla_ce::edition()).await;
    let owner = app.user().await;
    let workspace = app.workspace_of(owner).await;
    let project = text(&app.create_project(owner, "alpha").await.unwrap().id).to_owned();
    let created = app.create_app(owner, workspace, true).await.unwrap();
    let url = format!("http://{}", app.addr);
    let config = scylla_agent::join(&url, &created.join_code).await.unwrap();
    let agent = ActorId::parse(&config.app_id).unwrap();
    app.grant(&[PipelineHolder::RUNNER.of(workspace, agent)])
        .await;
    let root = std::env::temp_dir().join(format!("scylla-agent-{}", ActorId::generate()));
    let settings = Settings {
        workspace_root: root.clone(),
        capacity: 2,
        container_runtime: None,
        keep_workspace: false,
    };
    let (stop, stopped) = oneshot::channel::<()>();
    let task = tokio::spawn(scylla_agent::run(config, settings, async {
        let _ = stopped.await;
    }));

    // Run each pipeline until the end of its job, for at most 30 seconds.
    let mut jobs = Vec::new();
    for nodes in pipelines {
        let pipeline = app.create_pipeline(owner, &project, nodes).await.unwrap();
        let job = app.run_pipeline(owner, text(&pipeline.id)).await.unwrap();
        let mut ended = None;
        for _ in 0..300 {
            let read = app.get_job(owner, text(&job.id)).await.unwrap();
            if matches!(read.status(), JobStatus::Completed | JobStatus::Failed) {
                ended = Some(read);
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        jobs.push(ended.expect("the job did not end"));
    }

    // Stop the agent, and delete its folders.
    let _ = stop.send(());
    let _ = task.await;
    let _ = std::fs::remove_dir_all(&root);
    jobs
}

/// Makes a node that runs a shell command.
fn shell(node_id: &str, needs: &[&str], command: &str) -> Node {
    let mut node = exec_node(node_id, needs);
    node.step = Some(node::Step::Exec(Exec {
        argv: vec!["sh".to_owned(), "-c".to_owned(), command.to_owned()],
    }));
    node
}

/// Returns the env of one literal variable.
fn env(name: &str, value: &str) -> Vec<EnvVar> {
    vec![EnvVar {
        name: name.to_owned(),
        value: Some(env_var::Value::Literal(value.to_owned())),
    }]
}

/// Makes a switch with a literal variable in its env: `true` when the variable equals `value`,
/// else the otherwise port `false`.
fn equals(node_id: &str, needs: &[&str], name: &str, given: &str, value: &str) -> Node {
    let rules = vec![rule(var(name), Test::Equals, Some(literal(value)))];
    let mut node = switch_node(node_id, needs, vec![case("true", rules)], "false");
    node.env = env(name, given);
    node
}

/// Returns the state of each node of a job.
fn states(job: &Job) -> Vec<NodeStatus> {
    job.runs.iter().map(|run| run.status()).collect()
}

/// Returns the ports that a node of a job opened.
fn ports<'a>(job: &'a Job, node_id: &str) -> Vec<&'a str> {
    let run = job.runs.iter().find(|run| run.node_id == node_id).unwrap();
    run.ports.iter().map(String::as_str).collect()
}

/// The states of the nodes.
const DONE: NodeStatus = NodeStatus::Completed;
/// A node that did not run.
const SKIPPED: NodeStatus = NodeStatus::Skipped;
/// A node that failed.
const FAILED: NodeStatus = NodeStatus::Failed;

/// A switch opens the port of its case that matches: on a variable, on an output of
/// `$SCYLLA_OUTPUT`, and on each case with `all_matches`. The nodes after a closed port, after a
/// switch with no port, and after a closed merge are skipped, and the jobs complete.
#[sqlx::test(migrations = false)]
async fn a_switch_opens_the_ports_of_its_cases(pool: PgPool) {
    // given
    let mut none = equals("none", &[], "BRANCH", "main", "dev");
    if let Some(node::Step::Switch(switch)) = &mut none.step {
        switch.otherwise.clear();
    }
    let on_var = vec![
        equals("is-main", &[], "BRANCH", "main", "main"),
        shell("deploy", &["is-main:true"], "true"),
        shell("docs", &["is-main:false"], "true"),
        merge_node("join", &["docs"]),
        none,
        shell("after-none", &["none:true"], "true"),
    ];
    let script = "[ ! -s \"$SCYLLA_OUTPUT\" ] || exit 9\necho VERSION=2 >> \"$SCYLLA_OUTPUT\"\n\
                  [ -f marker ] || { touch marker; exit 1; }";
    let mut build = shell("build", &[], script);
    build.retries = 1;
    let version = vec![rule(
        output("build", "VERSION"),
        Test::NumberGe,
        Some(literal("2")),
    )];
    let on_output = vec![
        build,
        switch_node("gate", &["build"], vec![case("new", version)], "old"),
        shell("new", &["gate:new"], "true"),
        shell("old", &["gate:old"], "true"),
    ];
    let each_need = vec![
        shell("build", &[], "true"),
        equals("is-main", &[], "BRANCH", "dev", "main"),
        shell("deploy", &["build", "is-main:true"], "true"),
    ];
    let count = |test: Test, right: &str| vec![rule(var("COUNT"), test, Some(literal(right)))];
    let cases = vec![
        case("big", count(Test::NumberGt, "10")),
        case("even", count(Test::EndsWith, "2")),
        case("small", count(Test::NumberLt, "5")),
    ];
    let mut gate = switch_node("gate", &[], cases, "");
    if let Some(node::Step::Switch(switch)) = &mut gate.step {
        switch.all_matches = true;
    }
    gate.env = env("COUNT", "12");
    let all_matches = vec![
        gate,
        shell("a", &["gate:big"], "true"),
        shell("b", &["gate:even"], "true"),
        shell("c", &["gate:small"], "true"),
    ];

    // when
    let jobs = run(pool, vec![on_var, on_output, each_need, all_matches]).await;

    // then
    for job in &jobs {
        assert_eq!(job.status(), JobStatus::Completed, "{}", job.error);
    }
    assert_eq!(
        states(&jobs[0]),
        [DONE, DONE, SKIPPED, SKIPPED, DONE, SKIPPED]
    );
    assert_eq!(ports(&jobs[0], "is-main"), ["true"]);
    assert!(ports(&jobs[0], "none").is_empty());
    assert_eq!(states(&jobs[1]), [DONE, DONE, DONE, SKIPPED]);
    assert_eq!(
        (ports(&jobs[1], "gate"), jobs[1].runs[0].attempt),
        (vec!["new"], 2)
    );
    assert_eq!(states(&jobs[2]), [DONE, DONE, SKIPPED]);
    assert_eq!(ports(&jobs[2], "is-main"), ["false"]);
    assert_eq!(states(&jobs[3]), [DONE, DONE, DONE, SKIPPED]);
    assert_eq!(ports(&jobs[3], "gate"), ["big", "even"]);
}

/// An if and an else join in a merge, and the node after the merge runs on each branch.
#[sqlx::test(migrations = false)]
async fn a_merge_joins_each_branch(pool: PgPool) {
    // given
    let if_else = |branch: &str| {
        vec![
            equals("is-main", &[], "BRANCH", branch, "main"),
            shell("release", &["is-main:true"], "true"),
            shell("preview", &["is-main:false"], "true"),
            merge_node("join", &["release", "preview"]),
            shell("notify", &["join"], "true"),
        ]
    };

    // when
    let jobs = run(pool, vec![if_else("main"), if_else("dev")]).await;

    // then
    for job in &jobs {
        assert_eq!(job.status(), JobStatus::Completed, "{}", job.error);
    }
    assert_eq!(states(&jobs[0]), [DONE, DONE, SKIPPED, DONE, DONE]);
    assert_eq!(states(&jobs[1]), [DONE, SKIPPED, DONE, DONE, DONE]);
}

/// A failure with an `error` need runs that node with `SCYLLA_ERROR`, lets a parallel node end,
/// skips the nodes that need a success, and fails the job with the first failure.
#[sqlx::test(migrations = false)]
async fn an_error_need_takes_a_failure(pool: PgPool) {
    // given
    let two = "[ \"$SCYLLA_ERROR\" = \"$(printf 'test: exit code 1\\nlint: exit code 2')\" ]";
    let nodes = vec![
        shell("test", &[], "exit 1"),
        shell("lint", &[], "sleep 1; exit 2"),
        shell("slow", &[], "sleep 2"),
        shell(
            "notify",
            &["test:error"],
            "[ \"$SCYLLA_ERROR\" = 'test: exit code 1' ]",
        ),
        shell("report", &["test:error", "lint:error"], two),
        shell("deploy", &["test"], "true"),
        merge_node("always", &["test", "test:error"]),
        shell("never", &["test", "test:error"], "true"),
    ];

    // when
    let jobs = run(pool, vec![nodes]).await;

    // then
    let job = &jobs[0];
    assert_eq!(job.status(), JobStatus::Failed);
    assert_eq!(job.error, "the node test failed: exit code 1");
    let expected = [FAILED, FAILED, DONE, DONE, DONE, SKIPPED, DONE, SKIPPED];
    assert_eq!(states(job), expected);
}

/// A failure that no `error` need takes stops the running nodes, skips the others, and fails
/// the job. A switch that tests a number on a text that is not a number fails with the reason.
#[sqlx::test(migrations = false)]
async fn a_failure_that_no_need_takes_stops_the_job(pool: PgPool) {
    // given
    let failure = vec![
        shell("test", &[], "exit 1"),
        shell("slow", &[], "sleep 30"),
        shell("deploy", &["test"], "true"),
    ];
    let rules = vec![rule(var("COUNT"), Test::NumberGt, Some(literal("10")))];
    let mut gate = switch_node("gate", &[], vec![case("big", rules)], "");
    gate.env = env("COUNT", "many");
    let not_a_number = vec![gate, shell("after", &["gate:big"], "true")];

    // when
    let jobs = run(pool, vec![failure, not_a_number]).await;

    // then
    let reason = "rule 1 of the case big: \"many\" is not a number";
    for job in &jobs {
        assert_eq!(job.status(), JobStatus::Failed);
    }
    assert_eq!(jobs[0].error, "the node test failed: exit code 1");
    assert_eq!(states(&jobs[0]), [FAILED, SKIPPED, SKIPPED]);
    assert_eq!(jobs[1].error, format!("the node gate failed: {reason}"));
    assert_eq!(jobs[1].runs[0].error, reason);
    assert_eq!(states(&jobs[1]), [FAILED, SKIPPED]);
}
