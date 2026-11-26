//! A local SMTP relay for the tests of the mail settings. It accepts each mail, keeps its envelope,
//! its data and its `AUTH PLAIN` credentials, and answers as Postfix does. It sends no mail.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinHandle;

/// The longest time that `wait` waits for a mail.
const WAIT: Duration = Duration::from_secs(10);

/// A mail that the relay accepted.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Relayed {
    /// The name that the client gave in `EHLO`.
    pub hello: String,
    /// The user name and the password of `AUTH PLAIN`, if the client gave them.
    pub auth: Option<(String, String)>,
    /// The address of `MAIL FROM`.
    pub from: String,
    /// The addresses of `RCPT TO`.
    pub to: Vec<String>,
    /// The headers and the body of the mail.
    pub data: String,
}

/// A running local relay. A drop stops it.
pub struct SmtpRelay {
    /// The port of the relay on `127.0.0.1`.
    pub port: u16,
    /// The mails that the relay accepted, oldest first.
    mails: Arc<Mutex<Vec<Relayed>>>,
    /// The task that accepts the connections.
    task: JoinHandle<()>,
}

impl SmtpRelay {
    /// Starts a relay on a free port of `127.0.0.1`.
    ///
    /// # Panics
    ///
    /// If no port is free.
    pub async fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let port = listener.local_addr().expect("local addr").port();
        let mails = Arc::new(Mutex::new(Vec::new()));
        let shared = Arc::clone(&mails);
        let task = tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                tokio::spawn(session(stream, Arc::clone(&shared)));
            }
        });
        Self { port, mails, task }
    }

    /// Returns a port of `127.0.0.1` where no server listens: a connection gets a refusal.
    ///
    /// # Panics
    ///
    /// If no port is free.
    pub async fn closed_port() -> u16 {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        listener.local_addr().expect("local addr").port()
    }

    /// Waits until the relay has `count` mails or more, and returns them.
    ///
    /// # Panics
    ///
    /// If the mails do not come in 10 seconds.
    pub async fn wait(&self, count: usize) -> Vec<Relayed> {
        let start = tokio::time::Instant::now();
        loop {
            let mails = self.mails.lock().expect("relay").clone();
            if mails.len() >= count {
                return mails;
            }
            assert!(
                start.elapsed() < WAIT,
                "the relay got {} mails, not {count}",
                mails.len()
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }
}

impl Drop for SmtpRelay {
    fn drop(&mut self) {
        self.task.abort();
    }
}

/// Answers one connection: each command, then each mail at the end of its data.
async fn session(stream: TcpStream, mails: Arc<Mutex<Vec<Relayed>>>) {
    // Greet the client.
    let (read, mut write) = stream.into_split();
    let mut lines = BufReader::new(read).lines();
    if write.write_all(b"220 relay.test ESMTP\r\n").await.is_err() {
        return;
    }

    // Answer each command, and keep the mail at the end of its data.
    let (mut mail, mut data) = (Relayed::default(), false);
    while let Ok(Some(line)) = lines.next_line().await {
        let answer = match line.as_str() {
            "." if data => {
                let mut kept = mails.lock().expect("relay");
                kept.push(mail.clone());
                mail = Relayed {
                    hello: mail.hello.clone(),
                    auth: mail.auth.clone(),
                    ..Relayed::default()
                };
                format!("250 2.0.0 Ok: queued as Q{}", kept.len())
            }
            _ if data => {
                mail.data.push_str(&line);
                mail.data.push('\n');
                continue;
            }
            "DATA" => "354 End data with <CR><LF>.<CR><LF>".to_owned(),
            "QUIT" => break,
            command => answer(command, &mut mail),
        };
        data = answer.starts_with("354");
        let sent = write.write_all(format!("{answer}\r\n").as_bytes()).await;
        if sent.is_err() {
            return;
        }
    }
    let _ = write.write_all(b"221 2.0.0 Bye\r\n").await;
}

/// Answers a command that is not `DATA` or `QUIT`, and keeps what it tells about the mail.
fn answer(command: &str, mail: &mut Relayed) -> String {
    let (verb, rest) = command.split_once(' ').unwrap_or((command, ""));
    match verb.to_ascii_uppercase().as_str() {
        "EHLO" => {
            mail.hello = rest.to_owned();
            "250-relay.test\r\n250 AUTH PLAIN".to_owned()
        }
        "AUTH" => {
            mail.auth = rest.strip_prefix("PLAIN ").and_then(credentials);
            "235 2.7.0 Authentication successful".to_owned()
        }
        "MAIL" => {
            mail.from = address(rest);
            "250 2.1.0 Ok".to_owned()
        }
        "RCPT" => {
            mail.to.push(address(rest));
            "250 2.1.5 Ok".to_owned()
        }
        _ => "250 2.0.0 Ok".to_owned(),
    }
}

/// Reads the user name and the password of an `AUTH PLAIN` answer: base64 of
/// `\0user\0password`.
fn credentials(encoded: &str) -> Option<(String, String)> {
    let decoded = String::from_utf8(STANDARD.decode(encoded.trim()).ok()?).ok()?;
    let mut parts = decoded.split('\0').skip(1);
    Some((parts.next()?.to_owned(), parts.next()?.to_owned()))
}

/// Reads the address between `<` and `>` of `FROM:<address>` or `TO:<address>`.
fn address(rest: &str) -> String {
    let start = rest.find('<').map_or(0, |index| index + 1);
    let end = rest.rfind('>').unwrap_or(rest.len());
    rest.get(start..end).unwrap_or_default().to_owned()
}
