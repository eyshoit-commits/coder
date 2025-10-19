use axum::{routing::get, Router};
use std::net::SocketAddr;
use tracing::{info, Level};
use tracing_subscriber::FmtSubscriber;

#[tokio::main]
async fn main() {
    let subscriber = FmtSubscriber::builder()
        .with_max_level(Level::INFO)
        .with_env_filter("info")
        .finish();

    tracing::subscriber::set_global_default(subscriber).expect("setting tracing subscriber failed");

    let app = Router::new().route("/health", get(health));

    let port: u16 = std::env::var("RPC_PORT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(6813);
    let addr = SocketAddr::from(([0, 0, 0, 0], port));

    info!("Starting placeholder API service", %port);
    axum::Server::bind(&addr)
        .serve(app.into_make_service())
        .await
        .expect("server crashed");
}

async fn health() -> &'static str {
    "ok"
}
