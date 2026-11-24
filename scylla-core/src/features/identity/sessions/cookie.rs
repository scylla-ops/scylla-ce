//! The cookies of a browser: the session cookie with the refresh token, and the cookie of a
//! sign-in with an identity provider. Each one is HttpOnly and `SameSite=Lax`. With HTTPS, each one
//! is `Secure` and its name has the prefix `__Host-`.

use std::time::Duration;

/// The name of the session cookie.
pub const SESSION: &str = "scylla_session";

/// The name of the cookie of a sign-in that goes through an identity provider.
pub const FLOW: &str = "scylla_flow";

/// The name of the cookie of a link that waits for the password of an account.
pub const PENDING: &str = "scylla_pending";

/// The prefix of the name of a secure cookie. A browser keeps such a cookie only from HTTPS, with
/// `Secure`, `Path=/` and no `Domain`, so a subdomain cannot set it.
const HOST_PREFIX: &str = "__Host-";

/// Returns the name of a cookie in the browser: with the prefix `__Host-` when the cookie needs
/// HTTPS, else the name with no prefix.
pub fn name(base: &str, secure: bool) -> String {
    match secure {
        true => format!("{HOST_PREFIX}{base}"),
        false => base.to_owned(),
    }
}

/// Reads a cookie from the values of the `cookie` headers.
pub fn read<'a>(
    headers: impl IntoIterator<Item = &'a str>,
    base: &str,
    secure: bool,
) -> Option<String> {
    let name = name(base, secure);
    headers
        .into_iter()
        .flat_map(|header| header.split(';'))
        .filter_map(|pair| pair.trim().split_once('='))
        .find(|(key, _)| *key == name)
        .map(|(_, value)| value.to_owned())
}

/// Makes the `set-cookie` value of a cookie that lives `max_age`.
pub fn set(base: &str, value: &str, max_age: Duration, secure: bool) -> String {
    let name = name(base, secure);
    let secure = if secure { "; Secure" } else { "" };
    format!(
        "{name}={value}; Max-Age={}; Path=/; HttpOnly; SameSite=Lax{secure}",
        max_age.as_secs()
    )
}

/// Makes the `set-cookie` value that removes a cookie.
pub fn clear(base: &str, secure: bool) -> String {
    set(base, "", Duration::ZERO, secure)
}

#[cfg(test)]
mod tests {
    //! Unit tests of the cookies.

    use std::time::Duration;

    use super::{clear, read, set, SESSION};

    /// A cookie is found among the other cookies of the headers.
    #[test]
    fn read_finds_a_cookie_among_others() {
        // given
        let headers = ["a=1; scylla_session=abc.def", "b=2"];

        // when
        let found = read(headers, "scylla_session", false);
        let missing = read(headers, "scylla_flow", false);

        // then
        assert_eq!(found.as_deref(), Some("abc.def"));
        assert_eq!(missing, None);
    }

    /// With HTTPS, the server reads only the cookie with the prefix `__Host-`.
    #[test]
    fn read_takes_the_host_prefix_with_https() {
        // given
        let headers = ["scylla_session=old; __Host-scylla_session=new"];

        // when
        let secure = read(headers, SESSION, true);
        let local = read(headers, SESSION, false);

        // then
        assert_eq!(secure.as_deref(), Some("new"));
        assert_eq!(local.as_deref(), Some("old"));
    }

    /// A secure cookie has the flag `Secure`, and each cookie is HttpOnly.
    #[test]
    fn set_writes_the_flags() {
        // given
        let max_age = Duration::from_secs(60);

        // when
        let secure = set(SESSION, "v", max_age, true);
        let local = set(SESSION, "v", max_age, false);

        // then
        assert!(secure.ends_with("HttpOnly; SameSite=Lax; Secure"));
        assert!(local.ends_with("HttpOnly; SameSite=Lax"));
    }

    /// With HTTPS, a cookie has the prefix `__Host-`, `Path=/` and no `Domain`. With HTTP, it
    /// keeps the name with no prefix, which a browser takes with no HTTPS.
    #[test]
    fn set_adds_the_host_prefix_with_https() {
        // given
        let max_age = Duration::from_secs(60);

        // when
        let secure = set(SESSION, "v", max_age, true);
        let local = set(SESSION, "v", max_age, false);
        let cleared = clear(SESSION, true);

        // then
        assert!(secure.starts_with("__Host-scylla_session=v;"), "{secure}");
        assert!(secure.contains("; Path=/;") && !secure.contains("Domain"));
        assert!(local.starts_with("scylla_session=v;"), "{local}");
        assert!(cleared.starts_with("__Host-scylla_session=; Max-Age=0;"));
    }
}
