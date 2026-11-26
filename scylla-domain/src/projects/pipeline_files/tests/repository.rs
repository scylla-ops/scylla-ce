//! Tests of the pipeline files of this repository: they read as the server reads them.

use std::path::Path;

use crate::projects::pipeline_files::{lookup, parse, TreeEntry};

/// The labels of the platforms in the automatic labels of an agent.
const PLATFORMS: [&str; 3] = ["linux", "macos", "windows"];

/// Each pipeline file of this repository reads with no error and asks the label of one platform,
/// so an agent of another platform never takes its jobs.
#[test]
fn each_file_of_this_repository_asks_one_platform() {
    // given
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let entries: Vec<TreeEntry> = std::fs::read_dir(root.join(".scylla/pipelines"))
        .unwrap()
        .map(|entry| TreeEntry {
            path: format!(".scylla/pipelines/{}", entry.unwrap().file_name().display()),
            size: None,
        })
        .collect();

    // when
    let found = lookup(&entries);
    let platforms: Vec<(String, Vec<String>)> = found
        .files
        .iter()
        .map(|file| {
            let text = std::fs::read_to_string(root.join(&file.path)).unwrap();
            let pipeline = parse(&file.path, &file.key, file.format, &text)
                .unwrap_or_else(|errors| panic!("{}: {errors:?}", file.path));
            let labels = pipeline.agent_labels.texts();
            let asked = labels
                .into_iter()
                .filter(|label| PLATFORMS.contains(&label.as_str()))
                .collect();
            (file.path.clone(), asked)
        })
        .collect();

    // then
    assert!(found.rejected.is_empty(), "{:?}", found.rejected);
    assert!(!platforms.is_empty());
    for (path, asked) in platforms {
        assert_eq!(asked.len(), 1, "{path} asks the platforms {asked:?}");
    }
}
