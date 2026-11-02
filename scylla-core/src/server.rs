use std::env;

use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;
use tokio::net::TcpListener;
use tonic::transport::server::TcpIncoming;
use tonic::transport::Server;
use tracing::Level;
use tracing_subscriber::filter::Targets;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

use crate::app::{self, Edition};
use crate::{db, Error};

const DEFAULT_BIND_ADDR: &str = "127.0.0.1:3000";

pub async fn run(edition: Edition) -> Result<(), Error> {
    init_tracing();

    let database_url = env::var("DATABASE_URL").map_err(|_| Error::MissingEnv("DATABASE_URL"))?;
    let bind_addr = env::var("BIND_ADDR").unwrap_or_else(|_| DEFAULT_BIND_ADDR.to_owned());

    let pool = PgPoolOptions::new().connect(&database_url).await?;
    db::migrate(&pool, edition.migrations).await?;

    let listener = TcpListener::bind(&bind_addr).await?;
    tracing::info!(
        edition = edition.capabilities.edition(),
        addr = %listener.local_addr()?,
        "listening"
    );
    serve(listener, pool, edition).await
}

pub async fn serve(listener: TcpListener, pool: PgPool, edition: Edition) -> Result<(), Error> {
    Server::builder()
        .add_routes(app::routes(pool, &edition))
        .serve_with_incoming(TcpIncoming::from(listener))
        .await?;
    Ok(())
}

fn init_tracing() {
    let filter = env::var("RUST_LOG")
        .ok()
        .and_then(|value| value.parse::<Targets>().ok())
        .unwrap_or_else(|| {
            Targets::new()
                .with_default(Level::INFO)
                .with_target("sqlx", Level::WARN)
        });
    tracing_subscriber::registry()
        .with(tracing_subscriber::fmt::layer())
        .with(filter)
        .init();
}
