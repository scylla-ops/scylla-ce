//! The bare repository of the tests of the checkout, and what the agent sent.

use std::path::{Path, PathBuf};
use std::process::Command;

use scylla_proto::agents::v1::open_stream_request::Message as Up;
use scylla_proto::agents::v1::{DispatchCheckout, EventKind, OpenStreamRequest};
use tokio::sync::mpsc;

/// The header of the tests, with its credential.
pub(super) const HEADER: &str = "Authorization: Bearer s3cr3t-token";

/// A bare repository with three commits: the first two on `main`, the third only on
/// `refs/pull/1/head`.
pub(super) struct Origin {
    /// The folder of the test.
    pub(super) root: PathBuf,
    /// The bare repository.
    pub(super) url: String,
    /// The commits, in order.
    pub(super) shas: Vec<String>,
}

impl Origin {
    /// Makes the bare repository in a new folder.
    pub(super) fn new() -> Self {
        let mut bytes = [0u8; 8];
        getrandom::fill(&mut bytes).expect("the system gives random bytes");
        let root = std::env::temp_dir().join(format!("scylla-checkout-{}", hex::encode(bytes)));
        let source = root.join("source");
        std::fs::create_dir_all(source.join(".scylla")).unwrap();
        git(&root, &["init", "--quiet", "--bare", "origin.git"]);
        git(&source, &["init", "--quiet"]);
        let mut shas = Vec::new();
        for file in [".scylla/pipeline.toml", "b.txt", "c.txt"] {
            std::fs::write(source.join(file), file).unwrap();
            git(&source, &["add", "."]);
            git(&source, &["commit", "--quiet", "-m", file]);
            shas.push(git(&source, &["rev-parse", "HEAD"]));
        }
        let main = format!("{}:refs/heads/main", shas[1]);
        let pull = "HEAD:refs/pull/1/head";
        git(&source, &["push", "--quiet", "../origin.git", &main, pull]);
        let url = root.join("origin.git").display().to_string();
        Self { root, url, shas }
    }

    /// Makes an empty work folder, with the folder of the scripts as the agent makes it.
    pub(super) fn work(&self) -> PathBuf {
        let dir = self.root.join("work");
        std::fs::create_dir_all(dir.join(".scylla")).unwrap();
        dir
    }

    /// Makes a checkout of a commit of this repository.
    pub(super) fn checkout(&self, sha: usize, fetch_ref: &str, depth: u32) -> DispatchCheckout {
        DispatchCheckout {
            clone_url: self.url.clone(),
            sha: self.shas[sha].clone(),
            fetch_ref: fetch_ref.to_owned(),
            auth_header: HEADER.to_owned(),
            depth,
            ..DispatchCheckout::default()
        }
    }
}

impl Drop for Origin {
    /// Deletes the folder of the test.
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

/// Runs a git command of the test and returns its output.
pub(super) fn git(dir: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .args(["-c", "user.name=test", "-c", "user.email=test@example.com"])
        .args(["-c", "init.defaultBranch=main", "-c", "core.autocrlf=false"])
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap();
    assert!(output.status.success(), "git {args:?}: {output:?}");
    String::from_utf8_lossy(&output.stdout).trim().to_owned()
}

/// The events that the agent sent, and its lines of one node.
pub(super) fn sent(
    inbox: &mut mpsc::Receiver<OpenStreamRequest>,
    node: &str,
) -> (Vec<EventKind>, Vec<String>) {
    let (mut kinds, mut lines) = (Vec::new(), Vec::new());
    while let Ok(message) = inbox.try_recv() {
        match message.message {
            Some(Up::Event(event)) => kinds.push(event.kind()),
            Some(Up::Logs(batch)) if batch.node_id == node => {
                lines.extend(batch.lines.into_iter().map(|line| line.text));
            }
            _ => {}
        }
    }
    (kinds, lines)
}
