use crate::domain::clock;
use crate::domain::ids::{SessionId, UserId};
use chrono::{DateTime, Duration, Utc};
use std::net::IpAddr;

/// The last activity of a session moves at most once in this interval.
pub const ACTIVITY_INTERVAL: Duration = Duration::minutes(5);

pub const USER_AGENT_MAX_CHARS: usize = 512;

/// The client that opened a session. The user agent is trimmed, loses its control characters
/// and keeps at most `USER_AGENT_MAX_CHARS` characters; an empty one is `None`. An IP address
/// is kept only when it parses as one.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SessionClient {
    user_agent: Option<String>,
    ip_address: Option<IpAddr>,
}

impl SessionClient {
    #[must_use]
    pub fn new(user_agent: Option<&str>, ip_address: Option<&str>) -> Self {
        Self {
            user_agent: user_agent.and_then(clean_user_agent),
            ip_address: ip_address.and_then(parse_ip_address),
        }
    }

    #[must_use]
    pub fn user_agent(&self) -> Option<&str> {
        self.user_agent.as_deref()
    }

    #[must_use]
    pub fn ip_address(&self) -> Option<IpAddr> {
        self.ip_address
    }
}

fn clean_user_agent(raw: &str) -> Option<String> {
    let clean: String = raw
        .trim()
        .chars()
        .filter(|c| !c.is_control())
        .take(USER_AGENT_MAX_CHARS)
        .collect();
    let clean = clean.trim_end();
    (!clean.is_empty()).then(|| clean.to_owned())
}

fn parse_ip_address(raw: &str) -> Option<IpAddr> {
    raw.trim()
        .parse::<IpAddr>()
        .ok()
        .map(|ip| ip.to_canonical())
}

#[derive(Debug, Clone)]
pub struct Session {
    id: SessionId,
    token: String,
    user_id: UserId,
    created_at: DateTime<Utc>,
    expires_at: DateTime<Utc>,
    last_active_at: DateTime<Utc>,
    client: SessionClient,
}

impl Session {
    #[must_use]
    pub fn from_persistence(
        id: SessionId,
        token: String,
        user_id: UserId,
        created_at: DateTime<Utc>,
        expires_at: DateTime<Utc>,
        last_active_at: DateTime<Utc>,
    ) -> Self {
        Self {
            id,
            token,
            user_id,
            created_at,
            expires_at,
            last_active_at,
            client: SessionClient::default(),
        }
    }

    #[must_use]
    pub fn create(user_id: UserId, token: String, duration: Duration) -> Self {
        let now = clock::now();
        Self {
            id: SessionId::generate(),
            token,
            user_id,
            created_at: now,
            expires_at: now + duration,
            last_active_at: now,
            client: SessionClient::default(),
        }
    }

    #[must_use]
    pub fn with_client(mut self, client: SessionClient) -> Self {
        self.client = client;
        self
    }

    #[must_use]
    pub fn is_expired(&self) -> bool {
        clock::now() > self.expires_at
    }

    /// True when the last activity is `ACTIVITY_INTERVAL` old or older at `now`.
    #[must_use]
    pub fn activity_due(&self, now: DateTime<Utc>) -> bool {
        now - self.last_active_at >= ACTIVITY_INTERVAL
    }

    #[must_use]
    pub fn id(&self) -> &SessionId {
        &self.id
    }

    #[must_use]
    pub fn token(&self) -> &str {
        &self.token
    }

    #[must_use]
    pub fn user_id(&self) -> &UserId {
        &self.user_id
    }

    #[must_use]
    pub fn created_at(&self) -> DateTime<Utc> {
        self.created_at
    }

    #[must_use]
    pub fn expires_at(&self) -> DateTime<Utc> {
        self.expires_at
    }

    #[must_use]
    pub fn last_active_at(&self) -> DateTime<Utc> {
        self.last_active_at
    }

    #[must_use]
    pub fn client(&self) -> &SessionClient {
        &self.client
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_user_agent_is_trimmed_and_cut_to_its_limit_in_characters() {
        let client = SessionClient::new(Some("  Mozilla/5.0  "), None);
        assert_eq!(client.user_agent(), Some("Mozilla/5.0"));

        let long = "é".repeat(USER_AGENT_MAX_CHARS + 10);
        let client = SessionClient::new(Some(&long), None);
        let kept = client.user_agent().unwrap();
        assert_eq!(kept.chars().count(), USER_AGENT_MAX_CHARS);
        assert!(long.starts_with(kept));
    }

    #[test]
    fn a_blank_user_agent_or_one_of_control_characters_is_none() {
        for raw in ["", "   ", "\t\n", "\0\u{7}"] {
            assert_eq!(SessionClient::new(Some(raw), None).user_agent(), None);
        }
        assert_eq!(
            SessionClient::new(Some("curl/8.0\0"), None).user_agent(),
            Some("curl/8.0")
        );
    }

    #[test]
    fn an_ip_address_is_kept_only_when_it_parses() {
        let v4 = SessionClient::new(None, Some(" 203.0.113.7 "));
        assert_eq!(v4.ip_address(), Some("203.0.113.7".parse().unwrap()));

        let v6 = SessionClient::new(None, Some("2001:db8::1"));
        assert_eq!(v6.ip_address(), Some("2001:db8::1".parse().unwrap()));

        let mapped = SessionClient::new(None, Some("::ffff:198.51.100.2"));
        assert_eq!(mapped.ip_address(), Some("198.51.100.2".parse().unwrap()));

        for bad in [
            "",
            "unknown",
            "203.0.113.7:443",
            "999.1.1.1",
            "[2001:db8::1]",
        ] {
            assert_eq!(SessionClient::new(None, Some(bad)).ip_address(), None);
        }
    }

    #[test]
    fn a_new_session_has_no_client_until_one_is_attached() {
        let session = Session::create(UserId::new("u"), "t".into(), Duration::hours(1));
        assert_eq!(session.client(), &SessionClient::default());

        let client = SessionClient::new(Some("Firefox"), Some("192.0.2.1"));
        let session = session.with_client(client.clone());
        assert_eq!(session.client(), &client);
    }

    #[test]
    fn the_activity_is_due_after_the_interval() {
        let session = Session::create(UserId::new("u"), "t".into(), Duration::hours(1));
        let at = session.last_active_at();

        assert!(!session.activity_due(at));
        assert!(!session.activity_due(at + ACTIVITY_INTERVAL - Duration::seconds(1)));
        assert!(session.activity_due(at + ACTIVITY_INTERVAL));
    }
}
