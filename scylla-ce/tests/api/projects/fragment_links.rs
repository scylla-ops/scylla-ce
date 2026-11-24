//! The tests of the fragment links of a pipeline: their save with the nodes, their checks, and
//! a change of the nodes or of the links alone.

use scylla_testkit::{exec_node, fragment_link, text};
use sqlx::PgPool;

use crate::{app, assert_invalid_field, UNKNOWN_ID};

/// A pipeline keeps its links, and a read gives them back. The server does not read the
/// fragment: a link can name a fragment that it does not know.
#[sqlx::test(migrations = false)]
async fn create_pipeline_keeps_the_links(pool: PgPool) {
    // given
    let app = app(pool).await;
    let owner = app.user().await;
    let project = text(&app.create_project(owner, "web").await.unwrap().id).to_owned();
    let nodes = vec![exec_node("build", &[]), exec_node("test", &["build"])];
    let links = vec![fragment_link("ci", UNKNOWN_ID, 3, &["build", "test"])];

    // when
    let created = app
        .create_linked_pipeline(owner, &project, nodes, links.clone())
        .await
        .unwrap();
    let read = app.get_pipeline(owner, text(&created.id)).await.unwrap();

    // then
    assert_eq!(read.fragment_links, links);
    assert_eq!(created.fragment_links, links);
}

/// A bad link is a request error with its path, and no pipeline is made.
#[sqlx::test(migrations = false)]
async fn create_pipeline_refuses_bad_links(pool: PgPool) {
    // given
    let app = app(pool).await;
    let owner = app.user().await;
    let project = text(&app.create_project(owner, "web").await.unwrap().id).to_owned();
    let link = |block: &str, nodes: &[&str]| fragment_link(block, UNKNOWN_ID, 1, nodes);
    let mut zero = link("ci", &["build"]);
    zero.version = 0;
    let mut bad_value = link("ci", &["build"]);
    bad_value.values.insert("1BAD".to_owned(), String::new());
    let mut no_fragment = link("ci", &["build"]);
    no_fragment.fragment_id = None;
    let cases = [
        (
            "fragment_links[0].block_id",
            vec![link("build", &["build"])],
        ),
        (
            "fragment_links[1].block_id",
            vec![link("ci", &["build"]), link("ci", &["test"])],
        ),
        (
            "fragment_links[0].node_ids[0]",
            vec![link("ci", &["deploy"])],
        ),
        (
            "fragment_links[1].node_ids[0]",
            vec![link("ci", &["build"]), link("cd", &["build"])],
        ),
        ("fragment_links[0].node_ids", vec![link("ci", &[])]),
        ("fragment_links[0].version", vec![zero]),
        ("fragment_links[0].values", vec![bad_value]),
        ("fragment_links[0].fragment_id", vec![no_fragment]),
        (
            "fragment_links[0].block_id",
            vec![link("Not An Id", &["build"])],
        ),
    ];

    // when
    for (field, links) in cases {
        let nodes = vec![exec_node("build", &[]), exec_node("test", &[])];
        let status = app
            .create_linked_pipeline(owner, &project, nodes, links)
            .await
            .unwrap_err();

        // then
        assert_invalid_field(&status, field);
    }
}

/// A change of the nodes alone keeps the links and checks them again: a node that a block holds
/// cannot go. A change of the links alone replaces them.
#[sqlx::test(migrations = false)]
async fn update_pipeline_checks_the_kept_links(pool: PgPool) {
    // given
    let app = app(pool).await;
    let owner = app.user().await;
    let project = text(&app.create_project(owner, "web").await.unwrap().id).to_owned();
    let nodes = vec![exec_node("build", &[]), exec_node("test", &[])];
    let links = vec![fragment_link("ci", UNKNOWN_ID, 1, &["build"])];
    let pipeline = app
        .create_linked_pipeline(owner, &project, nodes, links)
        .await
        .unwrap();
    let id = text(&pipeline.id);

    // when
    let removes_held = app
        .update_pipeline_links(
            owner,
            id,
            pipeline.version,
            Some(vec![exec_node("test", &[])]),
            None,
        )
        .await;
    let keeps = app
        .update_pipeline_links(
            owner,
            id,
            pipeline.version,
            Some(vec![exec_node("build", &[])]),
            None,
        )
        .await
        .unwrap();
    let cleared = app
        .update_pipeline_links(owner, id, keeps.version, None, Some(Vec::new()))
        .await
        .unwrap();

    // then
    assert_invalid_field(&removes_held.unwrap_err(), "fragment_links[0].node_ids[0]");
    assert_eq!(keeps.fragment_links.len(), 1);
    assert!(cleared.fragment_links.is_empty());
    assert_eq!(cleared.nodes.len(), 1);
}
