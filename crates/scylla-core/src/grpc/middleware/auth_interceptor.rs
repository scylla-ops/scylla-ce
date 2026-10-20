use crate::application::auth::{SessionLookup, look_up_session, record_activity};
use crate::application::{AppTokenRepository, SessionRepository};
use crate::domain::caller::CallerContext;
use crate::domain::ids::SessionId;
use crate::grpc::mappers::domain_error_to_status;
use derive_more::Constructor;
use std::sync::Arc;
use tonic::{Request, Status};
use tonic_async_interceptor::AsyncInterceptor;

#[derive(Debug, Clone, Constructor)]
pub struct AuthContext {
    pub caller: CallerContext,
}

pub fn extract_auth_context<T>(request: &Request<T>) -> Result<AuthContext, Status> {
    request
        .extensions()
        .get::<AuthContext>()
        .ok_or_else(|| {
            Status::internal("Auth context not found — interceptor may not be configured")
        })
        .cloned()
}

/// The session behind the bearer token of a user caller. An action that must spare the session of
/// the call reads it; an app token has none.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallerSession(pub SessionId);

pub fn caller_session<T>(request: &Request<T>) -> Option<SessionId> {
    request
        .extensions()
        .get::<CallerSession>()
        .map(|session| session.0.clone())
}

fn extract_bearer_token<T>(request: &Request<T>) -> Result<String, Status> {
    let metadata = request.metadata();

    if let Some(auth_header) = metadata.get("authorization") {
        let auth_str = auth_header
            .to_str()
            .map_err(|_| Status::unauthenticated("Invalid authorization header"))?;

        if let Some(token) = auth_str.strip_prefix("Bearer ") {
            return Ok(token.to_string());
        }
    }

    Err(Status::unauthenticated(
        "Missing or invalid authorization token",
    ))
}

#[derive(Clone)]
pub struct AuthInterceptor {
    session_repo: Arc<dyn SessionRepository>,
    app_token_repo: Arc<dyn AppTokenRepository>,
}

impl AuthInterceptor {
    pub fn new(
        session_repo: Arc<dyn SessionRepository>,
        app_token_repo: Arc<dyn AppTokenRepository>,
    ) -> Self {
        Self {
            session_repo,
            app_token_repo,
        }
    }
}

impl AsyncInterceptor for AuthInterceptor {
    type Future = std::pin::Pin<Box<dyn Future<Output = Result<Request<()>, Status>> + Send>>;
    fn call(&mut self, mut request: Request<()>) -> Self::Future {
        let session_repo = self.session_repo.clone();
        let app_token_repo = self.app_token_repo.clone();

        Box::pin(async move {
            let token = extract_bearer_token(&request)?;

            // Only an unknown session falls through to the App token; a DB failure must surface as INTERNAL.
            match look_up_session(&*session_repo, &token)
                .await
                .map_err(domain_error_to_status)?
            {
                SessionLookup::Live(session) => {
                    record_activity(&*session_repo, &session).await;
                    let extensions = request.extensions_mut();
                    extensions.insert(AuthContext::new(CallerContext::User(
                        session.user_id().clone(),
                    )));
                    extensions.insert(CallerSession(session.id().clone()));
                    return Ok(request);
                }
                SessionLookup::Expired => return Err(Status::unauthenticated("Token has expired")),
                SessionLookup::Unknown => {}
            }

            match app_token_repo.find_by_token(&token).await {
                Ok(app_token) => {
                    if app_token.is_expired() {
                        return Err(Status::unauthenticated("Token has expired"));
                    }
                    request
                        .extensions_mut()
                        .insert(AuthContext::new(CallerContext::App(
                            app_token.app_id().clone(),
                        )));
                    return Ok(request);
                }
                Err(e) if e.is_not_found() => {}
                Err(e) => return Err(domain_error_to_status(e)),
            }

            Err(Status::unauthenticated("Invalid or expired token"))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::{AppTokenRepository, SessionRepository};
    use crate::test_support::sessions::SessionBuilder;
    use crate::test_support::stubs::StubSessions;
    use async_trait::async_trait;
    use chrono::Duration;
    use scylla_domain::domain::app::AppToken;
    use scylla_domain::domain::clock;
    use scylla_domain::domain::errors::{DomainError, DomainResult};
    use scylla_domain::domain::ids::{AppCredentialId, AppId, UserId};
    use scylla_domain::domain::session::{ACTIVITY_INTERVAL, Session};
    use std::sync::Arc;
    use tonic_async_interceptor::AsyncInterceptor;

    struct StubSessionRepo {
        find_by_token_fn: Box<dyn Fn(&str) -> DomainResult<Session> + Send + Sync>,
    }

    #[async_trait]
    impl SessionRepository for StubSessionRepo {
        async fn create(&self, _s: &Session) -> DomainResult<Session> {
            unreachable!("the interceptor opens no session")
        }
        async fn find_by_token(&self, token: &str) -> DomainResult<Session> {
            (self.find_by_token_fn)(token)
        }
        async fn delete_by_token(&self, _: &str) -> DomainResult<()> {
            unreachable!("the interceptor only reads")
        }
        async fn delete_expired(&self) -> DomainResult<u64> {
            unreachable!("the interceptor only reads")
        }
    }

    struct StubAppTokenRepo {
        find_by_token_fn: Box<dyn Fn(&str) -> DomainResult<AppToken> + Send + Sync>,
    }

    #[async_trait]
    impl AppTokenRepository for StubAppTokenRepo {
        async fn create(&self, _t: &AppToken) -> DomainResult<()> {
            unreachable!("the interceptor issues no token")
        }
        async fn find_by_token(&self, token: &str) -> DomainResult<AppToken> {
            (self.find_by_token_fn)(token)
        }
    }

    fn no_app_tokens() -> Arc<StubAppTokenRepo> {
        Arc::new(StubAppTokenRepo {
            find_by_token_fn: Box::new(|t| Err(DomainError::not_found("AppToken", t))),
        })
    }

    fn valid_session() -> Session {
        Session::create(
            UserId::generate(),
            "valid-token".to_string(),
            Duration::hours(24),
        )
    }

    fn expired_session() -> Session {
        Session::create(
            UserId::generate(),
            "expired-token".to_string(),
            Duration::hours(-1),
        )
    }

    #[test]
    fn extract_auth_context_missing() {
        let req = Request::new(());
        assert!(extract_auth_context(&req).is_err());
    }

    #[test]
    fn extract_auth_context_present() {
        let mut req = Request::new(());
        let user_id = UserId::generate();
        req.extensions_mut()
            .insert(AuthContext::new(CallerContext::User(user_id.clone())));

        let ctx = extract_auth_context(&req).unwrap();
        assert_eq!(ctx.caller, CallerContext::User(user_id));
    }

    #[tokio::test]
    async fn interceptor_valid_session_token() {
        let session = valid_session();
        let user_id = session.user_id().clone();
        let s = session.clone();

        let repo = Arc::new(StubSessionRepo {
            find_by_token_fn: Box::new(move |_| Ok(s.clone())),
        });

        let mut interceptor = AuthInterceptor::new(repo, no_app_tokens());
        let mut req = Request::new(());
        req.metadata_mut()
            .insert("authorization", "Bearer valid-token".parse().unwrap());

        let result = interceptor.call(req).await;
        assert!(result.is_ok());
        let req = result.unwrap();
        let ctx = req.extensions().get::<AuthContext>().unwrap();
        assert_eq!(ctx.caller, CallerContext::User(user_id));
        assert_eq!(caller_session(&req).as_ref(), Some(session.id()));
    }

    fn bearer(token: &str) -> Request<()> {
        let mut req = Request::new(());
        req.metadata_mut()
            .insert("authorization", format!("Bearer {token}").parse().unwrap());
        req
    }

    #[tokio::test]
    async fn a_call_moves_the_activity_of_an_idle_session_once_in_the_interval() {
        let user_id = UserId::generate();
        let idle = SessionBuilder::new(&user_id)
            .token("idle")
            .last_active_at(clock::now() - ACTIVITY_INTERVAL - Duration::seconds(1))
            .build();
        let fresh = SessionBuilder::new(&user_id).token("fresh").build();
        let sessions = Arc::new(StubSessions::with(idle.clone()));
        sessions.create(&fresh).await.unwrap();
        let mut interceptor = AuthInterceptor::new(sessions.clone(), no_app_tokens());

        for token in ["idle", "idle", "fresh"] {
            interceptor.call(bearer(token)).await.unwrap();
        }

        assert_eq!(sessions.touched(), vec![idle.id().clone()]);
    }

    #[tokio::test]
    async fn a_store_that_cannot_record_the_activity_does_not_refuse_the_call() {
        let idle = SessionBuilder::new(&UserId::generate())
            .token("idle")
            .last_active_at(clock::now() - ACTIVITY_INTERVAL * 2)
            .build();
        let s = idle.clone();
        let repo = Arc::new(StubSessionRepo {
            find_by_token_fn: Box::new(move |_| Ok(s.clone())),
        });
        let mut interceptor = AuthInterceptor::new(repo, no_app_tokens());

        let req = interceptor.call(bearer("idle")).await.unwrap();

        assert_eq!(caller_session(&req).as_ref(), Some(idle.id()));
    }

    #[tokio::test]
    async fn interceptor_app_token_resolves_to_app() {
        let app_id = AppId::new("agent-1");
        let token = AppToken::create(
            app_id.clone(),
            AppCredentialId::new("secret-1"),
            "app-token".to_string(),
            Duration::hours(24),
        );
        let t = token.clone();

        let session_repo = Arc::new(StubSessionRepo {
            find_by_token_fn: Box::new(|tok| Err(DomainError::not_found("Session", tok))),
        });
        let app_repo = Arc::new(StubAppTokenRepo {
            find_by_token_fn: Box::new(move |_| Ok(t.clone())),
        });

        let mut interceptor = AuthInterceptor::new(session_repo, app_repo);
        let mut req = Request::new(());
        req.metadata_mut()
            .insert("authorization", "Bearer app-token".parse().unwrap());

        let result = interceptor.call(req).await;
        assert!(result.is_ok());
        let req = result.unwrap();
        let ctx = req.extensions().get::<AuthContext>().unwrap();
        assert_eq!(ctx.caller, CallerContext::App(app_id));
        assert_eq!(caller_session(&req), None);
    }

    #[tokio::test]
    async fn interceptor_missing_auth_header() {
        let repo = Arc::new(StubSessionRepo {
            find_by_token_fn: Box::new(|_| unreachable!()),
        });

        let mut interceptor = AuthInterceptor::new(repo, no_app_tokens());
        let req = Request::new(());
        let result = interceptor.call(req).await;
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().code(), tonic::Code::Unauthenticated);
    }

    #[tokio::test]
    async fn interceptor_expired_session() {
        let session = expired_session();
        let s = session.clone();

        let repo = Arc::new(StubSessionRepo {
            find_by_token_fn: Box::new(move |_| Ok(s.clone())),
        });

        let mut interceptor = AuthInterceptor::new(repo, no_app_tokens());
        let mut req = Request::new(());
        req.metadata_mut()
            .insert("authorization", "Bearer expired-token".parse().unwrap());

        let result = interceptor.call(req).await;
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().code(), tonic::Code::Unauthenticated);
    }

    #[tokio::test]
    async fn interceptor_unknown_token() {
        let repo = Arc::new(StubSessionRepo {
            find_by_token_fn: Box::new(|_| Err(DomainError::not_found("Session", "x"))),
        });

        let mut interceptor = AuthInterceptor::new(repo, no_app_tokens());
        let mut req = Request::new(());
        req.metadata_mut()
            .insert("authorization", "Bearer unknown".parse().unwrap());

        let result = interceptor.call(req).await;
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().code(), tonic::Code::Unauthenticated);
    }
}
