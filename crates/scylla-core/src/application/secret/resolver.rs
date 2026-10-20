use crate::application::agent::dispatch::{DispatchEnv, DispatchNode};
use crate::application::secret::SecretCipher;
use crate::application::secret::repository::SecretRepository;
use crate::domain::errors::{DomainError, DomainResult};
use crate::domain::ids::ProjectId;
use crate::domain::pipeline::EnvSource;
use crate::domain::pipeline::PipelineNode;
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::Arc;

#[async_trait]
pub trait SecretResolver: Send + Sync {
    async fn resolve(
        &self,
        project_id: &ProjectId,
        nodes: &[PipelineNode],
    ) -> DomainResult<Vec<DispatchNode>>;
}

pub struct DispatchSecretResolver {
    secret_repo: Arc<dyn SecretRepository>,
    cipher: Arc<dyn SecretCipher>,
}

impl DispatchSecretResolver {
    #[must_use]
    pub fn new(secret_repo: Arc<dyn SecretRepository>, cipher: Arc<dyn SecretCipher>) -> Self {
        Self {
            secret_repo,
            cipher,
        }
    }
}

#[async_trait]
impl SecretResolver for DispatchSecretResolver {
    async fn resolve(
        &self,
        project_id: &ProjectId,
        nodes: &[PipelineNode],
    ) -> DomainResult<Vec<DispatchNode>> {
        let needs_secrets = nodes
            .iter()
            .flat_map(PipelineNode::env)
            .any(|e| matches!(e.source(), EnvSource::Secret(_)));

        let by_name: HashMap<String, Vec<u8>> = if needs_secrets {
            self.secret_repo
                .list_by_project(project_id)
                .await?
                .into_iter()
                .map(|s| (s.name().as_str().to_string(), s.encrypted_value().to_vec()))
                .collect()
        } else {
            HashMap::new()
        };

        let mut out = Vec::with_capacity(nodes.len());
        for node in nodes {
            let mut env = Vec::with_capacity(node.env().len());
            for ev in node.env() {
                match ev.source() {
                    EnvSource::Literal(value) => env.push(DispatchEnv {
                        key: ev.key().to_string(),
                        value: value.to_string(),
                        masked: false,
                    }),
                    EnvSource::Secret(name) => {
                        let ciphertext = by_name
                            .get(name.as_str())
                            .ok_or_else(|| {
                                DomainError::NotFound(format!(
                                    "secret '{name}' referenced by node '{}' env '{}' is not in this project",
                                    node.id(),
                                    ev.key()
                                ))
                            })?;
                        let value = self.cipher.decrypt(ciphertext)?;
                        env.push(DispatchEnv {
                            key: ev.key().to_string(),
                            value,
                            masked: true,
                        });
                    }
                }
            }
            out.push(DispatchNode {
                id: node.id().to_string(),
                deps: node.deps().iter().map(ToString::to_string).collect(),
                working_dir: node.working_dir().map(|w| w.as_str().to_string()),
                step: node.step().clone(),
                env,
            });
        }
        Ok(out)
    }
}
