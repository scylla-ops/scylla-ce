#[tokio::main]
async fn main() -> Result<(), scylla_core::Error> {
    scylla_core::server::run(scylla_ce::edition()).await
}
