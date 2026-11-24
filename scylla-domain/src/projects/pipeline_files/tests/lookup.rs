//! Tests of the lookup of the pipeline files in the files of a commit.

use crate::projects::pipeline_files::{
    lookup, FileFormat, Lookup, TreeEntry, MAX_BYTES, MAX_FILES,
};

/// Makes the files of a commit, each with a small size.
fn tree(paths: &[&str]) -> Vec<TreeEntry> {
    paths
        .iter()
        .map(|path| TreeEntry {
            path: (*path).to_owned(),
            size: Some(100),
        })
        .collect()
}

/// The keys and paths of the files to read, and the paths and keys of the refusals.
type Summary = (Vec<(String, String)>, Vec<(String, Option<String>)>);

/// Returns the summary of a lookup.
fn summary(lookup: &Lookup) -> Summary {
    let files = lookup
        .files
        .iter()
        .map(|file| (file.key.as_str().to_owned(), file.path.clone()))
        .collect();
    let rejected = lookup
        .rejected
        .iter()
        .map(|rejected| {
            let key = rejected.key.as_ref().map(|key| key.as_str().to_owned());
            (rejected.error.path.clone(), key)
        })
        .collect();
    (files, rejected)
}

/// Each file of `.scylla/pipelines` with a known extension is a pipeline, and other files are
/// ignored.
#[test]
fn each_file_of_the_folder_is_a_pipeline() {
    // given
    let entries = tree(&[
        ".scylla/pipelines/deploy.yml",
        ".scylla/pipelines/build.toml",
        ".scylla/pipelines/README.md",
        ".scylla/pipelines/old/test.toml",
        ".scylla/settings.toml",
        "pipeline.toml",
        "src/main.rs",
    ]);

    // when
    let found = lookup(&entries);

    // then
    let (files, rejected) = summary(&found);
    let expected = [
        ("build", ".scylla/pipelines/build.toml"),
        ("deploy", ".scylla/pipelines/deploy.yml"),
    ];
    assert_eq!(
        files,
        expected.map(|(key, path)| (key.to_owned(), path.to_owned()))
    );
    assert!(rejected.is_empty());
    assert_eq!(found.files[1].format, FileFormat::Yaml);
}

/// With no folder, the file `.scylla/pipeline.<ext>` is the pipeline `pipeline`.
#[test]
fn the_one_file_is_the_pipeline_named_pipeline() {
    // given
    let entries = tree(&[".scylla/pipeline.json", "README.md"]);

    // when
    let found = lookup(&entries);

    // then
    let (files, _) = summary(&found);
    assert_eq!(
        files,
        [("pipeline".to_owned(), ".scylla/pipeline.json".to_owned())]
    );
    assert_eq!(found.files[0].format, FileFormat::Json);
}

/// Two files for one key, and a file at the root next to the folder, are errors: no silent
/// choice.
#[test]
fn two_answers_are_an_error() {
    // given
    let in_folder = tree(&[
        ".scylla/pipelines/build.yml",
        ".scylla/pipelines/build.toml",
        ".scylla/pipelines/test.json",
        ".scylla/pipeline.toml",
    ]);
    let at_root = tree(&[".scylla/pipeline.yaml", ".scylla/pipeline.toml"]);

    // when
    let found = [lookup(&in_folder), lookup(&at_root)];

    // then
    let (files, rejected) = summary(&found[0]);
    assert_eq!(
        files,
        [("test".to_owned(), ".scylla/pipelines/test.json".to_owned())]
    );
    let expected = [
        (".scylla/pipeline.toml", None),
        (".scylla/pipelines/build.toml", Some("build")),
    ];
    let expected = expected.map(|(path, key)| (path.to_owned(), key.map(str::to_owned)));
    assert_eq!(rejected, expected);
    let message = &found[0].rejected[1].error.message;
    assert_eq!(
        message,
        "two definitions of \"build\": .scylla/pipelines/build.toml, .scylla/pipelines/build.yml"
    );
    let (files, rejected) = summary(&found[1]);
    assert!(files.is_empty());
    assert_eq!(
        rejected,
        [(
            ".scylla/pipeline.toml".to_owned(),
            Some("pipeline".to_owned())
        )]
    );
}

/// A name that is not a key, a file above 1 MiB, and the files after the 32nd are refused.
#[test]
fn the_limits_of_the_lookup_are_refused() {
    // given
    let mut entries = tree(&[".scylla/pipelines/Build.toml"]);
    entries.push(TreeEntry {
        path: ".scylla/pipelines/big.toml".to_owned(),
        size: Some(MAX_BYTES as u64 + 1),
    });
    let names: Vec<_> = (0..MAX_FILES)
        .map(|index| format!(".scylla/pipelines/p{index:02}.toml"))
        .collect();
    entries.extend(tree(&names.iter().map(String::as_str).collect::<Vec<_>>()));

    // when
    let found = lookup(&entries);

    // then
    let (files, rejected) = summary(&found);
    assert_eq!(files.len(), MAX_FILES - 1);
    let expected = [
        (".scylla/pipelines/Build.toml", None),
        (".scylla/pipelines/big.toml", Some("big")),
        (".scylla/pipelines/p31.toml", Some("p31")),
    ];
    let expected = expected.map(|(path, key)| (path.to_owned(), key.map(str::to_owned)));
    assert_eq!(rejected, expected);
}
