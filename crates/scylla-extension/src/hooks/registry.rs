use super::extension::Extension;
use super::next::{Done, Next, Proceed};
use super::position::{Around, Gate, Listener, Observer, Policy, Wrap};
use crate::action::{Action, Phase};
use crate::domain::errors::{DomainError, DomainResult};
use crate::stage::{Run, Stage, StageKind};
use std::any::{Any, TypeId};
use std::collections::HashMap;
use std::sync::Arc;

/// Order within a position is registration order; between extensions it is the order of `with`.
#[derive(Default)]
pub struct Hooks {
    policies: HashMap<StageKind, Vec<Arc<dyn Policy>>>,
    arounds: HashMap<StageKind, Vec<Arc<dyn Around>>>,
    observers: HashMap<StageKind, Vec<Arc<dyn Observer>>>,
    typed: HashMap<TypeId, Box<dyn Any + Send + Sync>>,
}

impl Hooks {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn with<E: Extension>(mut self, extension: &Arc<E>) -> Self {
        extension.register(&mut self);
        self
    }

    pub fn policy(&mut self, stage: StageKind, hook: Arc<dyn Policy>) -> &mut Self {
        self.policies.entry(stage).or_default().push(hook);
        self
    }

    pub fn gate<S: Stage>(&mut self, hook: Arc<dyn Gate<S>>) -> &mut Self {
        self.slot().push(hook);
        self
    }

    pub fn around(&mut self, stage: StageKind, hook: Arc<dyn Around>) -> &mut Self {
        self.arounds.entry(stage).or_default().push(hook);
        self
    }

    pub fn wrap<S: Stage>(&mut self, hook: Arc<dyn Wrap<S>>) -> &mut Self {
        self.slot().push(hook);
        self
    }

    pub fn listen<S: Stage>(&mut self, hook: Arc<dyn Listener<S>>) -> &mut Self {
        self.slot().push(hook);
        self
    }

    pub fn observe(&mut self, stage: StageKind, hook: Arc<dyn Observer>) -> &mut Self {
        self.observers.entry(stage).or_default().push(hook);
        self
    }

    // A slot is keyed by its own element type, so the downcast cannot fail.
    fn slot<T: Any + Send + Sync>(&mut self) -> &mut Vec<T> {
        self.typed
            .entry(TypeId::of::<T>())
            .or_insert_with(|| Box::new(Vec::<T>::new()))
            .downcast_mut()
            .expect("slot keyed by its own type")
    }

    fn typed<T: Any>(&self) -> impl DoubleEndedIterator<Item = &T> {
        self.typed
            .get(&TypeId::of::<T>())
            .and_then(|slot| slot.downcast_ref::<Vec<T>>())
            .into_iter()
            .flatten()
    }

    /// Policy, Gate, Around [ Wrap [ Run ] ], Listener, Observer. A veto stops everything to its
    /// right except the observers; a run error skips the listeners; observers see both outcomes.
    pub async fn run<S: Stage>(&self, stage: &dyn Run<S>, input: S::In) -> DomainResult<S::Out> {
        for policy in self.policies.get(&S::KIND).into_iter().flatten() {
            if let Err(veto) = policy.enforce(S::KIND, &input).await {
                return self.failed(S::KIND, &input, veto).await;
            }
        }
        for gate in self.typed::<Arc<dyn Gate<S>>>() {
            if let Err(veto) = gate.check(&input).await {
                return self.failed(S::KIND, &input, veto).await;
            }
        }
        let envelope = input.envelope();
        let output = match self.wrapped(stage, input).await {
            Ok(output) => output,
            Err(error) => return self.failed(S::KIND, &*envelope, error).await,
        };
        for listener in self.typed::<Arc<dyn Listener<S>>>() {
            listener.listen(&output).await;
        }
        for observer in self.observers.get(&S::KIND).into_iter().flatten() {
            observer.observe(S::KIND, &output, Ok(())).await;
        }
        Ok(output)
    }

    async fn failed<T>(
        &self,
        stage: StageKind,
        action: &dyn Action,
        error: DomainError,
    ) -> DomainResult<T> {
        for observer in self.observers.get(&stage).into_iter().flatten() {
            observer.observe(stage, action, Err(&error)).await;
        }
        Err(error)
    }

    async fn wrapped<'a, S: Stage>(
        &'a self,
        stage: &'a dyn Run<S>,
        input: S::In,
    ) -> DomainResult<S::Out> {
        let mut next: Next<'a, S> = Next::new(Box::new(move |input| stage.run(input)));
        for wrap in self.typed::<Arc<dyn Wrap<S>>>().rev() {
            let inner = next;
            next = Next::new(Box::new(move |input| {
                Box::pin(async move { wrap.wrap(input, inner).await })
            }));
        }
        let arounds = self
            .arounds
            .get(&S::KIND)
            .map(Vec::as_slice)
            .unwrap_or_default();
        if arounds.is_empty() {
            return next.run(input).await;
        }
        Proceed::new::<S>(input, next, arounds)
            .run()
            .await
            .map(Done::into_output::<S>)
    }
}
