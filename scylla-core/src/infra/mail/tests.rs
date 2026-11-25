//! Unit tests of the SMTP mailer: the `Message-ID`, the name in `EHLO` and the answer of the SMTP
//! server, against a local SMTP server that accepts one mail.

use scylla_extension::{Mail, Mailer};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpListener;
use tokio::task::JoinHandle;

use super::{address_of, hello_name, SmtpMailer};

/// The sender of the tests.
const FROM: &str = "Scylla <no-reply@mail.scylla.example>";

/// The answer of the local SMTP server to a mail, as Postfix gives it.
const QUEUED: &str = "250 2.0.0 Ok: queued as 12AE6699493";

/// Makes a mail to one address, with no image.
fn mail() -> Mail {
    Mail {
        to: "ada@example.com".to_owned(),
        subject: "Verify your email".to_owned(),
        body: "Open the link.".to_owned(),
        html: "<p>Open the link.</p>".to_owned(),
        images: Vec::new(),
    }
}

/// Starts a local SMTP server that accepts one mail. Returns its port, and a task that gives the
/// `EHLO` line of the client.
async fn smtp_server() -> (u16, JoinHandle<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let task = tokio::spawn(async move {
        // Greet the client.
        let (stream, _) = listener.accept().await.unwrap();
        let (read, mut write) = stream.into_split();
        let mut lines = BufReader::new(read).lines();
        write
            .write_all(b"220 relay.example ESMTP\r\n")
            .await
            .unwrap();

        // Answer each command, keep the `EHLO` line, and queue the mail at the end of its data.
        let (mut hello, mut data) = (String::new(), false);
        while let Some(line) = lines.next_line().await.unwrap() {
            let answer = match line.as_str() {
                "." if data => QUEUED,
                _ if data => continue,
                "DATA" => "354 End data with <CR><LF>.<CR><LF>",
                "QUIT" => break,
                command if command.starts_with("EHLO ") => {
                    hello = command.to_owned();
                    "250 relay.example"
                }
                _ => "250 2.1.0 Ok",
            };
            data = answer.starts_with("354");
            write
                .write_all(format!("{answer}\r\n").as_bytes())
                .await
                .unwrap();
        }
        let _ = write.write_all(b"221 2.0.0 Bye\r\n").await;
        hello
    });
    (port, task)
}

/// Each mail has its own `Message-ID` on the domain of the sender, so a spam filter does
/// not count a missing one.
#[test]
fn each_mail_has_a_message_id_on_the_domain_of_the_sender() {
    // given
    let mailer = SmtpMailer::new("smtp://127.0.0.1:51025", FROM).unwrap();

    // when
    let first = mailer.message(mail()).unwrap();
    let second = mailer.message(mail()).unwrap();

    // then
    let first = first.headers().get_raw("Message-ID").unwrap().to_owned();
    let second = second.headers().get_raw("Message-ID").unwrap();
    assert!(first.starts_with('<'), "{first}");
    assert!(first.ends_with("@mail.scylla.example>"), "{first}");
    assert_ne!(first, second);
}

/// A recipient that is not an address is a permanent error: a later send cannot work.
#[test]
fn a_bad_recipient_is_a_permanent_error() {
    // given
    let mailer = SmtpMailer::new("smtp://127.0.0.1:51025", FROM).unwrap();
    let bad = Mail {
        to: "not an address".to_owned(),
        ..mail()
    };

    // when
    let error = mailer.message(bad).unwrap_err();

    // then
    assert!(error.permanent, "{error}");
}

/// The name of `EHLO` is the host of the web app, and an address with no host gives none.
#[test]
fn the_hello_name_is_the_host_of_the_web_app() {
    // given
    let urls = [
        "https://app.scylla-ops.com",
        "https://staging.scylla-ops.com/",
        "http://127.0.0.1:3000",
        "not an address",
    ];

    // when
    let names = urls.map(|url| hello_name(url).map(|name| name.to_string()));

    // then
    assert_eq!(
        names,
        [
            Some("app.scylla-ops.com".to_owned()),
            Some("staging.scylla-ops.com".to_owned()),
            Some("[127.0.0.1]".to_owned()),
            None
        ]
    );
}

/// A mailer of the web app gives its host in `EHLO`, and a traced send returns the `Message-ID`
/// of the mail and the answer of the SMTP server with its queue id.
#[tokio::test]
async fn send_traced_gives_the_host_of_the_web_app_and_returns_the_answer() {
    // given
    let (port, server) = smtp_server().await;
    let url = format!("smtp://127.0.0.1:{port}");
    let mailer = SmtpMailer::new_for_app(&url, FROM, "https://app.scylla.example").unwrap();

    // when
    let receipt = mailer.send_traced(mail()).await.unwrap();

    // then
    assert_eq!(server.await.unwrap(), "EHLO app.scylla.example");
    assert_eq!(receipt.reply.as_deref(), Some(QUEUED));
    let id = receipt.message_id.unwrap();
    assert!(id.starts_with('<'), "{id}");
    assert!(id.ends_with("@mail.scylla.example>"), "{id}");
}

/// A name in the path of the SMTP URL wins over the host of the web app.
#[tokio::test]
async fn send_gives_the_name_of_the_smtp_url() {
    // given
    let (port, server) = smtp_server().await;
    let url = format!("smtp://127.0.0.1:{port}/relay-client.example");
    let mailer = SmtpMailer::new_for_app(&url, FROM, "https://app.scylla.example").unwrap();

    // when
    mailer.send(mail()).await.unwrap();

    // then
    assert_eq!(server.await.unwrap(), "EHLO relay-client.example");
}

/// The address of a sender has no name, and a text that is not an address stays as it is.
#[test]
fn the_address_of_a_sender_has_no_name() {
    // given
    let senders = [
        "Scylla <no-reply@scylla.example>",
        "no-reply@scylla.example",
        " not an address ",
    ];

    // when
    let addresses = senders.map(address_of);

    // then
    assert_eq!(
        addresses,
        [
            "no-reply@scylla.example",
            "no-reply@scylla.example",
            "not an address"
        ]
    );
}
