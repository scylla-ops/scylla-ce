use crate::ports::Scopes;
use scylla_auth::authz::{PermissionService, Visibility, VisibilityResolver};
use scylla_core::application::auth::{SessionLookup, look_up_session, record_activity};
use scylla_core::application::{AppTokenRepository, SessionRepository};
use scylla_domain::domain::caller::CallerContext;
use scylla_domain::domain::errors::{DomainError, DomainResult};
use scylla_domain::domain::ids::{AppId, SessionId, UserId};
use scylla_domain::domain::permission::Permission;
use scylla_domain::domain::session::SessionClient;
use std::sync::Arc;
use std::time::Instant;
use strata::{BuildCx, BuildError, Component, Context, Headers, Rejection, RequestMeta, Resource};

#[derive(Clone, Debug)]
pub enum Refusal {
    Unauthenticated(String),
    Internal,
}

impl Refusal {
    #[must_use]
    pub fn error(&self) -> DomainError {
        match self {
            Refusal::Unauthenticated(message) => DomainError::Unauthorized(message.clone()),
            Refusal::Internal => DomainError::internal("the bearer token could not be checked"),
        }
    }
}

#[derive(Clone, Debug)]
pub struct ScyllaCtx {
    pub caller: CallerContext,
    pub session: Option<SessionId>,
    pub client: Option<SessionClient>,
    pub refused: Option<Refusal>,
    pub meta: RequestMeta,
}

static ANONYMOUS: ScyllaCtx = ScyllaCtx {
    caller: CallerContext::Anonymous,
    session: None,
    client: None,
    refused: None,
    meta: RequestMeta::NONE,
};

impl Default for ScyllaCtx {
    fn default() -> Self {
        ANONYMOUS.clone()
    }
}

impl Context for ScyllaCtx {
    fn anonymous() -> &'static ScyllaCtx {
        &ANONYMOUS
    }

    fn deadline(&self) -> Option<Instant> {
        self.meta.deadline
    }
}

pub trait CallerOf: Context {
    fn caller(&self) -> &CallerContext;
    fn session(&self) -> Option<&SessionId>;
    fn client(&self) -> SessionClient;
    fn refused(&self) -> Option<&Refusal>;
    fn from_parts(identity: Identity, client: SessionClient, meta: RequestMeta) -> Self;
}

impl CallerOf for ScyllaCtx {
    fn caller(&self) -> &CallerContext {
        &self.caller
    }
    fn session(&self) -> Option<&SessionId> {
        self.session.as_ref()
    }
    fn client(&self) -> SessionClient {
        self.client.clone().unwrap_or_default()
    }
    fn refused(&self) -> Option<&Refusal> {
        self.refused.as_ref()
    }
    fn from_parts(identity: Identity, client: SessionClient, meta: RequestMeta) -> Self {
        let (caller, session, refused) = match identity {
            Identity::Caller { caller, session } => (caller, session, None),
            Identity::Refused(refusal) => (CallerContext::Anonymous, None, Some(refusal)),
        };
        ScyllaCtx {
            caller,
            session,
            client: Some(client),
            refused,
            meta,
        }
    }
}

#[derive(Clone, Debug)]
pub enum Identity {
    Caller {
        caller: CallerContext,
        session: Option<SessionId>,
    },
    Refused(Refusal),
}

impl Identity {
    #[must_use]
    pub fn missing() -> Self {
        Identity::Refused(Refusal::Unauthenticated(
            "Missing or invalid authorization token".to_owned(),
        ))
    }

    #[must_use]
    pub fn of(caller: CallerContext) -> Self {
        Identity::Caller {
            caller,
            session: None,
        }
    }
}

#[strata::port]
pub trait Authenticator: Send + Sync + 'static {
    async fn authenticate(&self, token: Option<&str>) -> Identity;
}

#[derive(strata::Component)]
#[dev = "accepts the tokens user:<id> and app:<id> with no check"]
pub struct DevTokens;

impl Authenticator for DevTokens {
    async fn authenticate(&self, token: Option<&str>) -> Identity {
        let Some(token) = token else {
            return Identity::missing();
        };
        match token.split_once(':') {
            Some(("user", id)) if !id.is_empty() => {
                Identity::of(CallerContext::User(UserId::new(id)))
            }
            Some(("app", id)) if !id.is_empty() => Identity::of(CallerContext::App(AppId::new(id))),
            _ => Identity::Refused(Refusal::Unauthenticated(
                "Invalid or expired token".to_owned(),
            )),
        }
    }
}

#[derive(Default)]
pub struct ForwardedHeaders {
    pub trust: bool,
}

impl ForwardedHeaders {
    #[must_use]
    pub fn client(&self, headers: &(impl Headers + ?Sized)) -> SessionClient {
        let forwarded = if self.trust {
            headers
                .header("x-forwarded-for")
                .map(|list| list.split(',').next().unwrap_or_default().trim().to_owned())
                .or_else(|| headers.header("x-real-ip").map(str::to_owned))
        } else {
            None
        };
        let ip = forwarded.or_else(|| headers.remote_addr().map(|addr| addr.ip().to_string()));
        SessionClient::new(headers.header("user-agent"), ip.as_deref())
    }
}

#[strata::port]
pub trait Permissions: Send + Sync + 'static {
    async fn check(&self, caller: &CallerContext, permission: Permission) -> DomainResult<()>;
}

#[derive(strata::Component)]
#[dev = "refuses every caller: a deployment selects the access model"]
pub struct DenyAll;

impl Permissions for DenyAll {
    async fn check(&self, _: &CallerContext, permission: Permission) -> DomainResult<()> {
        Err(DomainError::forbidden(format!(
            "{} is not granted",
            permission.key()
        )))
    }
}

/// The Cedar access model of Scylla: the permission check and the visible scopes. The binary
/// builds it and gives it to the build with `Builder::provide`.
#[derive(Clone)]
pub struct AccessModel {
    pub permissions: Arc<dyn PermissionService>,
    pub visibility: Arc<dyn VisibilityResolver>,
}

impl Resource for AccessModel {
    const SECTION: &'static str = "access";
    async fn build(_: &mut BuildCx<'_>) -> Result<Self, BuildError> {
        Err(BuildError::msg(
            "the access model is built by the binary: give it with `Builder::provide`",
        ))
    }
}

pub struct CedarPermissions(AccessModel);

impl Component for CedarPermissions {
    async fn build(cx: &mut BuildCx<'_>) -> Result<Self, BuildError> {
        Ok(CedarPermissions(cx.resource::<AccessModel>().await?))
    }
}

impl Permissions for CedarPermissions {
    async fn check(&self, caller: &CallerContext, permission: Permission) -> DomainResult<()> {
        self.0.permissions.check(caller, permission).await
    }
}

/// Who calls, and what the caller may do. Each module that needs a caller extends it.
#[strata::module(section = "access")]
pub trait Access {
    #[context]
    type Ctx: CallerOf = ScyllaCtx;

    #[component]
    type Authenticator: Authenticator = DevTokens;

    #[component]
    type Permissions: Permissions = DenyAll;

    #[component]
    type Forwarding: Forwarding = ForwardedHeaders;

    #[context]
    async fn context_from(
        &self,
        headers: &(impl Headers + ?Sized),
    ) -> Result<Self::Ctx, Rejection> {
        let meta = meta_like_ce(headers);
        let identity = match bearer_like_ce(headers) {
            Ok(token) => self.authenticator().authenticate(token).await,
            Err(refusal) => Identity::Refused(refusal),
        };
        let client = self.forwarding().headers().client(headers);
        Ok(Self::Ctx::from_parts(identity, client, meta))
    }

    /// Only gives the refusal of the context. No hook targets it.
    #[usecase]
    async fn authenticated(&self) -> DomainResult<()> {
        match self.ctx().refused() {
            Some(refusal) => Err(refusal.error()),
            None => Ok(()),
        }
    }

    async fn require(&self, permission: Permission) -> DomainResult<()> {
        self.require_access(scylla_extension::Access::Requires(permission))
            .await
    }

    async fn require_access(&self, access: scylla_extension::Access) -> DomainResult<()> {
        if access == scylla_extension::Access::Public {
            return Ok(());
        }
        if let Some(refusal) = self.ctx().refused() {
            return Err(refusal.error());
        }
        let caller = self.ctx().caller();
        if access == scylla_extension::Access::Authenticated && *caller == CallerContext::Anonymous
        {
            return Err(DomainError::forbidden("Anonymous caller is not permitted"));
        }
        for permission in access.permissions() {
            self.permissions().check(caller, permission.clone()).await?;
        }
        Ok(())
    }
}

pub struct CedarScopes(AccessModel);

impl Component for CedarScopes {
    async fn build(cx: &mut BuildCx<'_>) -> Result<Self, BuildError> {
        Ok(CedarScopes(cx.resource::<AccessModel>().await?))
    }
}

impl Scopes for CedarScopes {
    async fn visible(&self, caller: &CallerContext, permission: &str) -> DomainResult<Visibility> {
        self.0.visibility.visible_scopes(caller, permission).await
    }
}

/// The sessions and the app tokens of Scylla. The binary gives them with `Builder::provide`.
#[derive(Clone)]
pub struct SessionStore {
    pub sessions: Arc<dyn SessionRepository>,
    pub app_tokens: Arc<dyn AppTokenRepository>,
    pub trust_forwarded_headers: bool,
}

impl Resource for SessionStore {
    const SECTION: &'static str = "auth";
    async fn build(_: &mut BuildCx<'_>) -> Result<Self, BuildError> {
        Err(BuildError::msg(
            "the session store is built by the binary: give it with `Builder::provide`",
        ))
    }
}

/// The bearer tokens of Scylla: a user session, then an app token. The rule of
/// `AuthInterceptor`, as an adapter.
pub struct ScyllaSessions(SessionStore);

impl Component for ScyllaSessions {
    async fn build(cx: &mut BuildCx<'_>) -> Result<Self, BuildError> {
        Ok(ScyllaSessions(cx.resource::<SessionStore>().await?))
    }
}

impl Authenticator for ScyllaSessions {
    async fn authenticate(&self, token: Option<&str>) -> Identity {
        let unauthenticated = |m: &str| Identity::Refused(Refusal::Unauthenticated(m.to_owned()));
        let failed = |e: DomainError| {
            tracing::error!(error = %e, "cannot check the bearer token");
            Identity::Refused(Refusal::Internal)
        };
        let Some(token) = token else {
            return Identity::missing();
        };
        match look_up_session(self.0.sessions.as_ref(), token).await {
            Err(e) => return failed(e),
            Ok(SessionLookup::Live(session)) => {
                record_activity(self.0.sessions.as_ref(), &session).await;
                return Identity::Caller {
                    caller: CallerContext::User(session.user_id().clone()),
                    session: Some(session.id().clone()),
                };
            }
            Ok(SessionLookup::Expired) => return unauthenticated("Token has expired"),
            Ok(SessionLookup::Unknown) => {}
        }
        match self.0.app_tokens.find_by_token(token).await {
            Ok(app) if app.is_expired() => unauthenticated("Token has expired"),
            Ok(app) => Identity::of(CallerContext::App(app.app_id().clone())),
            Err(e) if e.is_not_found() => unauthenticated("Invalid or expired token"),
            Err(e) => failed(e),
        }
    }
}

pub trait Forwarding: Send + Sync + 'static {
    fn headers(&self) -> &ForwardedHeaders;
}

impl Forwarding for ForwardedHeaders {
    fn headers(&self) -> &ForwardedHeaders {
        self
    }
}

pub struct TrustedForwarding(ForwardedHeaders);

impl Component for TrustedForwarding {
    async fn build(cx: &mut BuildCx<'_>) -> Result<Self, BuildError> {
        Ok(TrustedForwarding(ForwardedHeaders {
            trust: cx.resource::<SessionStore>().await?.trust_forwarded_headers,
        }))
    }
}

impl Forwarding for TrustedForwarding {
    fn headers(&self) -> &ForwardedHeaders {
        &self.0
    }
}

pub fn meta_like_ce(headers: &(impl Headers + ?Sized)) -> RequestMeta {
    let deadline = headers
        .header("grpc-timeout")
        .and_then(grpc_timeout)
        .and_then(|timeout| Instant::now().checked_add(timeout));
    RequestMeta {
        request_id: headers.header("x-request-id").map(str::to_owned),
        deadline,
        traceparent: headers.header("traceparent").map(str::to_owned),
    }
}

fn grpc_timeout(value: &str) -> Option<std::time::Duration> {
    use std::time::Duration;
    let (digits, unit) = value.split_at(value.len().checked_sub(1)?);
    if digits.is_empty() || digits.len() > 8 || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let n: u64 = digits.parse().ok()?;
    Some(match unit {
        "H" => Duration::from_secs(n * 3600),
        "M" => Duration::from_secs(n * 60),
        "S" => Duration::from_secs(n),
        "m" => Duration::from_millis(n),
        "u" => Duration::from_micros(n),
        "n" => Duration::from_nanos(n),
        _ => return None,
    })
}

pub fn bearer_like_ce(headers: &(impl Headers + ?Sized)) -> Result<Option<&str>, Refusal> {
    match headers.header("authorization") {
        None if headers.present("authorization") => Err(Refusal::Unauthenticated(
            "Invalid authorization header".to_owned(),
        )),
        None => Ok(None),
        Some(value) => match value.strip_prefix("Bearer ") {
            Some(token) => Ok(Some(token)),
            None => Err(Refusal::Unauthenticated(
                "Missing or invalid authorization token".to_owned(),
            )),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct One(Option<&'static str>);

    impl Headers for One {
        fn header(&self, name: &str) -> Option<&str> {
            (name == "authorization").then_some(self.0).flatten()
        }
    }

    fn missing(r: Result<Option<&str>, Refusal>) -> bool {
        matches!(r, Err(Refusal::Unauthenticated(m)) if m == "Missing or invalid authorization token")
    }

    struct Pairs<'a>(&'a [(&'a str, &'a str)]);

    impl Headers for Pairs<'_> {
        fn header(&self, name: &str) -> Option<&str> {
            self.0.iter().find(|(n, _)| *n == name).map(|(_, v)| *v)
        }
    }

    #[test]
    fn the_deadline_rule_is_the_one_of_tonic() {
        assert!(
            meta_like_ce(&Pairs(&[("grpc-timeout", "300m")]))
                .deadline
                .is_some()
        );
        assert!(
            meta_like_ce(&Pairs(&[("grpc-timeout", "2S")]))
                .deadline
                .is_some()
        );
        for bad in ["abc", "", "m", "123456789S", "10x", " 5S"] {
            assert!(
                meta_like_ce(&Pairs(&[("grpc-timeout", bad)]))
                    .deadline
                    .is_none(),
                "{bad:?}"
            );
        }
        assert!(
            meta_like_ce(&Pairs(&[("x-request-timeout-ms", "0")]))
                .deadline
                .is_none()
        );
        let meta = meta_like_ce(&Pairs(&[("x-request-id", "r1"), ("traceparent", "t1")]));
        assert_eq!(
            (meta.request_id.as_deref(), meta.traceparent.as_deref()),
            (Some("r1"), Some("t1"))
        );
    }

    struct Unreadable;

    impl Headers for Unreadable {
        fn header(&self, _: &str) -> Option<&str> {
            None
        }

        fn present(&self, name: &str) -> bool {
            name == "authorization"
        }
    }

    #[test]
    fn an_unreadable_header_is_an_invalid_header() {
        assert!(matches!(
            bearer_like_ce(&Unreadable),
            Err(Refusal::Unauthenticated(m)) if m == "Invalid authorization header"
        ));
    }

    #[test]
    fn the_bearer_rule_is_the_one_of_the_ce_interceptor() {
        assert_eq!(bearer_like_ce(&One(None)).unwrap(), None);
        assert_eq!(bearer_like_ce(&One(Some("Bearer T"))).unwrap(), Some("T"));
        assert_eq!(bearer_like_ce(&One(Some("Bearer "))).unwrap(), Some(""));
        assert_eq!(bearer_like_ce(&One(Some("Bearer  T"))).unwrap(), Some(" T"));
        assert_eq!(bearer_like_ce(&One(Some("Bearer T "))).unwrap(), Some("T "));
        for value in ["bearer T", "BEARER T", "Token T", "T", ""] {
            assert!(missing(bearer_like_ce(&One(Some(value)))), "{value:?}");
        }
    }
}
