//! The SQL of the `mail_settings` table: at most one row, the relay of the instance.

use scylla_domain::platform::mail::{MailRelay, MailSecurity};
use scylla_extension::Tx;

/// The relay of the instance as the table keeps it.
pub(super) struct Saved {
    /// The relay, with no password.
    pub(super) relay: MailRelay,
    /// The password, sealed with the key of the secrets.
    pub(super) password: Option<Vec<u8>>,
}

/// A row of the table.
struct Row {
    /// The host name or the IP address.
    host: String,
    /// The TCP port.
    port: i32,
    /// The security of the connection: `tls`, `starttls` or `none`.
    security: String,
    /// The user name. Empty: no authentication.
    username: String,
    /// The sealed password.
    password: Option<Vec<u8>>,
    /// The address of the sender.
    from_address: String,
    /// The name of the sender.
    from_name: String,
}

impl From<Row> for Saved {
    fn from(row: Row) -> Self {
        Self {
            relay: MailRelay {
                host: row.host,
                port: u16::try_from(row.port).unwrap_or_default(),
                security: MailSecurity::parse(&row.security).unwrap_or(MailSecurity::Tls),
                username: row.username,
                from_address: row.from_address,
                from_name: row.from_name,
            },
            password: row.password,
        }
    }
}

/// Returns the relay of the instance, if an admin set one.
pub(super) async fn get(tx: &mut Tx<'_>) -> Result<Option<Saved>, sqlx::Error> {
    let row = sqlx::query_as!(
        Row,
        "SELECT host, port, security, username, password, from_address, from_name
         FROM mail_settings"
    )
    .fetch_optional(&mut **tx)
    .await?;
    Ok(row.map(Saved::from))
}

/// Sets the relay of the instance and its sealed password, in place of the old one.
pub(super) async fn save(
    tx: &mut Tx<'_>,
    relay: &MailRelay,
    password: Option<&[u8]>,
) -> Result<(), sqlx::Error> {
    sqlx::query!(
        "INSERT INTO mail_settings
             (id, host, port, security, username, password, from_address, from_name, updated_at)
         VALUES (true, $1, $2, $3, $4, $5, $6, $7, now())
         ON CONFLICT (id) DO UPDATE
         SET host = EXCLUDED.host,
             port = EXCLUDED.port,
             security = EXCLUDED.security,
             username = EXCLUDED.username,
             password = EXCLUDED.password,
             from_address = EXCLUDED.from_address,
             from_name = EXCLUDED.from_name,
             updated_at = EXCLUDED.updated_at",
        relay.host,
        i32::from(relay.port),
        relay.security.as_str(),
        relay.username,
        password,
        relay.from_address,
        relay.from_name,
    )
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// Removes the relay of the instance. Returns `true` if there was one.
pub(super) async fn delete(tx: &mut Tx<'_>) -> Result<bool, sqlx::Error> {
    let done = sqlx::query!("DELETE FROM mail_settings")
        .execute(&mut **tx)
        .await?;
    Ok(done.rows_affected() > 0)
}
