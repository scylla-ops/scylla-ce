use sqlx::{Postgres, Transaction};

pub type Tx<'c> = Transaction<'c, Postgres>;
