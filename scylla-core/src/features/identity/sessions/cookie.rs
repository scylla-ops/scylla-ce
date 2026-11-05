//! The cookies of a browser: the session cookie with the refresh token, and the cookie of a
//! sign-in with an identity provider. Each one is HttpOnly and `SameSite=Lax`.

use std::time::Duration;

/// The name of the session cookie.
pub const SESSION: &str = "scylla_session";

/// The name of the cookie of a sign-in that goes through an identity provider.
pub const FLOW: &str = "scylla_flow";

/// The name of the cookie of a link that waits for the password of an account.
pub const PENDING: &str = "scylla_pending";

/// Reads a cookie from the values of the `cookie` headers.
pub fn read<'a>(headers: impl IntoIterator<Item = &'a str>, name: &str) -> Option<String> {
    headers
        .into_iter()
        .flat_map(|header| header.split(';'))
        .filter_map(|pair| pair.trim().split_once('='))
        .find(|(key, _)| *key == name)
        .map(|(_, value)| value.to_owned())
}

/// Makes the `set-cookie` value of a cookie that lives `max_age`.
pub fn set(name: &str, value: &str, max_age: Duration, secure: bool) -> String {
    let secure = if secure { "; Secure" } else { "" };
    format!(
        "{name}={value}; Max-Age={}; Path=/; HttpOnly; SameSite=Lax{secure}",
        max_age.as_secs()
    )
}

/// Makes the `set-cookie` value that removes a cookie.
pub fn clear(name: &str, secure: bool) -> String {
    set(name, "", Duration::ZERO, secure)
}

#[cfg(test)]
mod tests {
    //! Unit tests of the cookies.

    use std::time::Duration;

    use super::{read, set};

    /// A cookie is found among the other cookies of the headers.
    #[test]
    fn read_finds_a_cookie_among_others() {
        // given
        let headers = ["a=1; scylla_session=abc.def", "b=2"];

        // when
        let found = read(headers, "scylla_session");
        let missing = read(headers, "scylla_flow");

        // then
        assert_eq!(found.as_deref(), Some("abc.def"));
        assert_eq!(missing, None);
    }

    /// A secure cookie has the flag `Secure`, and each cookie is HttpOnly.
    #[test]
    fn set_writes_the_flags() {
        // given
        let max_age = Duration::from_secs(60);

        // when
        let secure = set("scylla_session", "v", max_age, true);
        let local = set("scylla_session", "v", max_age, false);

        // then
        assert!(secure.ends_with("HttpOnly; SameSite=Lax; Secure"));
        assert!(local.ends_with("HttpOnly; SameSite=Lax"));
    }
}
