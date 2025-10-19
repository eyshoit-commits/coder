use std::net::SocketAddr;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;
use chrono::{DateTime, Utc};
use hex::encode as hex_encode;
use jsonwebtoken::{decode, Algorithm, DecodingKey, Validation};
use opentelemetry::global;
use opentelemetry::metrics::{Counter, Histogram, UpDownCounter};
use opentelemetry::sdk::trace::{self, Config};
use opentelemetry::sdk::Resource;
use opentelemetry::KeyValue;
use opentelemetry_otlp::WithExportConfig;
use opentelemetry_prometheus::PrometheusExporter;
use prometheus::{Encoder, TextEncoder};
use reqwest::{header::AUTHORIZATION, Client, Method, StatusCode as HttpStatus};
use sandbox::micro::{
    MicroConfig, MicroExecuteRequest, MicroImage, MicroStartRequest, SandboxMicro,
};
use sandbox::run::{RunConfig, RunRequest, SandboxRun};
use sandbox::{
    AgentContext, AgentContextFile, AgentDispatchRequest, AgentDispatcher, AgentDispatcherConfig,
    AgentFileContent, AgentKind, AgentParameters, SandboxConfig, SandboxError, SandboxFs,
    SandboxWasm, WasmConfig, WasmInvocation, WasmModuleSource, WasmValue,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use sqlx::postgres::PgPoolOptions;
use sqlx::types::Json;
use sqlx::{Error as SqlxError, PgPool, Row};
use tower::ServiceBuilder;
use tower_http::cors::CorsLayer;
use tower_http::trace::TraceLayer;
use tracing::{dispatcher, error, info, warn};
use tracing_opentelemetry::OpenTelemetryLayer;
use tracing_subscriber::prelude::*;
use uuid::Uuid;

struct AppMetrics {
    exporter: PrometheusExporter,
    request_counter: Counter<u64>,
    request_duration: Histogram<f64>,
    sandbox_counter: Counter<u64>,
    active_sessions: UpDownCounter<i64>,
}

impl AppMetrics {
    fn new() -> anyhow::Result<Self> {
        let exporter = opentelemetry_prometheus::exporter().init()?;
        let meter = global::meter("cyberdevstudio.api");
        let request_counter = meter
            .u64_counter("api_requests_total")
            .with_description("Total JSON-RPC requests processed by the API gateway")
            .init();
        let request_duration = meter
            .f64_histogram("api_request_duration_seconds")
            .with_description("Latency of JSON-RPC method execution in seconds")
            .init();
        let sandbox_counter = meter
            .u64_counter("sandbox_operations_total")
            .with_description("Sandbox operations executed by engine and operation")
            .init();
        let active_sessions = meter
            .i64_up_down_counter("active_sessions")
            .with_description("Concurrent JSON-RPC requests being processed")
            .init();
        Ok(Self {
            exporter,
            request_counter,
            request_duration,
            sandbox_counter,
            active_sessions,
        })
    }

    fn session_started(&self) {
        self.active_sessions.add(1, &[]);
    }

    fn session_finished(&self) {
        self.active_sessions.add(-1, &[]);
    }

    fn record_request(
        &self,
        method: &str,
        status: &str,
        duration: Duration,
        role: &str,
        auth_source: &str,
        error_code: Option<i32>,
    ) {
        let mut attributes = vec![
            KeyValue::new("method", method.to_string()),
            KeyValue::new("status", status.to_string()),
            KeyValue::new("role", role.to_string()),
            KeyValue::new("auth_source", auth_source.to_string()),
        ];
        if let Some(code) = error_code {
            attributes.push(KeyValue::new("error_code", code.to_string()));
        }
        self.request_counter.add(1, &attributes);
        self.request_duration
            .record(duration.as_secs_f64(), &attributes);
    }

    fn record_sandbox_op(&self, engine: &'static str, operation: &'static str, success: bool) {
        let status = if success { "success" } else { "error" };
        self.sandbox_counter.add(
            1,
            &[
                KeyValue::new("engine", engine),
                KeyValue::new("operation", operation),
                KeyValue::new("status", status),
            ],
        );
    }

    fn render(&self) -> anyhow::Result<String> {
        let metric_families = self.exporter.registry().gather();
        let mut buffer = Vec::new();
        TextEncoder::new().encode(&metric_families, &mut buffer)?;
        Ok(String::from_utf8(buffer)?)
    }
}

#[derive(Clone)]
struct AppState {
    sandbox: Arc<SandboxFs>,
    run: Arc<SandboxRun>,
    wasm: Arc<SandboxWasm>,
    micro: Arc<SandboxMicro>,
    agents: Arc<AgentDispatcher>,
    pool: PgPool,
    auth: JwtVerifier,
    llm: LlmClient,
    metrics: Arc<AppMetrics>,
}

#[derive(Clone)]
struct JwtVerifier {
    decoding: DecodingKey,
    validation: Validation,
}

impl JwtVerifier {
    fn from_env() -> anyhow::Result<Self> {
        let secret = std::env::var("API_JWT_SECRET")
            .or_else(|_| std::env::var("AUTH_JWT_SECRET"))
            .map_err(|_| anyhow::anyhow!("API_JWT_SECRET environment variable is required"))?;
        let issuer =
            std::env::var("API_JWT_ISSUER").unwrap_or_else(|_| "cyber-dev-studio".to_string());
        let mut validation = Validation::new(Algorithm::HS256);
        validation
            .set_required_spec_claims(&["exp", "iat", "sub", "iss"])
            .expect("required claim configuration");
        validation.iss = Some(issuer);
        Ok(Self {
            decoding: DecodingKey::from_secret(secret.as_bytes()),
            validation,
        })
    }

    fn verify(&self, token: &str) -> std::result::Result<Claims, RpcMethodError> {
        decode::<Claims>(token, &self.decoding, &self.validation)
            .map(|data| data.claims)
            .map_err(|_| RpcMethodError::unauthorized("invalid token"))
    }
}

#[derive(Debug, Deserialize)]
struct Claims {
    sub: i32,
    username: String,
    role: String,
    exp: usize,
    iat: usize,
    iss: String,
    jti: String,
}

#[derive(Debug, Clone)]
struct RequestContext {
    user_id: i32,
    username: String,
    role: Role,
    token_balance: i64,
    api_key_id: Option<Uuid>,
}

impl RequestContext {
    fn require(&self, permission: Permission) -> std::result::Result<(), RpcMethodError> {
        if self.role.allows(permission) {
            Ok(())
        } else {
            Err(RpcMethodError::forbidden("insufficient permissions"))
        }
    }

    fn auth_source(&self) -> &'static str {
        if self.api_key_id.is_some() {
            "api_key"
        } else {
            "jwt"
        }
    }

    fn ensure_tokens(&self) -> std::result::Result<(), RpcMethodError> {
        if self.token_balance > 0 || self.is_admin() {
            Ok(())
        } else {
            Err(RpcMethodError::new(
                -32092,
                "insufficient token balance",
                Some(json!({ "detail": "recharge required" })),
            ))
        }
    }

    fn is_admin(&self) -> bool {
        matches!(self.role, Role::Admin)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Role {
    Admin,
    Developer,
    Viewer,
}

impl Role {
    fn parse(value: &str) -> Option<Self> {
        match value {
            "admin" => Some(Role::Admin),
            "developer" => Some(Role::Developer),
            "viewer" => Some(Role::Viewer),
            _ => None,
        }
    }

    fn allows(self, permission: Permission) -> bool {
        match permission {
            Permission::FsRead | Permission::AgentView => true,
            Permission::FsWrite
            | Permission::Execute
            | Permission::AgentControl
            | Permission::LlmUse => matches!(self, Role::Admin | Role::Developer),
            Permission::LlmAdmin => matches!(self, Role::Admin),
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Role::Admin => "admin",
            Role::Developer => "developer",
            Role::Viewer => "viewer",
        }
    }
}

#[derive(Debug, Clone, Copy)]
enum Permission {
    FsRead,
    FsWrite,
    Execute,
    AgentView,
    AgentControl,
    LlmUse,
    LlmAdmin,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    init_tracing()?;
    let metrics = Arc::new(AppMetrics::new()?);
    let bind_addr = resolve_bind_address()?;
    let pool = build_pool().await?;
    let auth = JwtVerifier::from_env()?;
    let (fs_sandbox, run_sandbox, wasm_sandbox, micro_sandbox) = initialize_sandboxes()?;
    let agent_dispatcher = initialize_agent_dispatcher()?;
    let llm = LlmClient::from_env()?;

    let sandbox = Arc::new(fs_sandbox);
    let run = Arc::new(run_sandbox);
    let wasm = Arc::new(wasm_sandbox);
    let micro = Arc::new(micro_sandbox);
    let agents = Arc::new(agent_dispatcher);

    let state = AppState {
        sandbox,
        run,
        wasm,
        micro,
        agents,
        pool,
        auth,
        llm,
        metrics,
    };

    let app = Router::new()
        .route("/health", get(health))
        .route("/metrics", get(metrics_endpoint))
        .route("/rpc", post(handle_rpc))
        .with_state(state)
        .layer(
            ServiceBuilder::new()
                .layer(TraceLayer::new_for_http())
                .layer(CorsLayer::permissive()),
        );

    info!("binding", %bind_addr, "server starting");
    axum::Server::bind(&bind_addr)
        .serve(app.into_make_service())
        .await?;
    opentelemetry::global::shutdown_tracer_provider();
    Ok(())
}

fn init_tracing() -> anyhow::Result<()> {
    if dispatcher::has_been_set() {
        return Ok(());
    }

    let env_filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| "info,tower_http=info".into());
    let fmt_layer = tracing_subscriber::fmt::layer()
        .json()
        .with_timer(tracing_subscriber::fmt::time::UtcTime::rfc3339());
    let registry = tracing_subscriber::registry()
        .with(env_filter)
        .with(fmt_layer);

    let otlp_endpoint = std::env::var("OTEL_EXPORTER_OTLP_TRACES_ENDPOINT")
        .or_else(|_| std::env::var("OTEL_EXPORTER_OTLP_ENDPOINT"))
        .ok();

    if let Some(endpoint) = otlp_endpoint {
        let resource = Resource::new(vec![
            KeyValue::new("service.name", "cyberdevstudio-api"),
            KeyValue::new("service.namespace", "cyberdevstudio"),
            KeyValue::new("service.version", env!("CARGO_PKG_VERSION")),
        ]);
        let exporter = opentelemetry_otlp::new_exporter()
            .tonic()
            .with_endpoint(endpoint);
        let tracer = opentelemetry_otlp::new_pipeline()
            .tracing()
            .with_trace_config(Config::default().with_resource(resource))
            .with_exporter(exporter)
            .install_batch(opentelemetry::runtime::Tokio)?;
        registry.with(OpenTelemetryLayer::new(tracer)).try_init()?;
    } else {
        registry.try_init()?;
    }

    Ok(())
}

fn resolve_bind_address() -> anyhow::Result<SocketAddr> {
    let raw = std::env::var("API_BIND_ADDR").unwrap_or_else(|_| "0.0.0.0:6813".to_string());
    Ok(raw.parse()?)
}

async fn build_pool() -> anyhow::Result<PgPool> {
    let database_url = std::env::var("DATABASE_URL")
        .map_err(|_| anyhow::anyhow!("DATABASE_URL environment variable is required"))?;
    let max_connections = std::env::var("API_DATABASE_MAX_CONNECTIONS")
        .ok()
        .and_then(|v| v.parse::<u32>().ok())
        .unwrap_or(10);
    let pool = PgPoolOptions::new()
        .max_connections(max_connections)
        .acquire_timeout(Duration::from_secs(10))
        .connect(&database_url)
        .await?;
    Ok(pool)
}

fn initialize_sandboxes() -> anyhow::Result<(SandboxFs, SandboxRun, SandboxWasm, SandboxMicro)> {
    let max_size = std::env::var("SANDBOX_MAX_FILE_SIZE")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(512 * 1024);
    let root = sandbox_root()?;

    let fs = SandboxFs::new(SandboxConfig::new(root.clone(), max_size)?);

    let allowed_programs = std::env::var("SANDBOX_RUN_ALLOWED")
        .ok()
        .map(|value| {
            value
                .split(',')
                .map(|item| item.trim().to_string())
                .filter(|item| !item.is_empty())
                .collect::<Vec<_>>()
        })
        .filter(|items| !items.is_empty())
        .unwrap_or_else(|| vec!["/bin/sh".to_string(), "/usr/bin/env".to_string()]);

    let env_allowlist = std::env::var("SANDBOX_RUN_ENV_ALLOW")
        .ok()
        .map(|value| {
            value
                .split(',')
                .map(|item| item.trim().to_string())
                .filter(|item| !item.is_empty())
                .collect::<Vec<_>>()
        })
        .unwrap_or_else(|| vec!["PATH".to_string()]);

    let path_env =
        std::env::var("SANDBOX_RUN_PATH").unwrap_or_else(|_| "/usr/bin:/bin".to_string());
    let mut fixed_env = vec![
        ("PATH".to_string(), path_env),
        ("HOME".to_string(), root.to_string_lossy().to_string()),
    ];

    if let Ok(extra_fixed) = std::env::var("SANDBOX_RUN_FIXED_ENV") {
        for pair in extra_fixed
            .split(',')
            .map(|p| p.trim())
            .filter(|p| !p.is_empty())
        {
            if let Some((key, value)) = pair.split_once('=') {
                fixed_env.push((key.trim().to_string(), value.trim().to_string()));
            }
        }
    }

    let default_timeout_ms = std::env::var("SANDBOX_RUN_DEFAULT_TIMEOUT_MS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(10_000);
    let max_timeout_ms = std::env::var("SANDBOX_RUN_MAX_TIMEOUT_MS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(30_000);
    let max_output_bytes_raw = std::env::var("SANDBOX_RUN_MAX_OUTPUT_BYTES")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(512 * 1024);
    let max_output_bytes = usize::try_from(max_output_bytes_raw)
        .map_err(|_| anyhow::anyhow!("SANDBOX_RUN_MAX_OUTPUT_BYTES exceeds platform limits"))?;

    let run_config = RunConfig::new(
        &root,
        allowed_programs,
        env_allowlist,
        fixed_env,
        Duration::from_millis(default_timeout_ms),
        Duration::from_millis(max_timeout_ms),
        max_output_bytes,
    )?;

    let wasm_memory_limit = std::env::var("SANDBOX_WASM_MAX_MEMORY_BYTES")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(64 * 1024 * 1024);
    let wasm_table_limit = std::env::var("SANDBOX_WASM_MAX_TABLE_ELEMENTS")
        .ok()
        .and_then(|v| v.parse::<u32>().ok())
        .unwrap_or(2_048);
    let wasm_default_fuel = std::env::var("SANDBOX_WASM_DEFAULT_FUEL")
        .ok()
        .and_then(|v| v.parse::<u64>().ok());

    let wasm_config = WasmConfig::new(
        root.clone(),
        wasm_memory_limit,
        wasm_table_limit,
        wasm_default_fuel,
    )?;

    let micro_default_timeout_ms = std::env::var("SANDBOX_MICRO_DEFAULT_TIMEOUT_MS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(5_000);
    let micro_max_timeout_ms = std::env::var("SANDBOX_MICRO_MAX_TIMEOUT_MS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(30_000);
    let micro_max_output_bytes_raw = std::env::var("SANDBOX_MICRO_MAX_OUTPUT_BYTES")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(256 * 1024);
    let micro_max_output_bytes = usize::try_from(micro_max_output_bytes_raw)
        .map_err(|_| anyhow::anyhow!("SANDBOX_MICRO_MAX_OUTPUT_BYTES exceeds platform limits"))?;

    let micro_images = resolve_micro_images()?;
    let micro_base_env = resolve_micro_base_env();
    let micro_config = MicroConfig::new(
        &root,
        micro_images,
        Duration::from_millis(micro_default_timeout_ms),
        Duration::from_millis(micro_max_timeout_ms),
        micro_max_output_bytes,
        micro_base_env,
    )?;

    Ok((
        fs,
        SandboxRun::new(run_config),
        SandboxWasm::new(wasm_config),
        SandboxMicro::new(micro_config),
    ))
}

fn initialize_agent_dispatcher() -> anyhow::Result<AgentDispatcher> {
    let endpoint =
        std::env::var("AGENT_LLM_ENDPOINT").unwrap_or_else(|_| "http://localhost:6988".to_string());
    let default_model =
        std::env::var("AGENT_DEFAULT_MODEL").unwrap_or_else(|_| "nous-hermes-2-3b.Q4".to_string());
    let timeout_ms = std::env::var("AGENT_LLM_TIMEOUT_MS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(30_000);
    let history_capacity = std::env::var("AGENT_HISTORY_CAPACITY")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(128);
    let context_limit = std::env::var("AGENT_CONTEXT_LIMIT_BYTES")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(512 * 1024);
    let api_key = std::env::var("AGENT_LLM_API_KEY").ok();

    let config = AgentDispatcherConfig::new(endpoint, default_model)
        .with_timeout(Duration::from_millis(timeout_ms))
        .with_history_capacity(history_capacity)
        .with_context_limit(context_limit)
        .with_api_key(api_key);

    AgentDispatcher::new(config).map_err(|err| anyhow::anyhow!(err.to_string()))
}

fn sandbox_root() -> anyhow::Result<PathBuf> {
    let raw = std::env::var("SANDBOX_ROOT").unwrap_or_else(|_| "./data/sandbox".to_string());
    let path = PathBuf::from(&raw);
    if path.is_absolute() {
        Ok(path)
    } else {
        let cwd = std::env::current_dir()?;
        Ok(cwd.join(path))
    }
}

fn resolve_micro_images() -> anyhow::Result<Vec<MicroImage>> {
    if let Ok(raw) = std::env::var("SANDBOX_MICRO_IMAGES") {
        let definitions: Vec<RawMicroImage> = serde_json::from_str(&raw)
            .map_err(|err| anyhow::anyhow!("failed to parse SANDBOX_MICRO_IMAGES: {err}"))?;
        if definitions.is_empty() {
            anyhow::bail!("SANDBOX_MICRO_IMAGES must define at least one image");
        }
        let mut images = Vec::with_capacity(definitions.len());
        for definition in definitions {
            let extension = definition
                .extension
                .unwrap_or_else(|| guess_extension(&definition.name).to_string());
            let env_pairs = definition
                .env
                .into_iter()
                .map(|pair| (pair.key, pair.value))
                .collect::<Vec<_>>();
            images.push(MicroImage::new(
                definition.name,
                definition.command,
                definition.args,
                extension,
                env_pairs,
            )?);
        }
        Ok(images)
    } else {
        default_micro_images()
    }
}

fn default_micro_images() -> anyhow::Result<Vec<MicroImage>> {
    let python_command = std::env::var("SANDBOX_MICRO_PYTHON")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| detect_binary("python3").unwrap_or_else(|| "python3".to_string()));
    let node_command = std::env::var("SANDBOX_MICRO_NODE")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| detect_binary("node").unwrap_or_else(|| "node".to_string()));

    let mut images = Vec::new();
    images.push(MicroImage::new(
        "python",
        python_command,
        vec!["-u".to_string()],
        "py",
        vec![("PYTHONUNBUFFERED".to_string(), "1".to_string())],
    )?);
    images.push(MicroImage::new(
        "node",
        node_command,
        Vec::new(),
        "js",
        Vec::new(),
    )?);
    Ok(images)
}

fn detect_binary(name: &str) -> Option<String> {
    let path = std::env::var("PATH").ok()?;
    for entry in path
        .split(':')
        .map(|segment| segment.trim())
        .filter(|s| !s.is_empty())
    {
        let candidate = Path::new(entry).join(name);
        if let Ok(metadata) = std::fs::metadata(&candidate) {
            if metadata.is_file() {
                return Some(candidate.to_string_lossy().to_string());
            }
        }
    }
    None
}

fn guess_extension(name: &str) -> &'static str {
    let lower = name.to_ascii_lowercase();
    if lower.contains("python") {
        "py"
    } else if lower.contains("node") || lower.contains("js") {
        "js"
    } else if lower.contains("ruby") {
        "rb"
    } else if lower.contains("go") {
        "go"
    } else {
        "txt"
    }
}

fn resolve_micro_base_env() -> Vec<(String, String)> {
    let path_env = std::env::var("SANDBOX_MICRO_PATH")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| std::env::var("PATH").unwrap_or_else(|_| "/usr/bin:/bin".to_string()));
    let mut base = vec![
        ("PATH".to_string(), path_env),
        ("LANG".to_string(), "C".to_string()),
        ("LC_ALL".to_string(), "C".to_string()),
        ("TERM".to_string(), "dumb".to_string()),
    ];
    if let Ok(extra) = std::env::var("SANDBOX_MICRO_BASE_ENV") {
        for pair in extra
            .split(',')
            .map(|segment| segment.trim())
            .filter(|segment| !segment.is_empty())
        {
            if let Some((key, value)) = pair.split_once('=') {
                base.push((key.trim().to_string(), value.trim().to_string()));
            }
        }
    }
    base
}

async fn health() -> impl IntoResponse {
    (StatusCode::OK, Json(json!({ "status": "ok" })))
}

async fn metrics_endpoint(State(state): State<AppState>) -> Response {
    match state.metrics.render() {
        Ok(body) => (StatusCode::OK, body).into_response(),
        Err(err) => {
            error!(?err, "failed to encode metrics");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "failed to encode metrics",
            )
                .into_response()
        }
    }
}

async fn authenticate_request(
    state: &AppState,
    headers: &HeaderMap,
) -> std::result::Result<RequestContext, RpcMethodError> {
    if let Some(value) = headers.get("x-api-key") {
        if !value.as_bytes().is_empty() {
            return authenticate_with_api_key(state, value).await;
        }
    }

    let authorization = headers
        .get(axum::http::header::AUTHORIZATION)
        .ok_or_else(|| RpcMethodError::unauthorized("missing authorization header"))?;
    let authorization = authorization
        .to_str()
        .map_err(|_| RpcMethodError::unauthorized("invalid authorization header"))?;
    let token = authorization
        .strip_prefix("Bearer ")
        .ok_or_else(|| RpcMethodError::unauthorized("unsupported authorization scheme"))?;
    authenticate_with_jwt(state, token).await
}

async fn authenticate_with_api_key(
    state: &AppState,
    value: &axum::http::HeaderValue,
) -> std::result::Result<RequestContext, RpcMethodError> {
    let api_key = value
        .to_str()
        .map_err(|_| RpcMethodError::unauthorized("invalid api key header"))?;
    if api_key.is_empty() {
        return Err(RpcMethodError::unauthorized("invalid api key"));
    }
    let hash = hash_api_key(api_key);
    let row = sqlx::query(
        "SELECT api_keys.id AS api_key_id, users.id AS user_id, users.username, users.role, users.token_balance \
         FROM api_keys JOIN users ON users.id = api_keys.user_id WHERE api_keys.api_key_hash = $1",
    )
    .bind(&hash)
    .fetch_optional(&state.pool)
    .await
    .map_err(|err| RpcMethodError::internal(&err.to_string()))?;

    let row = row.ok_or_else(|| RpcMethodError::unauthorized("invalid api key"))?;
    let role_str: String = row.get("role");
    let role = Role::parse(&role_str)
        .ok_or_else(|| RpcMethodError::internal("user has unsupported role"))?;

    let api_key_id: Uuid = row.get("api_key_id");
    let context = RequestContext {
        user_id: row.get("user_id"),
        username: row.get("username"),
        role,
        token_balance: row.get("token_balance"),
        api_key_id: Some(api_key_id),
    };

    if let Err(err) = sqlx::query("UPDATE api_keys SET last_used_at = NOW() WHERE id = $1")
        .bind(api_key_id)
        .execute(&state.pool)
        .await
    {
        warn!("failed to update api key usage", error = %err);
    }

    Ok(context)
}

async fn authenticate_with_jwt(
    state: &AppState,
    token: &str,
) -> std::result::Result<RequestContext, RpcMethodError> {
    let claims = state.auth.verify(token)?;
    let row = sqlx::query("SELECT username, role, token_balance FROM users WHERE id = $1")
        .bind(claims.sub)
        .fetch_one(&state.pool)
        .await
        .map_err(|err| match err {
            sqlx::Error::RowNotFound => RpcMethodError::unauthorized("user not found"),
            other => RpcMethodError::internal(&other.to_string()),
        })?;

    let role_str: String = row.get("role");
    let role = Role::parse(&role_str)
        .ok_or_else(|| RpcMethodError::internal("user has unsupported role"))?;

    Ok(RequestContext {
        user_id: claims.sub,
        username: row.get("username"),
        role,
        token_balance: row.get("token_balance"),
        api_key_id: None,
    })
}

fn hash_api_key(key: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(key.as_bytes());
    hex_encode(hasher.finalize())
}

async fn handle_rpc(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<RpcRequest>,
) -> impl IntoResponse {
    let method_label = req.method.clone();
    state.metrics.session_started();
    let start = Instant::now();

    if req.jsonrpc != "2.0" {
        let response = Json(RpcResponse::error(
            req.id,
            -32600,
            "invalid jsonrpc version",
            None,
        ));
        state.metrics.record_request(
            &method_label,
            "invalid",
            start.elapsed(),
            "unknown",
            "unauthenticated",
            Some(-32600),
        );
        state.metrics.session_finished();
        return response;
    }

    let mut role_label = "unknown";
    let mut auth_source = "unauthenticated";
    let ctx = match authenticate_request(&state, &headers).await {
        Ok(ctx) => {
            role_label = ctx.role.as_str();
            auth_source = ctx.auth_source();
            ctx
        }
        Err(err) => {
            error!("authentication failed", message = %err.message);
            let response = Json(RpcResponse::error(req.id, err.code, &err.message, err.data));
            state.metrics.record_request(
                &method_label,
                "unauthorized",
                start.elapsed(),
                role_label,
                auth_source,
                Some(err.code),
            );
            state.metrics.session_finished();
            return response;
        }
    };

    let (response, status, error_code) =
        match process_request(&state, &ctx, req.method, req.params).await {
            Ok(result) => (Json(RpcResponse::success(req.id, result)), "ok", None),
            Err(err) => {
                error!("rpc error", message = %err.message);
                (
                    Json(RpcResponse::error(req.id, err.code, &err.message, err.data)),
                    "error",
                    Some(err.code),
                )
            }
        };
    state.metrics.record_request(
        &method_label,
        status,
        start.elapsed(),
        role_label,
        auth_source,
        error_code,
    );
    state.metrics.session_finished();
    response
}

async fn process_request(
    state: &AppState,
    ctx: &RequestContext,
    method: String,
    params: Option<Value>,
) -> std::result::Result<Value, RpcMethodError> {
    match method.as_str() {
        "fs.read" => {
            ctx.require(Permission::FsRead)?;
            let params: FsPathParams = parse_params(params)?;
            let bytes = state.sandbox.read(Path::new(&params.path)).map_err(|err| {
                state.metrics.record_sandbox_op("fs", "read", false);
                RpcMethodError::from_sandbox(-32001, "failed to read file", err)
            })?;
            state.metrics.record_sandbox_op("fs", "read", true);
            Ok(json!({ "data": BASE64.encode(bytes) }))
        }
        "fs.write" => {
            ctx.require(Permission::FsWrite)?;
            let params: FsWriteParams = parse_params(params)?;
            let data = BASE64.decode(params.data.as_bytes()).map_err(|err| {
                RpcMethodError::new(
                    -32602,
                    "invalid base64 payload",
                    Some(json!({ "detail": err.to_string() })),
                )
            })?;
            state
                .sandbox
                .write(Path::new(&params.path), data)
                .map_err(|err| {
                    state.metrics.record_sandbox_op("fs", "write", false);
                    RpcMethodError::from_sandbox(-32002, "failed to write file", err)
                })?;
            state.metrics.record_sandbox_op("fs", "write", true);
            Ok(json!({ "status": "ok" }))
        }
        "fs.list" => {
            ctx.require(Permission::FsRead)?;
            let params: FsPathParams = parse_params(params)?;
            let entries = state.sandbox.list(Path::new(&params.path)).map_err(|err| {
                state.metrics.record_sandbox_op("fs", "list", false);
                RpcMethodError::from_sandbox(-32003, "failed to list directory", err)
            })?;
            state.metrics.record_sandbox_op("fs", "list", true);
            Ok(serde_json::to_value(entries).expect("serialize entries"))
        }
        "fs.delete" => {
            ctx.require(Permission::FsWrite)?;
            let params: FsPathParams = parse_params(params)?;
            state
                .sandbox
                .delete(Path::new(&params.path))
                .map_err(|err| {
                    state.metrics.record_sandbox_op("fs", "delete", false);
                    RpcMethodError::from_sandbox(-32004, "failed to delete path", err)
                })?;
            state.metrics.record_sandbox_op("fs", "delete", true);
            Ok(json!({ "status": "ok" }))
        }
        "fs.mkdir" => {
            ctx.require(Permission::FsWrite)?;
            let params: FsPathParams = parse_params(params)?;
            state
                .sandbox
                .mkdir(Path::new(&params.path))
                .map_err(|err| {
                    state.metrics.record_sandbox_op("fs", "mkdir", false);
                    RpcMethodError::from_sandbox(-32005, "failed to create directory", err)
                })?;
            state.metrics.record_sandbox_op("fs", "mkdir", true);
            Ok(json!({ "status": "ok" }))
        }
        "project.create" => {
            ctx.require(Permission::FsWrite)?;
            let params: ProjectCreateParams = parse_params(params)?;
            let name = normalize_project_name(&params.name)?;
            let description = params.description.as_ref().map(|d| truncate_description(d));
            let record = create_project(&state.pool, ctx, &name, description.as_deref()).await?;
            let project_root = project_directory_relative(&record.id);
            state.sandbox.mkdir(&project_root).map_err(|err| {
                state.metrics.record_sandbox_op("fs", "mkdir", false);
                RpcMethodError::from_sandbox(-32050, "failed to prepare project", err)
            })?;
            state.metrics.record_sandbox_op("fs", "mkdir", true);
            let activity_name = record.name.clone();
            record_project_activity(
                &state.pool,
                record.id,
                ctx.user_id,
                "project.created",
                Some(json!({ "name": activity_name })),
            )
            .await
            .map_err(|err| map_db_activity_error(err, "failed to record project activity"))?;
            Ok(record.to_value())
        }
        "project.list" => {
            ctx.require(Permission::FsRead)?;
            let projects = list_projects(&state.pool, ctx).await?;
            Ok(Value::Array(projects))
        }
        "project.open" => {
            ctx.require(Permission::FsRead)?;
            let params: ProjectOpenParams = parse_params(params)?;
            let project_id = parse_project_id(&params.project_id)?;
            let record = load_project(&state.pool, ctx, &project_id).await?;
            let include_content = params.include_content.unwrap_or(false);
            let files = project_files(&state.pool, &project_id, include_content).await?;
            Ok(json!({
                "project": record.to_value(),
                "files": files,
            }))
        }
        "project.delete" => {
            ctx.require(Permission::FsWrite)?;
            let params: ProjectIdParams = parse_params(params)?;
            let project_id = parse_project_id(&params.project_id)?;
            let record = load_project(&state.pool, ctx, &project_id).await?;
            delete_project(&state.pool, &project_id).await?;
            let project_root = project_directory_relative(&project_id);
            state.sandbox.delete(&project_root).map_err(|err| {
                state.metrics.record_sandbox_op("fs", "delete", false);
                RpcMethodError::from_sandbox(-32054, "failed to remove project files", err)
            })?;
            state.metrics.record_sandbox_op("fs", "delete", true);
            let name = record.name.clone();
            record_project_activity(
                &state.pool,
                project_id,
                ctx.user_id,
                "project.deleted",
                Some(json!({ "name": name })),
            )
            .await
            .map_err(|err| map_db_activity_error(err, "failed to record project activity"))?;
            Ok(json!({ "status": "ok" }))
        }
        "project.file.save" => {
            ctx.require(Permission::FsWrite)?;
            let params: ProjectFileSaveParams = parse_params(params)?;
            let project_id = parse_project_id(&params.project_id)?;
            let _ = load_project(&state.pool, ctx, &project_id).await?;
            let encoding = params.encoding.unwrap_or_else(|| "base64".to_string());
            if encoding.to_lowercase() != "base64" {
                return Err(RpcMethodError::new(
                    -32602,
                    "unsupported file encoding",
                    Some(json!({ "detail": encoding })),
                ));
            }
            let data = BASE64.decode(params.data.as_bytes()).map_err(|err| {
                RpcMethodError::new(
                    -32602,
                    "invalid base64 payload",
                    Some(json!({ "detail": err.to_string() })),
                )
            })?;
            let relative_path = normalize_project_path(&params.path)?;
            let sha256 = Sha256::digest(&data);
            let saved =
                save_project_file(&state.pool, &project_id, &relative_path, &data, &sha256).await?;
            let project_root = project_directory_relative(&project_id).join(&relative_path);
            state.sandbox.write(project_root, &data).map_err(|err| {
                state.metrics.record_sandbox_op("fs", "write", false);
                RpcMethodError::from_sandbox(-32051, "failed to persist project file", err)
            })?;
            state.metrics.record_sandbox_op("fs", "write", true);
            if let Some(message) = params.message {
                if !message.trim().is_empty() {
                    record_project_activity(
                        &state.pool,
                        project_id,
                        ctx.user_id,
                        "project.file.save",
                        Some(json!({
                            "path": relative_path.to_string_lossy(),
                            "message": message.trim(),
                        })),
                    )
                    .await
                    .map_err(|err| {
                        map_db_activity_error(err, "failed to record project activity")
                    })?;
                }
            } else {
                record_project_activity(
                    &state.pool,
                    project_id,
                    ctx.user_id,
                    "project.file.save",
                    Some(json!({
                        "path": relative_path.to_string_lossy(),
                    })),
                )
                .await
                .map_err(|err| map_db_activity_error(err, "failed to record project activity"))?;
            }
            Ok(saved)
        }
        "project.file.read" => {
            ctx.require(Permission::FsRead)?;
            let params: ProjectFilePathParams = parse_params(params)?;
            let project_id = parse_project_id(&params.project_id)?;
            let _ = load_project(&state.pool, ctx, &project_id).await?;
            let relative_path = normalize_project_path(&params.path)?;
            let file = read_project_file(&state.pool, &project_id, &relative_path).await?;
            Ok(file)
        }
        "project.file.delete" => {
            ctx.require(Permission::FsWrite)?;
            let params: ProjectFilePathParams = parse_params(params)?;
            let project_id = parse_project_id(&params.project_id)?;
            let _ = load_project(&state.pool, ctx, &project_id).await?;
            let relative_path = normalize_project_path(&params.path)?;
            delete_project_file(&state.pool, &project_id, &relative_path).await?;
            let project_root = project_directory_relative(&project_id).join(&relative_path);
            state.sandbox.delete(project_root).map_err(|err| {
                state.metrics.record_sandbox_op("fs", "delete", false);
                RpcMethodError::from_sandbox(-32053, "failed to delete project file", err)
            })?;
            state.metrics.record_sandbox_op("fs", "delete", true);
            record_project_activity(
                &state.pool,
                project_id,
                ctx.user_id,
                "project.file.delete",
                Some(json!({ "path": relative_path.to_string_lossy() })),
            )
            .await
            .map_err(|err| map_db_activity_error(err, "failed to record project activity"))?;
            Ok(json!({ "status": "ok" }))
        }
        "run.exec" => {
            ctx.require(Permission::Execute)?;
            let params: RunExecParams = parse_params(params)?;
            let request = params.into_request()?;
            let result = state.run.execute(request).await.map_err(|err| {
                state.metrics.record_sandbox_op("run", "exec", false);
                RpcMethodError::from_sandbox(-32010, "failed to execute process", err)
            })?;
            state.metrics.record_sandbox_op("run", "exec", true);
            Ok(json!({
                "exit_code": result.exit_code,
                "stdout": BASE64.encode(result.stdout),
                "stderr": BASE64.encode(result.stderr),
                "duration_ms": result.duration.as_millis()
            }))
        }
        "run.describe" => {
            ctx.require(Permission::FsRead)?;
            let config = state.run.config();
            let allowed: Vec<String> = config.allowed_programs().cloned().collect();
            Ok(json!({
                "root": config.root().display().to_string(),
                "allowed_programs": allowed,
                "default_timeout_ms": config.default_timeout().as_millis(),
                "max_timeout_ms": config.max_timeout().as_millis(),
                "max_output_bytes": config.max_output_bytes()
            }))
        }
        "wasm.invoke" => {
            ctx.require(Permission::Execute)?;
            let params: WasmInvokeParams = parse_params(params)?;
            let module_source = resolve_wasm_module(&params)?;
            let wasm_params = params
                .params
                .into_iter()
                .map(WasmParam::into_value)
                .collect::<std::result::Result<Vec<_>, _>>()
                .map_err(|err| RpcMethodError::new(-32602, err.as_str(), None))?;

            let mut invocation =
                WasmInvocation::new(module_source, params.function).with_params(wasm_params);
            if let Some(fuel) = params.fuel {
                invocation = invocation.with_fuel(fuel);
            }
            if let Some(memory) = params.memory_limit {
                invocation = invocation.with_memory_limit(memory);
            }
            if let Some(table) = params.table_elements_limit {
                invocation = invocation.with_table_elements_limit(table);
            }

            let values = state.wasm.invoke(invocation).map_err(|err| {
                state.metrics.record_sandbox_op("wasm", "invoke", false);
                RpcMethodError::from_sandbox(-32020, "failed to execute wasm", err)
            })?;
            state.metrics.record_sandbox_op("wasm", "invoke", true);
            let serialized: Vec<Value> = values.into_iter().map(wasm_value_to_json).collect();
            Ok(json!({ "values": serialized }))
        }
        "wasm.describe" => {
            ctx.require(Permission::FsRead)?;
            let config = state.wasm.config();
            Ok(json!({
                "root": config.root().display().to_string(),
                "max_memory_bytes": config.max_memory_bytes(),
                "max_table_elements": config.max_table_elements(),
                "default_fuel": config.default_fuel(),
            }))
        }
        "micro.start" => {
            ctx.require(Permission::Execute)?;
            let params: MicroStartParams = parse_params(params)?;
            let init_script = match params.init_script {
                Some(ref value) if !value.is_empty() => {
                    let bytes = BASE64.decode(value.as_bytes()).map_err(|err| {
                        RpcMethodError::new(
                            -32602,
                            "invalid base64 payload",
                            Some(json!({ "detail": err.to_string() })),
                        )
                    })?;
                    Some(String::from_utf8(bytes).map_err(|err| {
                        RpcMethodError::new(
                            -32602,
                            "init script must be valid utf-8",
                            Some(json!({ "detail": err.to_string() })),
                        )
                    })?)
                }
                _ => None,
            };
            let request = MicroStartRequest {
                image: params.image,
                init_script,
            };
            let instance = state.micro.start(request).await.map_err(|err| {
                state.metrics.record_sandbox_op("micro", "start", false);
                RpcMethodError::from_sandbox(-32030, "failed to start micro vm", err)
            })?;
            state.metrics.record_sandbox_op("micro", "start", true);
            Ok(json!({
                "vm_id": instance.id().to_string(),
                "image": instance.image().to_string(),
                "working_dir": instance.workdir().display().to_string(),
            }))
        }
        "micro.execute" => {
            ctx.require(Permission::Execute)?;
            let params: MicroExecuteParams = parse_params(params)?;
            let vm_id = Uuid::parse_str(&params.vm_id).map_err(|err| {
                RpcMethodError::new(
                    -32602,
                    "invalid vm identifier",
                    Some(json!({ "detail": err.to_string() })),
                )
            })?;
            let code_bytes = BASE64.decode(params.code.as_bytes()).map_err(|err| {
                RpcMethodError::new(
                    -32602,
                    "invalid base64 payload",
                    Some(json!({ "detail": err.to_string() })),
                )
            })?;
            let code = String::from_utf8(code_bytes).map_err(|err| {
                RpcMethodError::new(
                    -32602,
                    "code must be valid utf-8",
                    Some(json!({ "detail": err.to_string() })),
                )
            })?;
            let request = MicroExecuteRequest {
                vm_id,
                code,
                timeout: params.timeout_ms.map(Duration::from_millis),
            };
            let result = state.micro.execute(request).await.map_err(|err| {
                state.metrics.record_sandbox_op("micro", "execute", false);
                RpcMethodError::from_sandbox(-32031, "failed to execute micro vm code", err)
            })?;
            state.metrics.record_sandbox_op("micro", "execute", true);
            Ok(json!({
                "exit_code": result.exit_code,
                "stdout": BASE64.encode(result.stdout),
                "stderr": BASE64.encode(result.stderr),
                "duration_ms": result.duration.as_millis(),
            }))
        }
        "micro.stop" => {
            ctx.require(Permission::Execute)?;
            let params: MicroStopParams = parse_params(params)?;
            let vm_id = Uuid::parse_str(&params.vm_id).map_err(|err| {
                RpcMethodError::new(
                    -32602,
                    "invalid vm identifier",
                    Some(json!({ "detail": err.to_string() })),
                )
            })?;
            state.micro.stop(vm_id).await.map_err(|err| {
                state.metrics.record_sandbox_op("micro", "stop", false);
                RpcMethodError::from_sandbox(-32032, "failed to stop micro vm", err)
            })?;
            state.metrics.record_sandbox_op("micro", "stop", true);
            Ok(json!({ "status": "ok" }))
        }
        "micro.describe" => {
            ctx.require(Permission::FsRead)?;
            let config = state.micro.config();
            let images: Vec<Value> = config
                .images()
                .map(|image| {
                    json!({
                        "name": image.name(),
                        "command": image.command(),
                        "args": image.args().cloned().collect::<Vec<_>>(),
                        "extension": image.extension(),
                        "env": image
                            .env()
                            .map(|(key, value)| json!({ "key": key, "value": value }))
                            .collect::<Vec<_>>(),
                    })
                })
                .collect();
            let base_env: Vec<Value> = config
                .base_env()
                .iter()
                .map(|(key, value)| json!({ "key": key, "value": value }))
                .collect();
            Ok(json!({
                "root": config.root().display().to_string(),
                "default_timeout_ms": config.default_timeout().as_millis(),
                "max_timeout_ms": config.max_timeout().as_millis(),
                "max_output_bytes": config.max_output_bytes(),
                "images": images,
                "base_env": base_env,
            }))
        }
        "llm.chat" => {
            ctx.require(Permission::LlmUse)?;
            ctx.ensure_tokens()?;
            let params: LlmChatParams = parse_params(params)?;
            state.llm.chat(ctx, params).await
        }
        "llm.completion" | "llm.completions" => {
            ctx.require(Permission::LlmUse)?;
            ctx.ensure_tokens()?;
            let params: LlmCompletionParams = parse_params(params)?;
            state.llm.completion(ctx, params).await
        }
        "llm.embed" => {
            ctx.require(Permission::LlmUse)?;
            ctx.ensure_tokens()?;
            let params: LlmEmbedParams = parse_params(params)?;
            state.llm.embed(ctx, params).await
        }
        "llm.list_models" => {
            ctx.require(Permission::LlmAdmin)?;
            state.llm.list_models().await
        }
        "llm.status" => {
            ctx.require(Permission::LlmAdmin)?;
            state.llm.status().await
        }
        "llm.download" => {
            ctx.require(Permission::LlmAdmin)?;
            let params: LlmModelParams = parse_params(params)?;
            state.llm.download(ctx, &params).await
        }
        "llm.start" => {
            ctx.require(Permission::LlmAdmin)?;
            let params: LlmAdminLoadParams = parse_params(params)?;
            state.llm.load(ctx, params).await
        }
        "llm.stop" => {
            ctx.require(Permission::LlmAdmin)?;
            let params: LlmModelParams = parse_params(params)?;
            state.llm.unload(ctx, &params).await
        }
        "agent.list" => {
            ctx.require(Permission::AgentView)?;
            let agents = state.agents.list_agents();
            Ok(serde_json::to_value(agents).expect("serialize agents"))
        }
        "agent.history" => {
            ctx.require(Permission::AgentView)?;
            let params: AgentHistoryParams = parse_params(params)?;
            let mut limit = params.limit.unwrap_or(20);
            if limit == 0 {
                limit = 1;
            }
            if limit > 256 {
                limit = 256;
            }
            let history = state.agents.history(limit);
            Ok(serde_json::to_value(history).expect("serialize history"))
        }
        "agent.status" => {
            ctx.require(Permission::AgentView)?;
            let params: AgentStatusParams = parse_params(params)?;
            let task_id = Uuid::parse_str(&params.task_id).map_err(|err| {
                RpcMethodError::new(
                    -32602,
                    "invalid task identifier",
                    Some(json!({ "detail": err.to_string() })),
                )
            })?;
            let snapshot = state
                .agents
                .status(&task_id)
                .ok_or_else(|| RpcMethodError::new(-32041, "agent task not found", None))?;
            Ok(serde_json::to_value(snapshot).expect("serialize status"))
        }
        "agent.cancel" => {
            ctx.require(Permission::AgentControl)?;
            let params: AgentStatusParams = parse_params(params)?;
            let task_id = Uuid::parse_str(&params.task_id).map_err(|err| {
                RpcMethodError::new(
                    -32602,
                    "invalid task identifier",
                    Some(json!({ "detail": err.to_string() })),
                )
            })?;
            let snapshot = state.agents.cancel(&task_id).map_err(|err| {
                RpcMethodError::from_sandbox(-32042, "failed to cancel agent", err)
            })?;
            Ok(serde_json::to_value(snapshot).expect("serialize status"))
        }
        "agent.dispatch" => {
            ctx.require(Permission::AgentControl)?;
            let params: AgentDispatchParams = parse_params(params)?;
            let AgentDispatchParams {
                agent,
                objective,
                context,
                model,
                metadata,
                parameters,
            } = params;
            let context = build_agent_context(&state.sandbox, context).map_err(|err| {
                RpcMethodError::from_sandbox(-32043, "failed to prepare agent context", err)
            })?;
            let parameters = parameters.map(AgentParameterOverrides::into_parameters);
            let metadata = enrich_agent_metadata(metadata, ctx);
            let request = AgentDispatchRequest {
                agent,
                objective,
                context,
                model,
                metadata,
                parameters,
            };
            let submission = state.agents.dispatch(request).map_err(|err| {
                RpcMethodError::from_sandbox(-32040, "failed to dispatch agent", err)
            })?;
            Ok(json!({
                "task_id": submission.id.to_string(),
                "status": submission.status,
            }))
        }
        _ => Err(RpcMethodError::new(-32601, "method not found", None)),
    }
}

#[derive(Clone)]
struct LlmClient {
    http: Client,
    base_url: String,
    admin_token: Option<String>,
}

impl LlmClient {
    fn from_env() -> anyhow::Result<Self> {
        let base_url =
            std::env::var("LLM_SERVER_URL").unwrap_or_else(|_| "http://127.0.0.1:6988".to_string());
        let admin_token = std::env::var("LLM_SERVER_ADMIN_TOKEN").ok();
        let timeout_secs = std::env::var("LLM_HTTP_TIMEOUT_SECS")
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .unwrap_or(30);
        let http = Client::builder()
            .timeout(Duration::from_secs(timeout_secs))
            .build()?;
        Ok(Self {
            http,
            base_url,
            admin_token,
        })
    }

    async fn chat(
        &self,
        ctx: &RequestContext,
        params: LlmChatParams,
    ) -> std::result::Result<Value, RpcMethodError> {
        self.post_user("/v1/chat/completions", &params, ctx).await
    }

    async fn completion(
        &self,
        ctx: &RequestContext,
        params: LlmCompletionParams,
    ) -> std::result::Result<Value, RpcMethodError> {
        self.post_user("/v1/completions", &params, ctx).await
    }

    async fn embed(
        &self,
        ctx: &RequestContext,
        params: LlmEmbedParams,
    ) -> std::result::Result<Value, RpcMethodError> {
        self.post_user("/v1/embeddings", &params, ctx).await
    }

    async fn list_models(&self) -> std::result::Result<Value, RpcMethodError> {
        self.get_admin("/admin/models").await
    }

    async fn status(&self) -> std::result::Result<Value, RpcMethodError> {
        self.get_admin("/admin/status").await
    }

    async fn download(
        &self,
        ctx: &RequestContext,
        params: &LlmModelParams,
    ) -> std::result::Result<Value, RpcMethodError> {
        self.post_admin("/admin/download", params, Some(ctx)).await
    }

    async fn load(
        &self,
        ctx: &RequestContext,
        params: LlmAdminLoadParams,
    ) -> std::result::Result<Value, RpcMethodError> {
        self.post_admin("/admin/load", &params, Some(ctx)).await
    }

    async fn unload(
        &self,
        ctx: &RequestContext,
        params: &LlmModelParams,
    ) -> std::result::Result<Value, RpcMethodError> {
        self.post_admin("/admin/unload", params, Some(ctx)).await
    }

    async fn post_user<T: Serialize>(
        &self,
        path: &str,
        body: &T,
        ctx: &RequestContext,
    ) -> std::result::Result<Value, RpcMethodError> {
        let request_id = Uuid::new_v4();
        self.send_request(
            Method::POST,
            path,
            Some(body),
            Some(ctx),
            false,
            Some(request_id),
        )
        .await
    }

    async fn post_admin<T: Serialize>(
        &self,
        path: &str,
        body: &T,
        ctx: Option<&RequestContext>,
    ) -> std::result::Result<Value, RpcMethodError> {
        self.send_request(
            Method::POST,
            path,
            Some(body),
            ctx,
            true,
            Some(Uuid::new_v4()),
        )
        .await
    }

    async fn get_admin(&self, path: &str) -> std::result::Result<Value, RpcMethodError> {
        self.send_request::<Value>(Method::GET, path, None, None, true, Some(Uuid::new_v4()))
            .await
    }

    async fn send_request<T: Serialize>(
        &self,
        method: Method,
        path: &str,
        body: Option<&T>,
        ctx: Option<&RequestContext>,
        admin: bool,
        request_id: Option<Uuid>,
    ) -> std::result::Result<Value, RpcMethodError> {
        let url = format!(
            "{}/{}",
            self.base_url.trim_end_matches('/'),
            path.trim_start_matches('/')
        );
        let mut builder = self.http.request(method, url);
        if let Some(ctx) = ctx {
            builder = builder.header("X-User-Id", ctx.user_id.to_string()).header(
                "X-Request-Id",
                request_id.unwrap_or_else(Uuid::new_v4).to_string(),
            );
        } else if let Some(request_id) = request_id {
            builder = builder.header("X-Request-Id", request_id.to_string());
        }
        if admin {
            let token = self
                .admin_token
                .as_ref()
                .ok_or_else(|| RpcMethodError::internal("LLM_SERVER_ADMIN_TOKEN not configured"))?;
            builder = builder.header(AUTHORIZATION, format!("Bearer {token}"));
        }
        if let Some(body) = body {
            builder = builder.json(body);
        }
        let response = builder
            .send()
            .await
            .map_err(|err| RpcMethodError::internal(&err.to_string()))?;
        self.handle_response(response).await
    }

    async fn handle_response(
        &self,
        response: reqwest::Response,
    ) -> std::result::Result<Value, RpcMethodError> {
        let status = response.status();
        let bytes = response
            .bytes()
            .await
            .map_err(|err| RpcMethodError::internal(&err.to_string()))?;
        let body: Value = serde_json::from_slice(&bytes).unwrap_or_else(
            |_| json!({ "error": String::from_utf8_lossy(&bytes).trim().to_string() }),
        );
        if status.is_success() {
            return Ok(body);
        }
        let message = body
            .get("error")
            .and_then(|value| value.as_str())
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| status.canonical_reason().unwrap_or("request failed"));
        let error = match status {
            HttpStatus::UNAUTHORIZED => RpcMethodError::unauthorized(message),
            HttpStatus::FORBIDDEN => RpcMethodError::forbidden(message),
            HttpStatus::TOO_MANY_REQUESTS => RpcMethodError::new(
                -32093,
                "insufficient token balance",
                Some(json!({ "detail": message })),
            ),
            HttpStatus::NOT_FOUND => RpcMethodError::new(-32044, message, Some(body.clone())),
            _ => RpcMethodError::internal(message),
        };
        Err(error)
    }
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
struct LlmChatParams {
    model: String,
    messages: Vec<LlmChatMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(default)]
    temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(default)]
    top_k: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(default)]
    top_p: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(default)]
    repeat_penalty: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(default)]
    max_tokens: Option<u32>,
}

#[derive(Debug, Deserialize, Serialize)]
struct LlmChatMessage {
    role: String,
    content: String,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
struct LlmCompletionParams {
    model: String,
    prompt: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(default)]
    temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(default)]
    top_k: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(default)]
    top_p: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(default)]
    repeat_penalty: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(default)]
    max_tokens: Option<u32>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
struct LlmEmbedParams {
    model: String,
    input: LlmEmbedInput,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(untagged)]
enum LlmEmbedInput {
    Text(String),
    Batch(Vec<String>),
}

#[derive(Debug, Deserialize, Serialize)]
struct LlmModelParams {
    model: String,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
struct LlmAdminLoadParams {
    model: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(default)]
    temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(default)]
    top_k: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(default)]
    top_p: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(default)]
    repeat_penalty: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(default)]
    max_tokens: Option<u32>,
}

#[derive(Debug, Clone)]
struct ProjectRecord {
    id: Uuid,
    owner_id: i32,
    name: String,
    description: Option<String>,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

impl ProjectRecord {
    fn to_value(&self) -> Value {
        json!({
            "id": self.id,
            "owner_id": self.owner_id,
            "name": self.name.clone(),
            "description": self.description.clone(),
            "created_at": self.created_at.to_rfc3339(),
            "updated_at": self.updated_at.to_rfc3339(),
        })
    }
}

fn normalize_project_name(name: &str) -> std::result::Result<String, RpcMethodError> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err(RpcMethodError::new(
            -32602,
            "project name is required",
            None,
        ));
    }
    if trimmed.len() > 128 {
        return Err(RpcMethodError::new(
            -32602,
            "project name must be at most 128 characters",
            Some(json!({ "max": 128 })),
        ));
    }
    Ok(trimmed.to_string())
}

fn truncate_description(value: &str) -> String {
    let trimmed = value.trim();
    let mut result = String::with_capacity(trimmed.len().min(512));
    for ch in trimmed.chars().take(512) {
        result.push(ch);
    }
    result
}

fn project_directory_relative(project_id: &Uuid) -> PathBuf {
    PathBuf::from("projects").join(project_id.to_string())
}

fn parse_project_id(value: &str) -> std::result::Result<Uuid, RpcMethodError> {
    Uuid::parse_str(value).map_err(|err| {
        RpcMethodError::new(
            -32602,
            "invalid project identifier",
            Some(json!({ "detail": err.to_string() })),
        )
    })
}

fn normalize_project_path(path: &str) -> std::result::Result<PathBuf, RpcMethodError> {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return Err(RpcMethodError::new(
            -32602,
            "project path is required",
            None,
        ));
    }
    if trimmed.len() > 512 {
        return Err(RpcMethodError::new(
            -32602,
            "project path must be at most 512 characters",
            Some(json!({ "max": 512 })),
        ));
    }
    let candidate = Path::new(trimmed);
    if candidate.is_absolute() {
        return Err(RpcMethodError::new(
            -32602,
            "project paths must be relative",
            Some(json!({ "path": trimmed })),
        ));
    }
    let mut normalized = PathBuf::new();
    for component in candidate.components() {
        match component {
            Component::Normal(part) => normalized.push(part),
            Component::CurDir => continue,
            _ => {
                return Err(RpcMethodError::new(
                    -32602,
                    "project path cannot traverse parents",
                    Some(json!({ "path": trimmed })),
                ))
            }
        }
    }
    if normalized.as_os_str().is_empty() {
        return Err(RpcMethodError::new(
            -32602,
            "project path cannot resolve to empty",
            Some(json!({ "path": trimmed })),
        ));
    }
    Ok(normalized)
}

async fn create_project(
    pool: &PgPool,
    ctx: &RequestContext,
    name: &str,
    description: Option<&str>,
) -> std::result::Result<ProjectRecord, RpcMethodError> {
    let row = sqlx::query(
        "INSERT INTO projects (user_id, name, description) VALUES ($1, $2, $3) RETURNING id, user_id, name, description, created_at, updated_at",
    )
    .bind(ctx.user_id)
    .bind(name)
    .bind(description)
    .fetch_one(pool)
    .await
    .map_err(|err| match &err {
        SqlxError::Database(db_err) if db_err.code().as_deref() == Some("23505") => {
            RpcMethodError::new(
                -32052,
                "a project with this name already exists",
                Some(json!({ "name": name })),
            )
        }
        _ => RpcMethodError::internal(&format!("failed to create project: {err}")),
    })?;

    Ok(ProjectRecord {
        id: row.get("id"),
        owner_id: row.get("user_id"),
        name: row.get("name"),
        description: row.get("description"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    })
}

async fn list_projects(
    pool: &PgPool,
    ctx: &RequestContext,
) -> std::result::Result<Vec<Value>, RpcMethodError> {
    let rows = if ctx.is_admin() {
        sqlx::query(
            "SELECT id, user_id, name, description, created_at, updated_at FROM projects ORDER BY created_at DESC",
        )
        .fetch_all(pool)
        .await
    } else {
        sqlx::query(
            "SELECT id, user_id, name, description, created_at, updated_at FROM projects WHERE user_id = $1 ORDER BY created_at DESC",
        )
        .bind(ctx.user_id)
        .fetch_all(pool)
        .await
    }
    .map_err(|err| RpcMethodError::internal(&format!("failed to list projects: {err}")))?;

    Ok(rows
        .into_iter()
        .map(|row| {
            let created: DateTime<Utc> = row.get("created_at");
            let updated: DateTime<Utc> = row.get("updated_at");
            json!({
                "id": row.get::<Uuid, _>("id"),
                "owner_id": row.get::<i32, _>("user_id"),
                "name": row.get::<String, _>("name"),
                "description": row.get::<Option<String>, _>("description"),
                "created_at": created.to_rfc3339(),
                "updated_at": updated.to_rfc3339(),
            })
        })
        .collect())
}

async fn load_project(
    pool: &PgPool,
    ctx: &RequestContext,
    project_id: &Uuid,
) -> std::result::Result<ProjectRecord, RpcMethodError> {
    let row = sqlx::query(
        "SELECT id, user_id, name, description, created_at, updated_at FROM projects WHERE id = $1",
    )
    .bind(project_id)
    .fetch_optional(pool)
    .await
    .map_err(|err| RpcMethodError::internal(&format!("failed to load project: {err}")))?;

    let row = row.ok_or_else(|| RpcMethodError::new(-32055, "project not found", None))?;
    let owner_id: i32 = row.get("user_id");
    if owner_id != ctx.user_id && !ctx.is_admin() {
        return Err(RpcMethodError::forbidden("project access denied"));
    }

    Ok(ProjectRecord {
        id: row.get("id"),
        owner_id,
        name: row.get("name"),
        description: row.get("description"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    })
}

async fn project_files(
    pool: &PgPool,
    project_id: &Uuid,
    include_content: bool,
) -> std::result::Result<Vec<Value>, RpcMethodError> {
    let rows = sqlx::query(
        "SELECT path, size, sha256, updated_at, content FROM project_files WHERE project_id = $1 ORDER BY path",
    )
    .bind(project_id)
    .fetch_all(pool)
    .await
    .map_err(|err| RpcMethodError::internal(&format!("failed to load project files: {err}")))?;

    let mut files = Vec::with_capacity(rows.len());
    for row in rows {
        let path: String = row.get("path");
        let size: i64 = row.get("size");
        let sha: Vec<u8> = row.get("sha256");
        let updated: DateTime<Utc> = row.get("updated_at");
        let mut object = serde_json::Map::new();
        object.insert("path".to_string(), Value::String(path));
        object.insert("size".to_string(), Value::Number(size.into()));
        object.insert("sha256".to_string(), Value::String(hex_encode(sha)));
        object.insert(
            "updated_at".to_string(),
            Value::String(updated.to_rfc3339()),
        );
        if include_content {
            let content: Vec<u8> = row.get("content");
            object.insert("data".to_string(), Value::String(BASE64.encode(content)));
        }
        files.push(Value::Object(object));
    }
    Ok(files)
}

async fn delete_project(
    pool: &PgPool,
    project_id: &Uuid,
) -> std::result::Result<(), RpcMethodError> {
    sqlx::query("DELETE FROM projects WHERE id = $1")
        .bind(project_id)
        .execute(pool)
        .await
        .map_err(|err| RpcMethodError::internal(&format!("failed to delete project: {err}")))?;
    Ok(())
}

async fn save_project_file(
    pool: &PgPool,
    project_id: &Uuid,
    path: &Path,
    data: &[u8],
    sha256: &[u8],
) -> std::result::Result<Value, RpcMethodError> {
    let path_str = path.to_string_lossy().to_string();
    let row = sqlx::query(
        "INSERT INTO project_files (project_id, path, content, sha256, size) VALUES ($1, $2, $3, $4, $5)
        ON CONFLICT (project_id, path) DO UPDATE SET content = EXCLUDED.content, sha256 = EXCLUDED.sha256, size = EXCLUDED.size, updated_at = NOW()
        RETURNING updated_at",
    )
    .bind(project_id)
    .bind(&path_str)
    .bind(data)
    .bind(sha256)
    .bind(data.len() as i64)
    .fetch_one(pool)
    .await
    .map_err(|err| RpcMethodError::internal(&format!("failed to save project file: {err}")))?;

    let updated: DateTime<Utc> = row.get("updated_at");
    Ok(json!({
        "status": "ok",
        "path": path_str,
        "size": data.len() as i64,
        "sha256": hex_encode(sha256),
        "updated_at": updated.to_rfc3339(),
    }))
}

async fn read_project_file(
    pool: &PgPool,
    project_id: &Uuid,
    path: &Path,
) -> std::result::Result<Value, RpcMethodError> {
    let path_str = path.to_string_lossy().to_string();
    let row = sqlx::query(
        "SELECT content, size, sha256, updated_at FROM project_files WHERE project_id = $1 AND path = $2",
    )
    .bind(project_id)
    .bind(&path_str)
    .fetch_optional(pool)
    .await
    .map_err(|err| RpcMethodError::internal(&format!("failed to read project file: {err}")))?;

    let row = row.ok_or_else(|| {
        RpcMethodError::new(
            -32052,
            "project file not found",
            Some(json!({ "path": path_str.clone() })),
        )
    })?;
    let content: Vec<u8> = row.get("content");
    let sha: Vec<u8> = row.get("sha256");
    let updated: DateTime<Utc> = row.get("updated_at");
    let size: i64 = row.get("size");

    Ok(json!({
        "path": path_str,
        "data": BASE64.encode(content),
        "size": size,
        "sha256": hex_encode(sha),
        "updated_at": updated.to_rfc3339(),
    }))
}

async fn delete_project_file(
    pool: &PgPool,
    project_id: &Uuid,
    path: &Path,
) -> std::result::Result<(), RpcMethodError> {
    let path_str = path.to_string_lossy().to_string();
    let result = sqlx::query("DELETE FROM project_files WHERE project_id = $1 AND path = $2")
        .bind(project_id)
        .bind(&path_str)
        .execute(pool)
        .await
        .map_err(|err| {
            RpcMethodError::internal(&format!("failed to delete project file: {err}"))
        })?;
    if result.rows_affected() == 0 {
        return Err(RpcMethodError::new(
            -32052,
            "project file not found",
            Some(json!({ "path": path_str })),
        ));
    }
    Ok(())
}

async fn record_project_activity(
    pool: &PgPool,
    project_id: Uuid,
    user_id: i32,
    action: &str,
    detail: Option<Value>,
) -> Result<(), SqlxError> {
    sqlx::query(
        "INSERT INTO project_activity (project_id, user_id, action, detail) VALUES ($1, $2, $3, $4)",
    )
    .bind(project_id)
    .bind(user_id)
    .bind(action)
    .bind(Json(detail.unwrap_or(Value::Null)))
    .execute(pool)
    .await
    .map(|_| ())
}

fn map_db_activity_error(err: SqlxError, message: &str) -> RpcMethodError {
    RpcMethodError::internal(&format!("{message}: {err}"))
}

fn parse_params<T: for<'a> Deserialize<'a>>(
    params: Option<Value>,
) -> std::result::Result<T, RpcMethodError> {
    let value = params.unwrap_or_else(|| Value::Object(Default::default()));
    serde_json::from_value(value).map_err(|err| {
        RpcMethodError::new(
            -32602,
            "invalid params",
            Some(json!({ "detail": err.to_string() })),
        )
    })
}

fn enrich_agent_metadata(metadata: Option<Value>, ctx: &RequestContext) -> Option<Value> {
    let mut map = metadata
        .and_then(|value| value.as_object().cloned())
        .unwrap_or_default();
    map.insert(
        "requested_by".to_string(),
        Value::String(ctx.username.clone()),
    );
    map.insert("requested_by_id".to_string(), json!(ctx.user_id));
    map.insert(
        "auth_source".to_string(),
        Value::String(ctx.auth_source().to_string()),
    );
    map.insert(
        "role".to_string(),
        Value::String(ctx.role.as_str().to_string()),
    );
    if let Some(api_key_id) = ctx.api_key_id {
        map.insert("api_key_id".to_string(), json!(api_key_id));
    }
    Some(Value::Object(map))
}

fn build_agent_context(
    sandbox: &SandboxFs,
    params: Option<AgentDispatchContextParams>,
) -> std::result::Result<AgentContext, SandboxError> {
    let mut context = AgentContext::default();
    if let Some(ctx) = params {
        context.notes = ctx.notes;
        for file in ctx.files {
            let (entry, note) = resolve_agent_context_file(sandbox, file)?;
            if let Some(extra) = note {
                context.notes.push(extra);
            }
            context.files.push(entry);
        }
    }
    Ok(context)
}

fn resolve_agent_context_file(
    sandbox: &SandboxFs,
    params: AgentDispatchContextFileParams,
) -> std::result::Result<(AgentContextFile, Option<String>), SandboxError> {
    let limit = params.max_bytes.unwrap_or(64 * 1024);
    let title = params
        .title
        .clone()
        .or_else(|| params.path.clone())
        .unwrap_or_else(|| "context".to_string());

    if let Some(content_base64) = params.content_base64 {
        let mut bytes = BASE64.decode(content_base64.as_bytes()).map_err(|err| {
            SandboxError::InvalidOperation(format!("invalid base64 inline content: {err}"))
        })?;
        let mut note = None;
        if bytes.len() > limit {
            bytes.truncate(limit);
            note = Some(format!(
                "Inline content '{}' truncated to {} bytes",
                title, limit
            ));
        }
        let encoding = params.encoding.unwrap_or_else(|| "utf-8".to_string());
        let content = if encoding.eq_ignore_ascii_case("utf-8") {
            match String::from_utf8(bytes) {
                Ok(text) => AgentFileContent::Utf8(text),
                Err(err) => {
                    let bytes = err.into_bytes();
                    let encoded = BASE64.encode(&bytes);
                    let detail = format!(
                        "Inline content '{}' was not valid UTF-8; provided as base64",
                        title
                    );
                    note = Some(match note {
                        Some(existing) => format!("{existing}; {detail}"),
                        None => detail,
                    });
                    AgentFileContent::Base64(encoded)
                }
            }
        } else {
            AgentFileContent::Base64(BASE64.encode(&bytes))
        };
        return Ok((
            AgentContextFile {
                path: params.path,
                title,
                content,
            },
            note,
        ));
    }

    let path = params.path.ok_or_else(|| {
        SandboxError::InvalidOperation(
            "context file path is required when no inline content is provided".to_string(),
        )
    })?;
    let mut data = sandbox.read(Path::new(&path))?;
    let mut note = None;
    if data.len() > limit {
        data.truncate(limit);
        note = Some(format!(
            "File '{}' truncated to {} bytes for agent context",
            path, limit
        ));
    }
    let encoding = params.encoding.unwrap_or_else(|| "utf-8".to_string());
    let content = if encoding.eq_ignore_ascii_case("base64") {
        AgentFileContent::Base64(BASE64.encode(&data))
    } else {
        match String::from_utf8(data) {
            Ok(text) => AgentFileContent::Utf8(text),
            Err(err) => {
                let bytes = err.into_bytes();
                let encoded = BASE64.encode(&bytes);
                let detail = format!(
                    "File '{}' contained non UTF-8 data; provided as base64",
                    path
                );
                note = Some(match note {
                    Some(existing) => format!("{existing}; {detail}"),
                    None => detail,
                });
                AgentFileContent::Base64(encoded)
            }
        }
    };
    Ok((
        AgentContextFile {
            path: Some(path),
            title,
            content,
        },
        note,
    ))
}

#[derive(Debug, Deserialize)]
struct RpcRequest {
    jsonrpc: String,
    method: String,
    params: Option<Value>,
    id: Value,
}

#[derive(Debug, Serialize)]
struct RpcResponse {
    jsonrpc: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<RpcError>,
    id: Value,
}

impl RpcResponse {
    fn success(id: Value, result: Value) -> Self {
        Self {
            jsonrpc: "2.0",
            result: Some(result),
            error: None,
            id,
        }
    }

    fn error(id: Value, code: i64, message: &str, data: Option<Value>) -> Self {
        Self {
            jsonrpc: "2.0",
            result: None,
            error: Some(RpcError {
                code,
                message: message.to_string(),
                data,
            }),
            id,
        }
    }
}

#[derive(Debug, Serialize)]
struct RpcError {
    code: i64,
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    data: Option<Value>,
}

#[derive(Debug)]
struct RpcMethodError {
    code: i64,
    message: String,
    data: Option<Value>,
}

impl RpcMethodError {
    fn new(code: i64, message: &str, data: Option<Value>) -> Self {
        Self {
            code,
            message: message.to_string(),
            data,
        }
    }

    fn from_sandbox(code: i64, message: &str, err: sandbox::SandboxError) -> Self {
        Self {
            code,
            message: message.to_string(),
            data: Some(json!({ "detail": err.to_string() })),
        }
    }

    fn unauthorized(message: &str) -> Self {
        Self::new(-32090, message, None)
    }

    fn forbidden(message: &str) -> Self {
        Self::new(-32091, message, None)
    }

    fn internal(detail: &str) -> Self {
        Self::new(-32603, "internal error", Some(json!({ "detail": detail })))
    }
}

#[derive(Debug, Deserialize)]
struct FsPathParams {
    path: String,
}

#[derive(Debug, Deserialize)]
struct FsWriteParams {
    path: String,
    data: String,
}

#[derive(Debug, Deserialize)]
struct ProjectCreateParams {
    name: String,
    #[serde(default)]
    description: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ProjectIdParams {
    project_id: String,
}

#[derive(Debug, Deserialize)]
struct ProjectOpenParams {
    project_id: String,
    #[serde(default)]
    include_content: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct ProjectFileSaveParams {
    project_id: String,
    path: String,
    data: String,
    #[serde(default)]
    encoding: Option<String>,
    #[serde(default)]
    message: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ProjectFilePathParams {
    project_id: String,
    path: String,
}

#[derive(Debug, Deserialize)]
struct RunExecParams {
    program: String,
    #[serde(default)]
    args: Vec<String>,
    #[serde(default)]
    env: Vec<RunEnvVar>,
    #[serde(default)]
    stdin: Option<String>,
    #[serde(default)]
    cwd: Option<String>,
    #[serde(default)]
    timeout_ms: Option<u64>,
}

impl RunExecParams {
    fn into_request(self) -> std::result::Result<RunRequest, RpcMethodError> {
        let mut request = RunRequest::new(self.program);
        if !self.args.is_empty() {
            request.args = self.args;
        }
        if !self.env.is_empty() {
            request.env = self
                .env
                .into_iter()
                .map(|pair| (pair.key, pair.value))
                .collect();
        }
        if let Some(stdin) = self.stdin {
            if !stdin.is_empty() {
                let data = BASE64.decode(stdin.as_bytes()).map_err(|err| {
                    RpcMethodError::new(
                        -32602,
                        "invalid base64 payload",
                        Some(json!({ "detail": err.to_string() })),
                    )
                })?;
                request.stdin = Some(data);
            }
        }
        if let Some(cwd) = self.cwd {
            if !cwd.is_empty() {
                request.working_dir = Some(cwd);
            }
        }
        if let Some(timeout_ms) = self.timeout_ms {
            request.timeout = Some(Duration::from_millis(timeout_ms));
        }
        Ok(request)
    }
}

#[derive(Debug, Deserialize, Clone)]
struct RunEnvVar {
    key: String,
    value: String,
}

#[derive(Debug, Deserialize)]
struct MicroStartParams {
    image: String,
    #[serde(default)]
    init_script: Option<String>,
}

#[derive(Debug, Deserialize)]
struct MicroExecuteParams {
    vm_id: String,
    code: String,
    #[serde(default)]
    timeout_ms: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct MicroStopParams {
    vm_id: String,
}

#[derive(Debug, Deserialize)]
struct RawMicroImage {
    name: String,
    command: String,
    #[serde(default)]
    args: Vec<String>,
    #[serde(default)]
    extension: Option<String>,
    #[serde(default)]
    env: Vec<RunEnvVar>,
}

#[derive(Debug, Deserialize)]
struct AgentDispatchParams {
    agent: AgentKind,
    objective: String,
    #[serde(default)]
    context: Option<AgentDispatchContextParams>,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    metadata: Option<Value>,
    #[serde(default)]
    parameters: Option<AgentParameterOverrides>,
}

#[derive(Debug, Deserialize, Default)]
struct AgentDispatchContextParams {
    #[serde(default)]
    notes: Vec<String>,
    #[serde(default)]
    files: Vec<AgentDispatchContextFileParams>,
}

#[derive(Debug, Deserialize)]
struct AgentDispatchContextFileParams {
    #[serde(default)]
    path: Option<String>,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    encoding: Option<String>,
    #[serde(default)]
    max_bytes: Option<usize>,
    #[serde(default)]
    content_base64: Option<String>,
}

#[derive(Debug, Deserialize)]
struct AgentParameterOverrides {
    #[serde(default)]
    temperature: Option<f32>,
    #[serde(default)]
    max_tokens: Option<u32>,
    #[serde(default)]
    top_p: Option<f32>,
}

impl AgentParameterOverrides {
    fn into_parameters(self) -> AgentParameters {
        let mut params = AgentParameters::default();
        if let Some(temp) = self.temperature {
            params.temperature = temp;
        }
        if let Some(max_tokens) = self.max_tokens {
            params.max_tokens = Some(max_tokens);
        }
        if let Some(top_p) = self.top_p {
            params.top_p = top_p;
        }
        params
    }
}

#[derive(Debug, Deserialize)]
struct AgentStatusParams {
    task_id: String,
}

#[derive(Debug, Deserialize)]
struct AgentHistoryParams {
    #[serde(default)]
    limit: Option<usize>,
}

#[derive(Debug, Deserialize)]
struct WasmInvokeParams {
    #[serde(default)]
    module_path: Option<String>,
    #[serde(default)]
    module_bytes: Option<String>,
    function: String,
    #[serde(default)]
    params: Vec<WasmParam>,
    #[serde(default)]
    fuel: Option<u64>,
    #[serde(default)]
    memory_limit: Option<u64>,
    #[serde(default)]
    table_elements_limit: Option<u32>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", content = "value")]
enum WasmParam {
    #[serde(rename = "i32")]
    I32(i32),
    #[serde(rename = "i64")]
    I64(i64),
    #[serde(rename = "f32")]
    F32(f32),
    #[serde(rename = "f64")]
    F64(f64),
}

impl WasmParam {
    fn into_value(self) -> std::result::Result<WasmValue, String> {
        Ok(match self {
            WasmParam::I32(value) => WasmValue::I32(value),
            WasmParam::I64(value) => WasmValue::I64(value),
            WasmParam::F32(value) => WasmValue::F32(value),
            WasmParam::F64(value) => WasmValue::F64(value),
        })
    }
}

fn resolve_wasm_module(
    params: &WasmInvokeParams,
) -> std::result::Result<WasmModuleSource, RpcMethodError> {
    match (&params.module_path, &params.module_bytes) {
        (Some(_), Some(_)) => Err(RpcMethodError::new(
            -32602,
            "specify either module_path or module_bytes",
            None,
        )),
        (None, None) => Err(RpcMethodError::new(
            -32602,
            "missing wasm module source",
            None,
        )),
        (Some(path), None) => Ok(WasmModuleSource::from_path(path.clone())),
        (None, Some(bytes)) => {
            if bytes.is_empty() {
                return Err(RpcMethodError::new(
                    -32602,
                    "module_bytes must not be empty",
                    None,
                ));
            }
            let decoded = BASE64.decode(bytes.as_bytes()).map_err(|err| {
                RpcMethodError::new(
                    -32602,
                    "invalid base64 payload",
                    Some(json!({ "detail": err.to_string() })),
                )
            })?;
            Ok(WasmModuleSource::from_bytes(decoded))
        }
    }
}

fn wasm_value_to_json(value: WasmValue) -> Value {
    match value {
        WasmValue::I32(v) => json!({ "type": "i32", "value": v }),
        WasmValue::I64(v) => json!({ "type": "i64", "value": v }),
        WasmValue::F32(v) => json!({ "type": "f32", "value": v }),
        WasmValue::F64(v) => json!({ "type": "f64", "value": v }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_project_name_trims_and_limits_length() {
        assert_eq!(normalize_project_name("  demo  ").unwrap(), "demo");
        assert!(normalize_project_name("").is_err());
        let oversized = "a".repeat(129);
        assert!(normalize_project_name(&oversized).is_err());
    }

    #[test]
    fn normalize_project_path_rejects_parent_traversal() {
        assert!(normalize_project_path("../secret").is_err());
        assert!(normalize_project_path("/absolute").is_err());
        let path = normalize_project_path("src/lib.rs").expect("valid path");
        assert_eq!(path.to_string_lossy(), "src/lib.rs");
    }
}
