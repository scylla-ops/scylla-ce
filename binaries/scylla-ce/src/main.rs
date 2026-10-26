use scylla_core::config::ControlPlaneConfig;
use scylla_server::Server;
use scylla_server::cli::{self, Cli};
use scylla_strata::{EditionHost, ScyllaCeOnDb, Slice};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse_as(
        "scylla-ce",
        env!("CARGO_PKG_VERSION"),
        "Scylla Community Edition",
    );
    if cli.print_example_config {
        ControlPlaneConfig::print_example();
        return Ok(());
    }
    cli::init_tracing(&["scylla_ce"]);
    let config = cli.load_config()?;
    let db = scylla_db::init_db(&config.database).await?;
    Server::new(config, db)
        .strata(EditionHost::<ScyllaCeOnDb>::new(Slice::ALL))
        .serve()
        .await
}
