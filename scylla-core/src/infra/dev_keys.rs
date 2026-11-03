//! The guard against the public dev keys: a server that other machines can reach never starts
//! with a key of the `.env` files of the repositories.

use std::net::IpAddr;

use sha2::{Digest, Sha256};
use url::{Host, Url};

use crate::{Config, Error};

/// The SHA-256 fingerprints of the dev keys in the `.env` files, which are public: the signing
/// key of the tokens and its public key, the SpiceDB key and the key of the client secrets. A
/// fingerprint is of the key with no space around it, in lower case.
const DEV_KEYS: [&str; 4] = [
    "75d43efce796364e478dcd335e0e16eaef47e01a2691543c7ef8020018b1667c",
    "7372aca5c2a32f060779041abc56cbe94cf66b699577d78f35fc2c1708190a6f",
    "e1ae89df8ce568943f0cf86a36424c92a797ea10f371ada4d85f5234eda2d9ee",
    "ce0ce9cf7e92bd17a8931d73dfe1a9907ff5a25da2db363b90a983310e431a95",
];

/// Refuses to serve with a dev key, unless the server is local: it binds a loopback address,
/// trusts no proxy, and its URL and the URL of the web app are on a loopback address. A local
/// server with a dev key logs a warning.
pub fn guard(config: &Config) -> Result<(), Error> {
    let Some(name) = dev_key(config) else {
        return Ok(());
    };
    match not_local(config) {
        None => {
            tracing::warn!(
                setting = name,
                "a public dev key is in use: serve on a loopback address only"
            );
            Ok(())
        }
        Some(why) => Err(Error::Config {
            name,
            reason: format!(
                "is a public dev key, and {why}: make a new key with `token keygen` or \
                 `secret keygen`"
            ),
        }),
    }
}

/// Returns the name of the first setting that holds a dev key.
fn dev_key(config: &Config) -> Option<&'static str> {
    let public = config
        .token_public_keys
        .iter()
        .any(|entry| is_dev_key(entry.split_once(':').map_or(entry.as_str(), |(_, key)| key)));
    if public {
        return Some("SCYLLA_TOKEN_PUBLIC_KEYS");
    }
    [
        ("SCYLLA_TOKEN_PRIVATE_KEY", &config.token_private_key),
        ("SCYLLA_SPICEDB_KEY", &config.spicedb_key),
        ("SCYLLA_SECRET_KEY", &config.secret_key),
    ]
    .into_iter()
    .find(|(_, value)| value.as_deref().is_some_and(is_dev_key))
    .map(|(name, _)| name)
}

/// Returns why other machines can reach the server, or `None` for a local server.
fn not_local(config: &Config) -> Option<String> {
    if !config.bind_addr.ip().is_loopback() {
        return Some(format!("{} is not a loopback address", config.bind_addr));
    }
    if config.trust_forwarded_headers {
        return Some("the server trusts the headers of a proxy".to_owned());
    }
    [
        ("SCYLLA_API_URL", &config.api_url),
        ("SCYLLA_APP_URL", &config.app_url),
    ]
    .into_iter()
    .find(|(_, url)| !is_loopback_url(url))
    .map(|(name, url)| format!("{name} {url} is not on a loopback address"))
}

/// Returns `true` if the host of a URL is `localhost` or a loopback address.
fn is_loopback_url(url: &str) -> bool {
    match Url::parse(url)
        .ok()
        .and_then(|url| url.host().map(|host| host.to_owned()))
    {
        Some(Host::Domain(name)) => name == "localhost",
        Some(Host::Ipv4(ip)) => IpAddr::V4(ip).is_loopback(),
        Some(Host::Ipv6(ip)) => IpAddr::V6(ip).is_loopback(),
        None => false,
    }
}

/// Returns `true` if a key is one of the dev keys.
fn is_dev_key(key: &str) -> bool {
    let digest = Sha256::digest(key.trim().to_ascii_lowercase().as_bytes());
    DEV_KEYS.contains(&hex::encode(digest).as_str())
}

#[cfg(test)]
mod tests {
    //! Unit tests of the guard: each key, the public key of the dev signing key, the addresses,
    //! the proxy, the URLs, and the keys of the `.env` file.

    use clap::Parser;

    use super::{guard, is_dev_key};
    use crate::access::TokenKeys;
    use crate::cli::Cli;
    use crate::{Config, Error};

    /// The `.env` file of the repository, with its dev keys.
    const ENV: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../.env"));

    /// A key that no `.env` file has.
    const NEW_KEY: &str = "1111111111111111111111111111111111111111111111111111111111111111";

    /// A loopback address.
    const LOCAL: &str = "127.0.0.1:3000";

    /// Returns the dev value of a setting of the `.env` file.
    fn dev(name: &str) -> &'static str {
        ENV.lines()
            .find_map(|line| line.strip_prefix(&format!("{name}=")))
            .expect("the setting is in .env")
    }

    /// Returns the public key of the dev signing key, in hex.
    fn dev_public_key() -> String {
        let private = dev("SCYLLA_TOKEN_PRIVATE_KEY");
        TokenKeys::new(private, 1, &[]).unwrap().public_hex()
    }

    /// Reads the settings of `serve` on an address, with the signing key, the SpiceDB key, the
    /// key of the secrets, and more flags.
    fn config(addr: &str, keys: [&str; 3], more: &[&str]) -> Config {
        let [token, spicedb, secret] = keys;
        let mut args = vec![
            "scylla".to_owned(),
            "--database-url=postgres://localhost/scylla".to_owned(),
            format!("--bind-addr={addr}"),
            format!("--token-private-key={token}"),
            format!("--spicedb-key={spicedb}"),
            format!("--secret-key={secret}"),
        ];
        args.extend(more.iter().map(|&flag| flag.to_owned()));
        Cli::try_parse_from(args).expect("valid flags").config
    }

    /// Returns the setting that stops the start, or `None` if the server starts.
    fn refused(config: &Config) -> Option<&'static str> {
        match guard(config) {
            Ok(()) => None,
            Err(Error::Config { name, .. }) => Some(name),
            Err(error) => panic!("an error of a setting: {error}"),
        }
    }

    /// Each dev key of the `.env` file and the public key of the dev signing key have their
    /// fingerprint in the list, in any case.
    #[test]
    fn the_list_has_each_key_of_the_env_file() {
        // given
        let names = [
            "SCYLLA_TOKEN_PRIVATE_KEY",
            "SCYLLA_SPICEDB_KEY",
            "SCYLLA_SECRET_KEY",
        ];

        // when
        let found = names.map(|name| is_dev_key(dev(name)));
        let public = is_dev_key(&dev_public_key());
        let upper = is_dev_key(&dev("SCYLLA_SECRET_KEY").to_uppercase());

        // then
        assert_eq!(found, [true, true, true]);
        assert!(public && upper && !is_dev_key(NEW_KEY));
    }

    /// A dev key on an address that is not a loopback address stops the start, and the error
    /// names its setting.
    #[test]
    fn guard_refuses_a_dev_key_on_a_public_address() {
        // given
        let addr = "0.0.0.0:3000";
        let configs = [
            config(
                addr,
                [dev("SCYLLA_TOKEN_PRIVATE_KEY"), NEW_KEY, NEW_KEY],
                &[],
            ),
            config(addr, [NEW_KEY, dev("SCYLLA_SPICEDB_KEY"), NEW_KEY], &[]),
            config(addr, [NEW_KEY, NEW_KEY, dev("SCYLLA_SECRET_KEY")], &[]),
        ];

        // when
        let names = configs.map(|config| refused(&config));

        // then
        let expected = [
            Some("SCYLLA_TOKEN_PRIVATE_KEY"),
            Some("SCYLLA_SPICEDB_KEY"),
            Some("SCYLLA_SECRET_KEY"),
        ];
        assert_eq!(names, expected);
    }

    /// The public key of the dev signing key in `SCYLLA_TOKEN_PUBLIC_KEYS` stops the start of a
    /// server that is not local, as the private key does.
    #[test]
    fn guard_refuses_the_public_key_of_the_dev_signing_key() {
        // given
        let keys = format!(
            "--token-public-keys=2:{NEW_KEY},1:{}",
            dev_public_key().to_uppercase()
        );
        let public = config("0.0.0.0:3000", [NEW_KEY; 3], &[&keys]);
        let local = config(LOCAL, [NEW_KEY; 3], &[&keys]);

        // when
        let (on_public, on_local) = (refused(&public), refused(&local));

        // then
        assert_eq!(on_public, Some("SCYLLA_TOKEN_PUBLIC_KEYS"));
        assert_eq!(on_local, None);
    }

    /// A dev key on a loopback address stops the start when the server trusts the headers of a
    /// proxy, or when its URL or the URL of the web app is not on a loopback address.
    #[test]
    fn guard_refuses_a_dev_key_behind_a_proxy_or_a_public_url() {
        // given
        let keys = [dev("SCYLLA_TOKEN_PRIVATE_KEY"), NEW_KEY, NEW_KEY];
        let configs = [
            config(LOCAL, keys, &["--trust-forwarded-headers"]),
            config(LOCAL, keys, &["--api-url=https://scylla.example.com"]),
            config(LOCAL, keys, &["--app-url=https://app.example.com"]),
        ];
        let urls = [
            "--api-url=http://localhost:3000",
            "--app-url=http://[::1]:5173",
        ];
        let local = config(LOCAL, keys, &urls);

        // when
        let names = configs.map(|config| refused(&config));

        // then
        assert_eq!(names, [Some("SCYLLA_TOKEN_PRIVATE_KEY"); 3]);
        assert_eq!(refused(&local), None);
    }

    /// A dev key on a local server only gives a warning, and new keys start on each address.
    #[test]
    fn guard_allows_a_local_server_or_new_keys() {
        // given
        let token = dev("SCYLLA_TOKEN_PRIVATE_KEY");
        let configs = [
            config(LOCAL, [token, NEW_KEY, NEW_KEY], &[]),
            config("[::1]:3000", [token, NEW_KEY, NEW_KEY], &[]),
            config("0.0.0.0:3000", [NEW_KEY; 3], &["--trust-forwarded-headers"]),
        ];

        // when
        let names = configs.map(|config| refused(&config));

        // then
        assert_eq!(names, [None, None, None]);
    }
}
