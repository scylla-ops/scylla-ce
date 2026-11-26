//! The tests of the mail settings: the check of a relay and of its password.

use super::*;

/// Makes the values of a relay with TLS and a sender.
fn raw() -> RawMailRelay {
    RawMailRelay {
        host: " SMTP.tem.Scaleway.com ".to_owned(),
        port: 465,
        security: Some(MailSecurity::Tls),
        username: " project-id ".to_owned(),
        from_address: "No-Reply@Scylla.example".to_owned(),
        from_name: " Scylla ".to_owned(),
    }
}

/// A relay keeps its values trimmed, with the host and the address in lower case.
#[test]
fn a_relay_keeps_its_values_trimmed() {
    // when
    let relay = MailRelay::parse(raw()).unwrap();

    // then
    assert_eq!(
        relay,
        MailRelay {
            host: "smtp.tem.scaleway.com".to_owned(),
            port: 465,
            security: MailSecurity::Tls,
            username: "project-id".to_owned(),
            from_address: "no-reply@scylla.example".to_owned(),
            from_name: "Scylla".to_owned(),
        }
    );
    assert!(relay.authenticates());
}

/// A bad value gives the name of its field.
#[test]
fn a_bad_value_gives_its_field() {
    // given
    let cases = [
        (
            RawMailRelay {
                host: " ".to_owned(),
                ..raw()
            },
            "host",
        ),
        (
            RawMailRelay {
                host: "smtp relay".to_owned(),
                ..raw()
            },
            "host",
        ),
        (
            RawMailRelay {
                host: "-relay.example".to_owned(),
                ..raw()
            },
            "host",
        ),
        (
            RawMailRelay {
                host: "relay.example.".to_owned(),
                ..raw()
            },
            "host",
        ),
        (
            RawMailRelay {
                host: "a".repeat(254),
                ..raw()
            },
            "host",
        ),
        (RawMailRelay { port: 0, ..raw() }, "port"),
        (
            RawMailRelay {
                port: 65536,
                ..raw()
            },
            "port",
        ),
        (
            RawMailRelay {
                security: None,
                ..raw()
            },
            "security",
        ),
        (
            RawMailRelay {
                username: "a\nb".to_owned(),
                ..raw()
            },
            "username",
        ),
        (
            RawMailRelay {
                username: "a".repeat(256),
                ..raw()
            },
            "username",
        ),
        (
            RawMailRelay {
                from_address: "no-reply".to_owned(),
                ..raw()
            },
            "from_address",
        ),
        (
            RawMailRelay {
                from_name: "a".repeat(101),
                ..raw()
            },
            "from_name",
        ),
        (
            RawMailRelay {
                from_name: "Scylla\r\nBcc: x".to_owned(),
                ..raw()
            },
            "from_name",
        ),
    ];

    // when
    let fields: Vec<&str> = cases
        .iter()
        .map(|(raw, _)| MailRelay::parse(raw.clone()).unwrap_err().0)
        .collect();

    // then
    let expected: Vec<&str> = cases.iter().map(|(_, field)| *field).collect();
    assert_eq!(fields, expected);
}

/// The ends of the ranges pass: port 1 and 65535, an IPv6 address, and a relay with no user name
/// that does not authenticate.
#[test]
fn the_limits_pass() {
    // given
    let cases = [
        RawMailRelay { port: 1, ..raw() },
        RawMailRelay {
            port: 65535,
            ..raw()
        },
        RawMailRelay {
            host: "::1".to_owned(),
            ..raw()
        },
        RawMailRelay {
            host: "a".repeat(253),
            ..raw()
        },
        RawMailRelay {
            username: String::new(),
            from_name: String::new(),
            ..raw()
        },
    ];

    // when
    let relays: Vec<MailRelay> = cases
        .into_iter()
        .map(|raw| MailRelay::parse(raw).unwrap())
        .collect();

    // then
    assert!(!relays[4].authenticates());
}

/// A password keeps its spaces, and refuses a line break or more than 1024 bytes.
#[test]
fn a_password_is_one_line_of_at_most_1024_bytes() {
    assert_eq!(password(" key with spaces ").unwrap(), " key with spaces ");
    assert_eq!(password(&"k".repeat(1024)).unwrap().len(), 1024);
    assert_eq!(
        password(&"k".repeat(1025)).unwrap_err(),
        DomainError::TooLarge { max: 1024 }
    );
    assert_eq!(password("a\nb").unwrap_err(), DomainError::NotOneLine);
}

/// Each security has a name in the database that reads back.
#[test]
fn a_security_reads_back_from_its_name() {
    for security in [
        MailSecurity::Tls,
        MailSecurity::StartTls,
        MailSecurity::None,
    ] {
        assert_eq!(MailSecurity::parse(security.as_str()), Some(security));
    }
    assert_eq!(MailSecurity::parse("ssl"), None);
}
