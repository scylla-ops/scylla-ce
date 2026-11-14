//! The `TestApp` helpers of the pipelines from the files of a repository: the source of the
//! pipelines of a project, the read of a file pipeline and of the files at a commit, and the
//! answers of a fake GitHub or GitLab with pipeline files.

use std::hash::{DefaultHasher, Hash, Hasher};

use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use scylla_domain::identity::actors::ActorId;
use scylla_proto::integrations::v1::PipelineSource;
use scylla_proto::pipelines::v1::{
    git_ref, GetFilePipelineRequest, GitRef, ListPipelineFilesRequest, ListPipelineFilesResponse,
    PipelineFile, SetPipelineSourceRequest,
};
use serde_json::{json, Value};
use tonic::Status;

use super::FakeProvider;
use crate::{id, TestApp};

/// The folder of the pipeline files.
const ROOT: &str = ".scylla";

/// The folder of many pipeline files.
const FOLDER: &str = ".scylla/pipelines";

impl TestApp {
    /// Calls `SetPipelineSource` as `actor` for a project.
    pub async fn set_pipeline_source(
        &self,
        actor: ActorId,
        project: &str,
        source: PipelineSource,
    ) -> Result<(), Status> {
        let request = SetPipelineSourceRequest {
            project_id: Some(id(project)),
            source: source.into(),
        };
        let request = self.authed(actor, request).await;
        self.pipelines().set_pipeline_source(request).await?;
        Ok(())
    }

    /// Calls `GetFilePipeline` as `actor` at a branch, a tag or a SHA, or at the head of the
    /// default branch with no ref, and returns the file.
    pub async fn get_file_pipeline(
        &self,
        actor: ActorId,
        pipeline: &str,
        at: Option<git_ref::Kind>,
    ) -> Result<PipelineFile, Status> {
        let request = GetFilePipelineRequest {
            pipeline_id: Some(id(pipeline)),
            r#ref: at.map(|kind| GitRef { kind: Some(kind) }),
        };
        let request = self.authed(actor, request).await;
        let response = self.pipelines().get_file_pipeline(request).await?;
        Ok(response.into_inner().file.expect("file in response"))
    }

    /// Calls `ListPipelineFiles` as `actor` at a branch, a tag or a SHA, or at the head of the
    /// default branch with no ref.
    pub async fn list_pipeline_files(
        &self,
        actor: ActorId,
        project: &str,
        at: Option<git_ref::Kind>,
    ) -> Result<ListPipelineFilesResponse, Status> {
        let request = ListPipelineFilesRequest {
            project_id: Some(id(project)),
            r#ref: at.map(|kind| GitRef { kind: Some(kind) }),
        };
        let request = self.authed(actor, request).await;
        Ok(self
            .pipelines()
            .list_pipeline_files(request)
            .await?
            .into_inner())
    }
}

impl FakeProvider {
    /// Programs a fake GitHub for the repository `octo/app` of the installation 42, with the
    /// token `ghs_test`: the commit `sha` at `at` (`heads/<branch>`, `tags/<tag>` or a SHA), and
    /// the pipeline files at this commit, a path and a text for each file. Each call adds one
    /// answer for `at`, so the calls give the commits of `at` in order.
    pub fn github_files(&self, at: &str, sha: &str, files: &[(&str, &str)]) {
        // Give the token and the commit.
        let token = json!({ "token": "ghs_test", "expires_at": "2099-01-01T00:00:00Z" });
        self.respond(
            "POST",
            "/app/installations/42/access_tokens",
            201,
            token,
            &[],
        );
        let commit = json!({
            "sha": sha,
            "commit": { "message": "Change the pipelines" },
            "author": { "login": "octocat", "avatar_url": "https://avatars.example/u/1" },
        });
        let path = format!("/repos/octo/app/commits/{at}");
        self.respond("GET", &path, 200, commit, &[]);

        // Give the folders and the blobs.
        let item = |path: &str, text: &str| json!({ "path": path, "sha": blob_sha(text), "size": text.len(), "type": "file" });
        let mut root: Vec<Value> = files
            .iter()
            .filter(|(path, _)| folder_of(path) == ROOT)
            .map(|(path, text)| item(path, text))
            .collect();
        let many: Vec<Value> = files
            .iter()
            .filter(|(path, _)| folder_of(path) == FOLDER)
            .map(|(path, text)| item(path, text))
            .collect();
        if !many.is_empty() {
            root.push(json!({ "path": FOLDER, "sha": "0", "size": 0, "type": "dir" }));
            let folder = format!("/repos/octo/app/contents/{FOLDER}?ref={sha}");
            self.respond("GET", &folder, 200, Value::Array(many), &[]);
        }
        if !root.is_empty() {
            let folder = format!("/repos/octo/app/contents/{ROOT}?ref={sha}");
            self.respond("GET", &folder, 200, Value::Array(root), &[]);
        }
        for (_, text) in files {
            let path = format!("/repos/octo/app/git/blobs/{}", blob_sha(text));
            self.respond("GET", &path, 200, blob(text), &[]);
        }
    }

    /// Programs a fake GitLab for the project 7: the commit `sha` at `at` (a branch, a tag or a
    /// SHA), and the pipeline files at this commit, a path and a text for each file. Each call
    /// adds one answer for `at`, so the calls give the commits of `at` in order.
    pub fn gitlab_files(&self, at: &str, sha: &str, files: &[(&str, &str)]) {
        // Give the commit.
        let commit = json!({ "id": sha, "message": "Change the pipelines" });
        let path = format!("/api/v4/projects/7/repository/commits/{at}");
        self.respond("GET", &path, 200, commit, &[]);

        // Give the tree and the blobs.
        let mut tree: Vec<Value> = files
            .iter()
            .map(|(path, text)| json!({ "id": blob_sha(text), "path": path, "type": "blob" }))
            .collect();
        if files.iter().any(|(path, _)| folder_of(path) == FOLDER) {
            tree.push(json!({ "id": "0", "path": FOLDER, "type": "tree" }));
        }
        let query = format!("path=.scylla&ref={sha}&recursive=true&per_page=100");
        let path = format!("/api/v4/projects/7/repository/tree?{query}");
        self.respond("GET", &path, 200, Value::Array(tree), &[]);
        for (_, text) in files {
            let path = format!("/api/v4/projects/7/repository/blobs/{}", blob_sha(text));
            self.respond("GET", &path, 200, blob(text), &[]);
        }
    }
}

/// Returns the SHA of the blob of a text in the fakes: 40 hexadecimal characters.
fn blob_sha(text: &str) -> String {
    let hash = |salt: u8| {
        let mut hasher = DefaultHasher::new();
        (salt, text).hash(&mut hasher);
        hasher.finish()
    };
    format!("{:016x}{:016x}{:08x}", hash(1), hash(2), text.len())
}

/// Returns the answer of a blob of a text, in base64 with a line break as the providers give it.
fn blob(text: &str) -> Value {
    let encoded = STANDARD.encode(text);
    let (head, tail) = encoded.split_at(encoded.len() / 2);
    json!({ "content": format!("{head}\n{tail}"), "encoding": "base64", "size": text.len() })
}

/// Returns the folder of a path.
fn folder_of(path: &str) -> &str {
    path.rsplit_once('/').map_or("", |(folder, _)| folder)
}
