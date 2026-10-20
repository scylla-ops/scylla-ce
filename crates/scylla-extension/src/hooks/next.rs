use super::position::Around;
use crate::action::Action;
use crate::domain::errors::DomainResult;
use crate::stage::Stage;
use std::any::Any;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

pub(super) type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;
type StageCall<'a, S> =
    Box<dyn FnOnce(<S as Stage>::In) -> BoxFuture<'a, DomainResult<<S as Stage>::Out>> + Send + 'a>;

/// The rest of a typed chain: the inner wraps and the stage itself.
pub struct Next<'a, S: Stage> {
    call: StageCall<'a, S>,
}

impl<'a, S: Stage> Next<'a, S> {
    pub(super) fn new(call: StageCall<'a, S>) -> Self {
        Self { call }
    }

    pub async fn run(self, input: S::In) -> DomainResult<S::Out> {
        (self.call)(input).await
    }
}

/// The typed output, opaque to an `Around`: it can only come from `Proceed::run`.
pub struct Done(Box<dyn Any + Send>);

impl Done {
    pub(super) fn into_output<S: Stage>(self) -> S::Out {
        *self
            .0
            .downcast()
            .expect("an Around returned the Done of another stage")
    }
}

/// The rest of an erased chain. `action()` shows the input; `run()` consumes it.
pub struct Proceed<'a> {
    step: Box<dyn Step<'a> + 'a>,
}

impl<'a> Proceed<'a> {
    pub(super) fn new<S: Stage>(
        input: S::In,
        next: Next<'a, S>,
        rest: &'a [Arc<dyn Around>],
    ) -> Self {
        Self {
            step: Box::new(Chain { input, next, rest }),
        }
    }

    #[must_use]
    pub fn action(&self) -> &dyn Action {
        self.step.action()
    }

    pub async fn run(self) -> DomainResult<Done> {
        self.step.run().await
    }
}

trait Step<'a>: Send {
    fn action(&self) -> &dyn Action;
    fn run(self: Box<Self>) -> BoxFuture<'a, DomainResult<Done>>;
}

struct Chain<'a, S: Stage> {
    input: S::In,
    next: Next<'a, S>,
    rest: &'a [Arc<dyn Around>],
}

impl<'a, S: Stage> Step<'a> for Chain<'a, S> {
    fn action(&self) -> &dyn Action {
        &self.input
    }

    fn run(self: Box<Self>) -> BoxFuture<'a, DomainResult<Done>> {
        Box::pin(async move {
            let Chain { input, next, rest } = *self;
            match rest.split_first() {
                None => Ok(Done(Box::new(next.run(input).await?))),
                Some((around, rest)) => {
                    around
                        .around(S::KIND, Proceed::new::<S>(input, next, rest))
                        .await
                }
            }
        })
    }
}
