//! The limits of an API token on its short tokens: a block of checks after the first block. The
//! holder cannot remove a block, so each request of the short token passes these checks too.

use std::collections::{BTreeSet, HashMap};

use biscuit_auth::builder::{BlockBuilder, Term};
use biscuit_auth::UnverifiedBiscuit;
use scylla_domain::identity::api_tokens::{
    TokenLimits, TokenPreset, DEPLOY_METHODS, READ_PREFIXES,
};
use scylla_extension::AccessError;

/// Adds the checks of the limits to a token. Limits with no check give the token back.
pub fn attenuate(token: &str, limits: &TokenLimits) -> Result<String, AccessError> {
    let (code, params) = checks(limits);
    if code.is_empty() {
        return Ok(token.to_owned());
    }
    BlockBuilder::new()
        .code_with_params(&code, params, HashMap::new())
        .and_then(|block| UnverifiedBiscuit::from_base64(token)?.append(block))
        .and_then(|token| token.to_base64())
        .map_err(|error| AccessError::Unavailable(format!("token limits failed: {error}")))
}

/// Makes the Datalog checks of limits and their parameters.
fn checks(limits: &TokenLimits) -> (String, HashMap<String, Term>) {
    let reads = READ_PREFIXES
        .iter()
        .map(|prefix| format!("$op.contains(\"/{prefix}\")"))
        .collect::<Vec<_>>()
        .join(" || ");
    let mut params = HashMap::new();
    let mut code = match limits.preset {
        TokenPreset::Full => String::new(),
        TokenPreset::ReadOnly => format!("check if operation($op), {reads};"),
        TokenPreset::Deploy => {
            params.insert("deploy".to_owned(), set(DEPLOY_METHODS));
            format!("check if operation($op), {reads} || {{deploy}}.contains($op);")
        }
        TokenPreset::Custom => {
            params.insert("actions".to_owned(), set(&limits.actions));
            "check if operation($op), {actions}.contains($op);".to_owned()
        }
    };
    if !limits.projects.is_empty() {
        let projects = limits.projects.iter().map(ToString::to_string);
        params.insert("projects".to_owned(), set(projects));
        code.push_str(" check if resource(\"project\", $id), {projects}.contains($id);");
    }
    (code, params)
}

/// Makes a set of texts.
fn set<T: ToString>(texts: impl IntoIterator<Item = T>) -> Term {
    let texts = texts.into_iter().map(|text| Term::Str(text.to_string()));
    Term::Set(texts.collect::<BTreeSet<_>>())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn full_limits_add_no_check() {
        let limits = TokenLimits {
            preset: TokenPreset::Full,
            ..TokenLimits::default()
        };

        let (code, _) = checks(&limits);

        assert!(code.is_empty());
    }

    #[test]
    fn read_only_limits_check_the_names_of_the_methods() {
        let (code, _) = checks(&TokenLimits::default());

        assert!(code.starts_with("check if operation($op), $op.contains(\"/Get\") ||"));
    }
}
