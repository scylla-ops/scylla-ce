//! Unit tests of the labels of the agents: the form of a label, the list of an App or of a
//! pipeline, the automatic labels of a host, and the fit of an agent for a job.

use super::host;
use crate::identity::agents::{fits, AgentLabel, AgentLabels};
use crate::DomainError;

/// Returns the texts as owned strings.
fn texts(labels: &[&str]) -> Vec<String> {
    labels.iter().map(|label| (*label).to_owned()).collect()
}

/// A label has 1 to 32 lower case letters, digits, dots, dashes or underscores, and starts with a
/// letter or a digit. The spaces around it go.
#[test]
fn a_label_has_its_form() {
    // given
    let longest = "a".repeat(AgentLabel::MAX_LEN);
    let too_long = "a".repeat(AgentLabel::MAX_LEN + 1);

    // when
    let good = [
        "deploy",
        "0.7",
        "gpu-a100",
        "eu_west.2",
        longest.as_str(),
        " prod ",
    ]
    .map(AgentLabel::parse);
    let bad = [
        "",
        "Deploy",
        "-x",
        ".x",
        "_x",
        "a b",
        "été",
        too_long.as_str(),
    ]
    .map(AgentLabel::parse);

    // then
    assert!(good.iter().all(Result::is_ok), "{good:?}");
    assert_eq!(good[5].as_ref().unwrap().as_str(), "prod");
    assert!(bad.iter().all(|one| *one == Err(DomainError::NotALabel)));
}

/// A list has at most 16 labels, each one time, and a refusal names the bad label.
#[test]
fn a_list_of_labels_is_checked() {
    // given
    let full: Vec<String> = (0..AgentLabels::MAX).map(|n| format!("l{n}")).collect();
    let mut over = full.clone();
    over.push("more".to_owned());

    // when
    let kept = AgentLabels::parse(texts(&["deploy", "linux"])).unwrap();
    let too_many = AgentLabels::parse(over);
    let twice = AgentLabels::parse(texts(&["deploy", "linux", "deploy"]));
    let bad = AgentLabels::parse(texts(&["deploy", "Linux"]));

    // then
    assert_eq!(kept.texts(), ["deploy", "linux"]);
    assert!(AgentLabels::parse(full).is_ok());
    assert_eq!(too_many, Err((None, DomainError::TooMany { max: 16 })));
    assert_eq!(twice, Err((Some(2), DomainError::Duplicate)));
    assert_eq!(bad, Err((Some(1), DomainError::NotALabel)));
}

/// The host gives the system, the architecture in its short name, and `container` with a
/// container runtime.
#[test]
fn the_host_gives_the_automatic_labels() {
    // given
    let linux = host(2, &[]);
    let mut mac = host(2, &[]);
    mac.os = "macos".to_owned();
    mac.arch = "aarch64".to_owned();
    mac.container_runtime = Some("docker".to_owned());
    let mut windows = host(2, &[]);
    windows.os = "windows".to_owned();
    windows.arch = String::new();

    // when
    let labels = [linux, mac, windows].map(|one| one.automatic_labels().texts());

    // then
    assert_eq!(labels[0], ["linux", "x64"]);
    assert_eq!(labels[1], ["macos", "arm64", "container"]);
    assert_eq!(labels[2], ["windows"]);
}

/// An agent fits a job when it has each label of the job, from its host or from the admins. A
/// job with no label fits each agent.
#[test]
fn an_agent_fits_a_job_with_each_label() {
    // given
    let automatic = AgentLabels::parse(texts(&["linux", "x64"])).unwrap();
    let given = AgentLabels::parse(texts(&["deploy"])).unwrap();
    let asked = |labels: &[&str]| AgentLabels::parse(texts(labels)).unwrap();

    // when
    let both = fits(&asked(&["deploy", "linux"]), &automatic, &given);
    let none = fits(&asked(&[]), &automatic, &given);
    let missing = fits(&asked(&["deploy", "windows"]), &automatic, &given);

    // then
    assert!(both);
    assert!(none);
    assert!(!missing);
}
