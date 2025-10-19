use axum::{routing::post, Json, Router};
use serde::Deserialize;
use std::net::SocketAddr;
use tracing::{info, Level};
use tracing_subscriber::FmtSubscriber;

type AppResult<T> = Result<Json<T>, axum::http::StatusCode>;

#[derive(Deserialize)]
struct LoginRequest {
    username: String,
    password: String,
}

#[tokio::main]
async fn main() {
    let subscriber = FmtSubscriber::builder()
        .with_max_level(Level::INFO)
        .with_env_filter("info")
        .finish();

    tracing::subscriber::set_global_default(subscriber).expect("setting tracing subscriber failed");

    let app = Router::new().route("/login", post(login));

    let port: u16 = std::env::var("AUTH_PORT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(6971);

    let addr = SocketAddr::from(([0, 0, 0, 0], port));
    info!("Starting placeholder Auth service", %port);

    axum::Server::bind(&addr)
        .serve(app.into_make_service())
        .await
        .expect("auth server crashed");
}

async fn login(Json(_payload): Json<LoginRequest>) -> AppResult<serde_json::Value> {
    Ok(Json(serde_json::json!({
        "token": "placeholder",
        "role": "developer"
    })))
}
