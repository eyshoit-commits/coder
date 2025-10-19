use std::net::SocketAddr;
use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use chrono::{Duration, Utc};
use jsonwebtoken::{decode, encode, Algorithm, DecodingKey, EncodingKey, Header, Validation};
use serde::{Deserialize, Serialize};
use sqlx::postgres::PgPoolOptions;
use sqlx::{PgPool, Row};
use tower_http::trace::TraceLayer;
use tracing::{dispatcher, error, info};
use uuid::Uuid;

use hex::encode as hex_encode;
use rand::rngs::OsRng;
use rand::RngCore;
use sha2::{Digest, Sha256};

#[derive(Clone)]
struct AppState {
    pool: PgPool,
    jwt: JwtConfig,
}

#[derive(Clone)]
struct JwtConfig {
    secret: Arc<[u8]>,
    expiration: Duration,
    issuer: String,
}

impl JwtConfig {
    fn from_env() -> anyhow::Result<Self> {
        let secret = std::env::var("AUTH_JWT_SECRET")
            .map_err(|_| anyhow::anyhow!("AUTH_JWT_SECRET environment variable is required"))?;
        let expiration_minutes = std::env::var("AUTH_JWT_EXP_MINUTES")
            .ok()
            .and_then(|v| v.parse::<i64>().ok())
            .unwrap_or(60);
        let issuer =
            std::env::var("AUTH_JWT_ISSUER").unwrap_or_else(|_| "cyber-dev-studio".to_string());
        Ok(Self {
            secret: Arc::from(secret.into_bytes()),
            expiration: Duration::minutes(expiration_minutes),
            issuer,
        })
    }

    fn validation(&self) -> Validation {
        let mut validation = Validation::new(Algorithm::HS256);
        validation
            .set_required_spec_claims(&["exp", "iat", "sub", "iss"])
            .expect("required claim configuration");
        validation.iss = Some(self.issuer.clone());
        validation
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct Claims {
    sub: i32,
    username: String,
    role: String,
    exp: usize,
    iat: usize,
    iss: String,
    jti: String,
}

#[derive(Debug)]
struct AuthenticatedUser {
    user_id: i32,
    username: String,
    role: String,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    init_tracing();
    let bind_addr = resolve_bind_address()?;
    let pool = build_pool().await?;
    let jwt = JwtConfig::from_env()?;

    let state = AppState { pool, jwt };

    let app = Router::new()
        .route("/health", get(health))
        .route("/auth/register", post(register_user))
        .route("/auth/login", post(login_user))
        .route("/auth/api-keys", get(list_api_keys).post(create_api_key))
        .route("/auth/api-keys/:id", delete(delete_api_key))
        .with_state(state)
        .layer(TraceLayer::new_for_http());

    info!("binding", %bind_addr, "auth service starting");
    axum::Server::bind(&bind_addr)
        .serve(app.into_make_service())
        .await?;
    Ok(())
}

fn init_tracing() {
    if dispatcher::has_been_set() {
        return;
    }
    let subscriber = tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,tower_http=info".into()),
        )
        .json()
        .finish();
    if let Err(err) = tracing::subscriber::set_global_default(subscriber) {
        eprintln!("failed to install tracing subscriber: {err}");
    }
}

fn resolve_bind_address() -> anyhow::Result<SocketAddr> {
    let raw = std::env::var("AUTH_BIND_ADDR").unwrap_or_else(|_| "0.0.0.0:6971".to_string());
    Ok(raw.parse()?)
}

async fn build_pool() -> anyhow::Result<PgPool> {
    let database_url = std::env::var("DATABASE_URL")
        .map_err(|_| anyhow::anyhow!("DATABASE_URL environment variable is required"))?;
    let max_connections = std::env::var("DATABASE_MAX_CONNECTIONS")
        .ok()
        .and_then(|v| v.parse::<u32>().ok())
        .unwrap_or(5);
    let pool = PgPoolOptions::new()
        .max_connections(max_connections)
        .acquire_timeout(std::time::Duration::from_secs(10))
        .connect(&database_url)
        .await?;
    Ok(pool)
}

async fn health() -> impl IntoResponse {
    (StatusCode::OK, Json(serde_json::json!({ "status": "ok" })))
}

async fn register_user(
    State(state): State<AppState>,
    Json(payload): Json<RegisterRequest>,
) -> Result<Json<RegisterResponse>, AuthError> {
    validate_role(&payload.role)?;
    if payload.password.len() < 12 {
        return Err(AuthError::BadRequest(
            "password must contain at least 12 characters".to_string(),
        ));
    }

    let hashed = bcrypt::hash(&payload.password, bcrypt::DEFAULT_COST)
        .map_err(|err| AuthError::Internal(err.to_string()))?;
    let role = payload.role.unwrap_or_else(|| "developer".to_string());

    let rec = sqlx::query(
        "INSERT INTO users (username, password_hash, role, token_balance) VALUES ($1, $2, $3, $4) RETURNING id",
    )
    .bind(&payload.username)
    .bind(&hashed)
    .bind(&role)
    .bind(payload.initial_tokens.unwrap_or(0_i64))
    .fetch_one(&state.pool)
    .await
    .map_err(|err| match err {
        sqlx::Error::Database(db_err) if db_err.code().as_deref() == Some("23505") => {
            AuthError::Conflict(format!("user '{}' already exists", payload.username))
        }
        other => AuthError::Internal(other.to_string()),
    })?;

    let id: i32 = rec.get("id");
    Ok(Json(RegisterResponse { user_id: id }))
}

async fn login_user(
    State(state): State<AppState>,
    Json(payload): Json<LoginRequest>,
) -> Result<Json<LoginResponse>, AuthError> {
    let row = sqlx::query("SELECT id, password_hash, role FROM users WHERE username = $1")
        .bind(&payload.username)
        .fetch_one(&state.pool)
        .await
        .map_err(|err| match err {
            sqlx::Error::RowNotFound => AuthError::Unauthorized("invalid credentials".to_string()),
            other => AuthError::Internal(other.to_string()),
        })?;

    let stored_hash: String = row.get("password_hash");
    if !bcrypt::verify(&payload.password, &stored_hash)
        .map_err(|err| AuthError::Internal(err.to_string()))?
    {
        return Err(AuthError::Unauthorized("invalid credentials".to_string()));
    }
    let user_id: i32 = row.get("id");
    let role: String = row.get("role");

    let claims = Claims::new(user_id, &payload.username, &role, &state.jwt);
    let token = encode(
        &Header::default(),
        &claims,
        &EncodingKey::from_secret(&state.jwt.secret),
    )
    .map_err(|err| AuthError::Internal(err.to_string()))?;

    Ok(Json(LoginResponse {
        token,
        expires_at: chrono::DateTime::<Utc>::from_timestamp(claims.exp as i64, 0)
            .expect("valid expiration timestamp"),
    }))
}

async fn list_api_keys(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<ListApiKeysResponse>, AuthError> {
    let user = authenticate(&headers, &state).await?;
    let records = sqlx::query(
        "SELECT id, name, created_at, last_used_at FROM api_keys WHERE user_id = $1 ORDER BY created_at DESC",
    )
    .bind(user.user_id)
    .fetch_all(&state.pool)
    .await
    .map_err(|err| AuthError::Internal(err.to_string()))?;

    let keys = records
        .into_iter()
        .map(|row| ApiKeySummary {
            id: row.get("id"),
            name: row.get("name"),
            created_at: row.get("created_at"),
            last_used_at: row.get("last_used_at"),
        })
        .collect();

    Ok(Json(ListApiKeysResponse { keys }))
}

async fn create_api_key(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<CreateApiKeyRequest>,
) -> Result<Json<CreateApiKeyResponse>, AuthError> {
    let user = authenticate(&headers, &state).await?;
    let mut name = payload
        .name
        .unwrap_or_else(|| format!("key-{}", Utc::now().timestamp()));
    name.truncate(128);
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err(AuthError::BadRequest("name must not be empty".to_string()));
    }
    let normalized_name = trimmed.to_string();

    let api_key = generate_api_key();
    let hash = hash_api_key(&api_key);

    let record = sqlx::query(
        "INSERT INTO api_keys (user_id, name, api_key_hash) VALUES ($1, $2, $3) RETURNING id, created_at",
    )
    .bind(user.user_id)
    .bind(&normalized_name)
    .bind(&hash)
    .fetch_one(&state.pool)
    .await
    .map_err(|err| AuthError::Internal(err.to_string()))?;

    Ok(Json(CreateApiKeyResponse {
        id: record.get("id"),
        name: normalized_name,
        key: api_key,
        created_at: record.get("created_at"),
    }))
}

async fn delete_api_key(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, AuthError> {
    let user = authenticate(&headers, &state).await?;
    let result = sqlx::query("DELETE FROM api_keys WHERE id = $1 AND user_id = $2")
        .bind(id)
        .bind(user.user_id)
        .execute(&state.pool)
        .await
        .map_err(|err| AuthError::Internal(err.to_string()))?;

    if result.rows_affected() == 0 {
        return Err(AuthError::NotFound("api key not found".to_string()));
    }

    Ok(StatusCode::NO_CONTENT)
}

async fn authenticate(
    headers: &HeaderMap,
    state: &AppState,
) -> Result<AuthenticatedUser, AuthError> {
    let authorization = headers
        .get(axum::http::header::AUTHORIZATION)
        .ok_or_else(|| AuthError::Unauthorized("missing authorization header".to_string()))?;
    let authorization = authorization
        .to_str()
        .map_err(|_| AuthError::Unauthorized("invalid authorization header".to_string()))?;
    let token = authorization
        .strip_prefix("Bearer ")
        .ok_or_else(|| AuthError::Unauthorized("unsupported authorization scheme".to_string()))?;

    let validation = state.jwt.validation();
    let token_data = decode::<Claims>(
        token,
        &DecodingKey::from_secret(&state.jwt.secret),
        &validation,
    )
    .map_err(|_| AuthError::Unauthorized("invalid token".to_string()))?;
    let claims = token_data.claims;

    let row = sqlx::query("SELECT username, role FROM users WHERE id = $1")
        .bind(claims.sub)
        .fetch_one(&state.pool)
        .await
        .map_err(|err| match err {
            sqlx::Error::RowNotFound => AuthError::Unauthorized("user not found".to_string()),
            other => AuthError::Internal(other.to_string()),
        })?;

    Ok(AuthenticatedUser {
        user_id: claims.sub,
        username: row.get("username"),
        role: row.get("role"),
    })
}

fn generate_api_key() -> String {
    let mut bytes = [0u8; 32];
    OsRng.fill_bytes(&mut bytes);
    format!("cds_{}", hex_encode(bytes))
}

fn hash_api_key(key: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(key.as_bytes());
    hex_encode(hasher.finalize())
}

impl Claims {
    fn new(user_id: i32, username: &str, role: &str, jwt: &JwtConfig) -> Self {
        let now = Utc::now();
        let exp = now + jwt.expiration;
        Self {
            sub: user_id,
            username: username.to_string(),
            role: role.to_string(),
            exp: exp.timestamp() as usize,
            iat: now.timestamp() as usize,
            iss: jwt.issuer.clone(),
            jti: Uuid::new_v4().to_string(),
        }
    }
}

fn validate_role(role: &Option<String>) -> Result<(), AuthError> {
    if let Some(role) = role {
        match role.as_str() {
            "admin" | "developer" | "viewer" => Ok(()),
            _ => Err(AuthError::BadRequest(format!(
                "unsupported role '{}'",
                role
            ))),
        }
    } else {
        Ok(())
    }
}

#[derive(Debug, Deserialize)]
struct RegisterRequest {
    username: String,
    password: String,
    role: Option<String>,
    initial_tokens: Option<i64>,
}

#[derive(Debug, Serialize)]
struct RegisterResponse {
    user_id: i32,
}

#[derive(Debug, Deserialize)]
struct LoginRequest {
    username: String,
    password: String,
}

#[derive(Debug, Serialize)]
struct LoginResponse {
    token: String,
    expires_at: chrono::DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
struct CreateApiKeyRequest {
    #[serde(default)]
    name: Option<String>,
}

#[derive(Debug, Serialize)]
struct CreateApiKeyResponse {
    id: Uuid,
    name: String,
    key: String,
    created_at: chrono::DateTime<Utc>,
}

#[derive(Debug, Serialize)]
struct ListApiKeysResponse {
    keys: Vec<ApiKeySummary>,
}

#[derive(Debug, Serialize)]
struct ApiKeySummary {
    id: Uuid,
    name: String,
    created_at: chrono::DateTime<Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    last_used_at: Option<chrono::DateTime<Utc>>,
}

#[derive(Debug, thiserror::Error)]
enum AuthError {
    #[error("bad request: {0}")]
    BadRequest(String),
    #[error("unauthorized: {0}")]
    Unauthorized(String),
    #[error("conflict: {0}")]
    Conflict(String),
    #[error("not found: {0}")]
    NotFound(String),
    #[error("internal error: {0}")]
    Internal(String),
}

impl IntoResponse for AuthError {
    fn into_response(self) -> Response {
        let (status, message) = match &self {
            AuthError::BadRequest(msg) => (StatusCode::BAD_REQUEST, msg.clone()),
            AuthError::Unauthorized(msg) => (StatusCode::UNAUTHORIZED, msg.clone()),
            AuthError::Conflict(msg) => (StatusCode::CONFLICT, msg.clone()),
            AuthError::NotFound(msg) => (StatusCode::NOT_FOUND, msg.clone()),
            AuthError::Internal(msg) => (StatusCode::INTERNAL_SERVER_ERROR, msg.clone()),
        };
        error!("auth error", %message, kind = ?self);
        let body = Json(serde_json::json!({
            "error": message,
        }));
        (status, body).into_response()
    }
}
