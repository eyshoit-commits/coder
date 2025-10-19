use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::Duration;

use axum::body::to_bytes;
use axum::body::Body;
use axum::http::Request;
use axum::routing::{get, post};
use axum::{Json, Router};
use base64::Engine;
use cyberdev_sandbox::{
    execute_with_config, write_file, ExecuteConfig, ExecutionResult, FsError, RunError,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tracing::{error, info, Level};
use tracing_subscriber::FmtSubscriber;

fn app_router() -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/rpc", post(handle_rpc))
}

#[tokio::main]
async fn main() {
    let subscriber = FmtSubscriber::builder()
        .with_max_level(Level::INFO)
        .with_env_filter("info")
        .finish();

    tracing::subscriber::set_global_default(subscriber).expect("setting tracing subscriber failed");

    let app = app_router();

    let port: u16 = std::env::var("RPC_PORT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(6813);
    let addr = SocketAddr::from(([0, 0, 0, 0], port));

    info!("Starting API gateway", %port);
    axum::Server::bind(&addr)
        .serve(app.into_make_service())
        .await
        .expect("server crashed");
}

async fn health() -> &'static str {
    "ok"
}

async fn handle_rpc(Json(request): Json<RpcRequest>) -> Json<RpcResponse> {
    let id = request.id.clone();
    match dispatch(request) {
        Ok(result) => Json(RpcResponse::success(id, result)),
        Err(err) => {
            error!("rpc_error" = %err.message, code = err.code, data = ?err.data);
            Json(RpcResponse::error(id, err))
        }
    }
}

fn dispatch(request: RpcRequest) -> Result<Value, RpcErrorResponse> {
    match request.method.as_str() {
        "fs.write" => handle_fs_write(request.params),
        "run.exec" => handle_run_exec(request.params),
        other => Err(RpcErrorResponse::method_not_found(other)),
    }
}

fn handle_fs_write(params: Value) -> Result<Value, RpcErrorResponse> {
    let params: FsWriteParams = serde_json::from_value(params)
        .map_err(|err| RpcErrorResponse::invalid_params(err.to_string()))?;

    let data = match params.encoding {
        Encoding::Utf8 => params.contents.into_bytes(),
        Encoding::Base64 => base64::engine::general_purpose::STANDARD
            .decode(&params.contents)
            .map_err(|err| {
                RpcErrorResponse::invalid_params(format!("invalid base64 contents: {err}"))
            })?,
    };

    let bytes = data.len();
    write_file(&params.path, &data).map_err(RpcErrorResponse::from_fs_error)?;

    Ok(json!({
        "path": params.path,
        "bytes": bytes,
    }))
}

fn handle_run_exec(params: Value) -> Result<Value, RpcErrorResponse> {
    let params: RunExecParams = serde_json::from_value(params)
        .map_err(|err| RpcErrorResponse::invalid_params(err.to_string()))?;

    let mut config = ExecuteConfig::default();
    config.timeout = params.timeout_ms.map(|ms| Duration::from_millis(ms as u64));
    if let Some(dir) = params.working_directory {
        config.working_directory = Some(PathBuf::from(dir));
    }

    let result = execute_with_config(&params.command, &params.args, config)
        .map_err(RpcErrorResponse::from_run_error)?;

    Ok(serialize_execution_result(&result))
}

fn serialize_execution_result(result: &ExecutionResult) -> Value {
    let exit_code = result.status.code();
    json!({
        "status": {
            "success": result.status.success(),
            "code": exit_code,
        },
        "stdout": result.stdout,
        "stderr": result.stderr,
        "duration_ms": result.duration.as_millis() as u64,
    })
}

#[derive(Debug, Deserialize)]
struct RpcRequest {
    #[serde(default)]
    jsonrpc: Option<String>,
    method: String,
    #[serde(default)]
    params: Value,
    #[serde(default)]
    id: Option<Value>,
}

#[derive(Debug, Deserialize)]
struct FsWriteParams {
    path: String,
    contents: String,
    #[serde(default)]
    encoding: Encoding,
}

#[derive(Debug, Deserialize)]
struct RunExecParams {
    command: String,
    #[serde(default)]
    args: Vec<String>,
    #[serde(default)]
    timeout_ms: Option<u64>,
    #[serde(default)]
    working_directory: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "lowercase")]
enum Encoding {
    Utf8,
    Base64,
}

impl Default for Encoding {
    fn default() -> Self {
        Encoding::Utf8
    }
}

#[derive(Debug, Serialize)]
struct RpcResponse {
    jsonrpc: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    id: Option<Value>,
    #[serde(flatten)]
    payload: RpcPayload,
}

impl RpcResponse {
    fn success(id: Option<Value>, result: Value) -> Self {
        Self {
            jsonrpc: "2.0",
            id,
            payload: RpcPayload::Result { result },
        }
    }

    fn error(id: Option<Value>, err: RpcErrorResponse) -> Self {
        Self {
            jsonrpc: "2.0",
            id,
            payload: RpcPayload::Error {
                error: RpcErrorObject::from(err),
            },
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(untagged)]
enum RpcPayload {
    Result { result: Value },
    Error { error: RpcErrorObject },
}

#[derive(Debug, Serialize)]
struct RpcErrorObject {
    code: i64,
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    data: Option<Value>,
}

#[derive(Debug)]
struct RpcErrorResponse {
    code: i64,
    message: String,
    data: Option<Value>,
}

impl RpcErrorResponse {
    fn method_not_found(method: &str) -> Self {
        Self {
            code: -32601,
            message: format!("method `{method}` not found"),
            data: None,
        }
    }

    fn invalid_params(message: String) -> Self {
        Self {
            code: -32602,
            message,
            data: None,
        }
    }

    fn server_error(message: impl Into<String>, data: Option<Value>) -> Self {
        Self {
            code: -32000,
            message: message.into(),
            data,
        }
    }

    fn from_fs_error(error: FsError) -> Self {
        match error {
            FsError::AbsolutePath(path) => Self::invalid_params(format!(
                "absolute paths are not permitted: {}",
                path.display()
            )),
            FsError::TraversalAttempt => {
                Self::invalid_params("path traversal outside the workspace is not allowed".into())
            }
            FsError::FileTooLarge { size, limit } => {
                Self::invalid_params(format!("file size {size} exceeds limit of {limit} bytes"))
            }
            other => Self::server_error(
                "filesystem operation failed",
                Some(json!({
                    "kind": "FsError",
                    "details": other.to_string(),
                })),
            ),
        }
    }

    fn from_run_error(error: RunError) -> Self {
        match error {
            RunError::CommandNotAllowed(command) => Self {
                code: -32010,
                message: format!("command `{command}` is not permitted"),
                data: Some(json!({ "command": command })),
            },
            RunError::Timeout(duration) => Self {
                code: -32011,
                message: "command timed out".into(),
                data: Some(json!({ "timeout_ms": duration.as_millis() })),
            },
            RunError::OutputLimit { stream, limit } => Self {
                code: -32012,
                message: format!("{stream} output exceeded limit"),
                data: Some(json!({ "stream": stream.to_string(), "limit": limit })),
            },
            RunError::Workspace(error) => Self::from_fs_error(error),
            RunError::MissingStream { stream } => {
                Self::server_error(format!("child process missing {stream} stream"), None)
            }
            RunError::ReaderThread => Self::server_error("output reader thread failed", None),
            RunError::Io(error) => Self::server_error(
                "io error while executing command",
                Some(json!({ "details": error.to_string() })),
            ),
        }
    }
}

impl From<RpcErrorResponse> for RpcErrorObject {
    fn from(value: RpcErrorResponse) -> Self {
        Self {
            code: value.code,
            message: value.message,
            data: value.data,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use axum::http::StatusCode;
    use serde_json::json;
    use tower::ServiceExt;

    use cyberdev_sandbox::{read_file, WORKSPACE_ROOT_ENV};

    async fn send_rpc(router: Router, payload: Value) -> (StatusCode, Value) {
        let request = Request::post("/rpc")
            .header("content-type", "application/json")
            .body(Body::from(
                serde_json::to_vec(&payload).expect("serialize payload"),
            ))
            .expect("request");

        let response = router.oneshot(request).await.expect("router response");

        let status = response.status();
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("read body");
        let value: Value = serde_json::from_slice(&body).expect("decode json");
        (status, value)
    }

    #[tokio::test]
    async fn fs_write_roundtrip() {
        let temp_dir = tempfile::tempdir().expect("create workspace");
        std::env::set_var(WORKSPACE_ROOT_ENV, temp_dir.path());

        let (status, body) = send_rpc(
            app_router(),
            json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "fs.write",
                "params": {
                    "path": "hello.txt",
                    "contents": "cyberdev",
                }
            }),
        )
        .await;

        std::env::remove_var(WORKSPACE_ROOT_ENV);

        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["result"]["path"], "hello.txt");
        assert_eq!(body["result"]["bytes"], 8);

        std::env::set_var(WORKSPACE_ROOT_ENV, temp_dir.path());
        let contents = read_file("hello.txt").expect("read file");
        assert_eq!(contents, "cyberdev");
        std::env::remove_var(WORKSPACE_ROOT_ENV);
    }

    #[tokio::test]
    async fn run_exec_invokes_command() {
        let temp_dir = tempfile::tempdir().expect("create workspace");
        std::env::set_var(WORKSPACE_ROOT_ENV, temp_dir.path());

        let (status, body) = send_rpc(
            app_router(),
            json!({
                "jsonrpc": "2.0",
                "id": 2,
                "method": "run.exec",
                "params": {
                    "command": "sh",
                    "args": ["-c", "printf done"],
                }
            }),
        )
        .await;

        std::env::remove_var(WORKSPACE_ROOT_ENV);

        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["result"]["status"]["success"], true);
        assert_eq!(body["result"]["stdout"], "done");
        assert_eq!(body["result"]["stderr"], "");
    }

    #[tokio::test]
    async fn run_exec_blocks_disallowed_commands() {
        let temp_dir = tempfile::tempdir().expect("create workspace");
        std::env::set_var(WORKSPACE_ROOT_ENV, temp_dir.path());

        let (status, body) = send_rpc(
            app_router(),
            json!({
                "jsonrpc": "2.0",
                "id": 3,
                "method": "run.exec",
                "params": {
                    "command": "rm"
                }
            }),
        )
        .await;

        std::env::remove_var(WORKSPACE_ROOT_ENV);

        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["error"]["code"], -32010);
        assert!(body["error"]["message"]
            .as_str()
            .unwrap()
            .contains("not permitted"));
    }
}
