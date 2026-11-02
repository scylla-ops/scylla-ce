use scylla_domain::users::UserId;
use uuid::Uuid;

use crate::db::Tx;
use crate::Error;

pub async fn insert(tx: &mut Tx<'_>, name: &str) -> Result<UserId, Error> {
    let id = Uuid::now_v7();
    sqlx::query!("INSERT INTO users (id, name) VALUES ($1, $2)", id, name)
        .execute(&mut **tx)
        .await?;
    Ok(UserId(id))
}
