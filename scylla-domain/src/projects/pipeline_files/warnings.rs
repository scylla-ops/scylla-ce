//! The warnings of a checked pipeline file: the secrets that the nodes name and that the project
//! does not have. The file stays valid, and a run of it fails.

use super::positions::Positions;
use super::{FileError, FileFormat, FilePipeline};
use crate::projects::pipelines::EnvValue;

impl FilePipeline {
    /// Returns the names of the secrets that the nodes name, each one time, in the order of the
    /// file.
    pub fn secret_names(&self) -> Vec<String> {
        let mut names: Vec<String> = Vec::new();
        for var in self.nodes.as_slice().iter().flat_map(|node| &node.env) {
            if let EnvValue::Secret(name) = &var.value {
                if !names.iter().any(|known| known == name.as_str()) {
                    names.push(name.as_str().to_owned());
                }
            }
        }
        names
    }
}

/// Returns a warning for each variable of a node that names a secret that is not in `known`, at
/// the place of its value in the text of the file at `path`.
pub fn missing_secrets(
    path: &str,
    format: FileFormat,
    text: &str,
    pipeline: &FilePipeline,
    known: &[String],
) -> Vec<FileError> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let positions = Positions::of(format, text);
    let mut warnings = Vec::new();
    for (index, node) in pipeline.nodes.as_slice().iter().enumerate() {
        for (at, var) in node.env.iter().enumerate() {
            let EnvValue::Secret(name) = &var.value else {
                continue;
            };
            if known.iter().any(|known| known == name.as_str()) {
                continue;
            }
            let field = format!("nodes[{index}].env[{at}].value");
            warnings.push(FileError {
                path: path.to_owned(),
                location: positions.find(&field),
                field,
                message: format!(
                    "names the secret \"{}\", which the project does not have",
                    name.as_str()
                ),
            });
        }
    }
    warnings
}
