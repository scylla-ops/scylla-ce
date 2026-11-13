//! The calls of the merge blocks to GitHub: the rulesets of one repository, with an
//! installation token of the repository.

use reqwest::Method;
use serde::Deserialize;
use serde_json::Value;

use crate::features::integrations::merge_blocks::rules::{Ruleset, RULESET_NAME};
use crate::infra::vcs::{Auth, Call, Hidden, ProviderError, VcsClients};

/// The most items of one page of GitHub.
const PAGE: u32 = 100;

/// The rulesets of one repository.
pub(crate) struct Rulesets<'a> {
    /// The clients of the providers.
    pub(crate) clients: &'a VcsClients,
    /// An installation token of the repository.
    pub(crate) token: &'a Hidden,
    /// The full name of the repository, for example `acme/api`.
    pub(crate) full_name: &'a str,
}

/// A ruleset in the list of a repository.
#[derive(Deserialize)]
struct Listed {
    /// The id.
    id: i64,
    /// The name.
    name: String,
    /// `Repository` for a ruleset of the repository.
    source_type: Option<String>,
}

/// A ruleset that GitHub made.
#[derive(Deserialize)]
struct Created {
    /// The id.
    id: i64,
}

/// A ruleset, as GitHub gives it.
#[derive(Deserialize)]
struct Answer {
    /// `active`, `evaluate` or `disabled`.
    enforcement: String,
    /// The branches of the ruleset.
    conditions: Option<Conditions>,
    /// The rules.
    #[serde(default)]
    rules: Vec<Rule>,
}

/// The conditions of a ruleset.
#[derive(Deserialize)]
struct Conditions {
    /// The branch names.
    ref_name: Option<RefName>,
}

/// The branch names of a ruleset.
#[derive(Deserialize)]
struct RefName {
    /// The names and patterns of the branches.
    #[serde(default)]
    include: Vec<String>,
}

/// A rule of a ruleset.
#[derive(Deserialize)]
struct Rule {
    /// The kind, for example `required_status_checks`.
    #[serde(rename = "type")]
    kind: String,
    /// The parameters of the rule.
    parameters: Option<Parameters>,
}

/// The parameters of a rule of required status checks.
#[derive(Deserialize)]
struct Parameters {
    /// The required checks.
    #[serde(default)]
    required_status_checks: Vec<Check>,
}

/// A required check.
#[derive(Deserialize)]
struct Check {
    /// The context of the check, for example `scylla/build`.
    context: String,
}

impl Rulesets<'_> {
    /// Returns the id of the ruleset of Scylla in the repository, found by its name, or `None`.
    pub(crate) async fn find(&self) -> Result<Option<i64>, ProviderError> {
        let listed: Vec<Listed> = self
            .call(Method::GET, None)
            .query("includes_parents", false)
            .query("per_page", PAGE)
            .list(None)
            .await?;
        Ok(listed
            .into_iter()
            .find(|ruleset| {
                ruleset.name == RULESET_NAME
                    && ruleset.source_type.as_deref().unwrap_or("Repository") == "Repository"
            })
            .map(|ruleset| ruleset.id))
    }

    /// Returns a ruleset, or `None` when the repository does not have it.
    pub(crate) async fn read(&self, id: i64) -> Result<Option<Ruleset>, ProviderError> {
        let answer: Answer = match self.call(Method::GET, Some(id)).read().await {
            Ok(answer) => answer,
            Err(ProviderError::NotFound) => return Ok(None),
            Err(error) => return Err(error),
        };
        let include = answer
            .conditions
            .and_then(|conditions| conditions.ref_name)
            .map(|names| names.include)
            .unwrap_or_default();
        let contexts = answer
            .rules
            .into_iter()
            .filter(|rule| rule.kind == "required_status_checks")
            .filter_map(|rule| rule.parameters)
            .flat_map(|parameters| parameters.required_status_checks)
            .map(|check| check.context)
            .collect();
        Ok(Some(Ruleset {
            enforcement: answer.enforcement,
            include,
            contexts,
        }))
    }

    /// Makes a ruleset, and returns its id.
    pub(crate) async fn create(&self, body: Value) -> Result<i64, ProviderError> {
        let created: Created = self.call(Method::POST, None).json(body).read().await?;
        Ok(created.id)
    }

    /// Replaces a ruleset.
    pub(crate) async fn update(&self, id: i64, body: Value) -> Result<(), ProviderError> {
        let _: Value = self.call(Method::PUT, Some(id)).json(body).read().await?;
        Ok(())
    }

    /// Deletes a ruleset. A ruleset that is not there is a success.
    pub(crate) async fn delete(&self, id: i64) -> Result<(), ProviderError> {
        match self.call(Method::DELETE, Some(id)).read::<Value>().await {
            Ok(_) | Err(ProviderError::NotFound) => Ok(()),
            Err(error) => Err(error),
        }
    }

    /// Starts a call on the rulesets of the repository, or on one ruleset.
    fn call(&self, method: Method, id: Option<i64>) -> Call<'_> {
        let path = match id {
            Some(id) => format!("/repos/{}/rulesets/{id}", self.full_name),
            None => format!("/repos/{}/rulesets", self.full_name),
        };
        self.clients
            .github
            .call(method, &path)
            .auth(Auth::Bearer(self.token))
    }
}
