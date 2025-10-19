use std::collections::{HashMap, VecDeque};
use std::fmt::{Display, Formatter};
use std::sync::Arc;
use std::time::Duration;

use crate::errors::{Result, SandboxError};
use async_trait::async_trait;
use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;
use chrono::{DateTime, Utc};
use parking_lot::Mutex;
use reqwest::StatusCode;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::task;
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, warn};
use uuid::Uuid;

const DEFAULT_HISTORY_CAPACITY: usize = 128;
const DEFAULT_MAX_CONTEXT_BYTES: usize = 512 * 1024; // 512KB

#[derive(Debug, Clone)]
pub struct AgentDispatcherConfig {
    pub llm_endpoint: String,
    pub default_model: String,
    pub request_timeout: Duration,
    pub history_capacity: usize,
    pub max_context_bytes: usize,
    pub api_key: Option<String>,
}

impl AgentDispatcherConfig {
    pub fn new(llm_endpoint: impl Into<String>, default_model: impl Into<String>) -> Self {
        Self {
            llm_endpoint: llm_endpoint.into(),
            default_model: default_model.into(),
            request_timeout: Duration::from_secs(30),
            history_capacity: DEFAULT_HISTORY_CAPACITY,
            max_context_bytes: DEFAULT_MAX_CONTEXT_BYTES,
            api_key: None,
        }
    }

    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.request_timeout = timeout;
        self
    }

    pub fn with_api_key(mut self, api_key: Option<String>) -> Self {
        self.api_key = api_key;
        self
    }

    pub fn with_history_capacity(mut self, capacity: usize) -> Self {
        self.history_capacity = capacity.max(1);
        self
    }

    pub fn with_context_limit(mut self, max_context_bytes: usize) -> Self {
        self.max_context_bytes = max_context_bytes.max(1024);
        self
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash, Copy)]
#[serde(rename_all = "snake_case")]
pub enum AgentKind {
    Code,
    Test,
    Design,
    Debug,
    Security,
    Doc,
}

impl Display for AgentKind {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        let label = match self {
            AgentKind::Code => "code",
            AgentKind::Test => "test",
            AgentKind::Design => "design",
            AgentKind::Debug => "debug",
            AgentKind::Security => "security",
            AgentKind::Doc => "doc",
        };
        f.write_str(label)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AgentContext {
    #[serde(default)]
    pub notes: Vec<String>,
    #[serde(default)]
    pub files: Vec<AgentContextFile>,
}

impl AgentContext {
    pub fn total_bytes(&self) -> Result<usize> {
        let mut total = 0usize;
        for note in &self.notes {
            total = total
                .checked_add(note.as_bytes().len())
                .ok_or_else(|| SandboxError::InvalidOperation("context too large".to_string()))?;
        }
        for file in &self.files {
            total = total
                .checked_add(file.content.bytes_len()?)
                .ok_or_else(|| SandboxError::InvalidOperation("context too large".to_string()))?;
        }
        Ok(total)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentContextFile {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    pub title: String,
    pub content: AgentFileContent,
}

impl AgentContextFile {
    pub fn new_utf8(
        path: Option<String>,
        title: impl Into<String>,
        body: impl Into<String>,
    ) -> Self {
        Self {
            path,
            title: title.into(),
            content: AgentFileContent::Utf8(body.into()),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "encoding", content = "data", rename_all = "snake_case")]
pub enum AgentFileContent {
    #[serde(rename = "utf-8")]
    Utf8(String),
    Base64(String),
}

impl AgentFileContent {
    pub fn bytes_len(&self) -> Result<usize> {
        match self {
            AgentFileContent::Utf8(value) => Ok(value.as_bytes().len()),
            AgentFileContent::Base64(value) => BASE64
                .decode(value.as_bytes())
                .map(|decoded| decoded.len())
                .map_err(|err| {
                    SandboxError::InvalidOperation(format!("invalid base64 context payload: {err}"))
                }),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentParameters {
    pub temperature: f32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u32>,
    pub top_p: f32,
}

impl Default for AgentParameters {
    fn default() -> Self {
        Self {
            temperature: 0.2,
            max_tokens: Some(768),
            top_p: 0.9,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentDispatchRequest {
    pub agent: AgentKind,
    pub objective: String,
    #[serde(default)]
    pub context: AgentContext,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub metadata: Option<Value>,
    #[serde(default)]
    pub parameters: Option<AgentParameters>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentOutcome {
    pub summary: String,
    #[serde(default)]
    pub insights: Vec<String>,
    #[serde(default)]
    pub actions: Vec<AgentAction>,
    pub raw_response: String,
}

impl Default for AgentOutcome {
    fn default() -> Self {
        Self {
            summary: String::new(),
            insights: Vec::new(),
            actions: Vec::new(),
            raw_response: String::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AgentAction {
    Message {
        title: String,
        body: String,
    },
    FilePatch {
        path: String,
        patch: String,
    },
    FileWrite {
        path: String,
        content: AgentFileContent,
    },
    Command {
        command: String,
        #[serde(default)]
        args: Vec<String>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AgentTaskStatus {
    Pending,
    Running,
    Completed,
    Failed,
    Cancelled,
}

impl AgentTaskStatus {
    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            AgentTaskStatus::Completed | AgentTaskStatus::Failed | AgentTaskStatus::Cancelled
        )
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentTaskSnapshot {
    pub id: Uuid,
    pub agent: AgentKind,
    pub status: AgentTaskStatus,
    pub objective: String,
    pub model: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    pub created_at: DateTime<Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub started_at: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub finished_at: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub outcome: Option<AgentOutcome>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<Value>,
    pub parameters: AgentParameters,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentTaskSubmission {
    pub id: Uuid,
    pub status: AgentTaskSnapshot,
}

#[derive(Clone)]
struct AgentTaskEntry {
    agent: AgentKind,
    state: Arc<Mutex<AgentTaskState>>,
    cancellation: CancellationToken,
}

struct AgentTaskState {
    id: Uuid,
    agent: AgentKind,
    objective: String,
    model: String,
    status: AgentTaskStatus,
    created_at: DateTime<Utc>,
    started_at: Option<DateTime<Utc>>,
    finished_at: Option<DateTime<Utc>>,
    outcome: Option<AgentOutcome>,
    error: Option<String>,
    metadata: Option<Value>,
    parameters: AgentParameters,
}

impl AgentTaskState {
    fn new(
        id: Uuid,
        agent: AgentKind,
        objective: String,
        model: String,
        metadata: Option<Value>,
        parameters: AgentParameters,
    ) -> Self {
        Self {
            id,
            agent,
            objective,
            model,
            status: AgentTaskStatus::Pending,
            created_at: Utc::now(),
            started_at: None,
            finished_at: None,
            outcome: None,
            error: None,
            metadata,
            parameters,
        }
    }

    fn snapshot(&self) -> AgentTaskSnapshot {
        AgentTaskSnapshot {
            id: self.id,
            agent: self.agent,
            status: self.status,
            objective: self.objective.clone(),
            model: self.model.clone(),
            summary: self.outcome.as_ref().map(|outcome| outcome.summary.clone()),
            error: self.error.clone(),
            created_at: self.created_at,
            started_at: self.started_at,
            finished_at: self.finished_at,
            outcome: self.outcome.clone(),
            metadata: self.metadata.clone(),
            parameters: self.parameters.clone(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct AgentMetadata {
    pub agent: AgentKind,
    pub name: String,
    pub description: String,
    pub capabilities: Vec<String>,
    pub default_model: String,
    pub default_parameters: AgentParameters,
}

#[derive(Debug, Clone)]
pub struct AgentInvocation {
    pub id: Uuid,
    pub agent: AgentKind,
    pub objective: String,
    pub context: AgentContext,
    pub model: String,
    pub metadata: Option<Value>,
    pub parameters: AgentParameters,
}

#[async_trait]
trait Agent: Send + Sync {
    fn metadata(&self) -> AgentMetadata;
    async fn execute(
        &self,
        invocation: AgentInvocation,
        cancellation: CancellationToken,
    ) -> Result<AgentOutcome>;
}

#[derive(Clone)]
pub struct AgentDispatcher {
    config: AgentDispatcherConfig,
    agents: HashMap<AgentKind, Arc<dyn Agent>>, // each entry already inside Arc
    tasks: Arc<Mutex<HashMap<Uuid, AgentTaskEntry>>>,
    history: Arc<Mutex<VecDeque<AgentTaskSnapshot>>>,
}

impl AgentDispatcher {
    pub fn new(config: AgentDispatcherConfig) -> Result<Self> {
        let client = Arc::new(LlmClient::new(
            config.llm_endpoint.clone(),
            config.request_timeout,
            config.api_key.clone(),
        )?);
        let agents = default_agents(client, config.default_model.clone());
        Self::with_agents(config, agents)
    }

    pub fn with_agents(
        config: AgentDispatcherConfig,
        agents: HashMap<AgentKind, Arc<dyn Agent>>,
    ) -> Result<Self> {
        if agents.is_empty() {
            return Err(SandboxError::InvalidOperation(
                "agent dispatcher requires at least one agent".to_string(),
            ));
        }
        Ok(Self {
            config,
            agents,
            tasks: Arc::new(Mutex::new(HashMap::new())),
            history: Arc::new(Mutex::new(VecDeque::new())),
        })
    }

    pub fn dispatch(&self, request: AgentDispatchRequest) -> Result<AgentTaskSubmission> {
        if request.objective.trim().is_empty() {
            return Err(SandboxError::InvalidOperation(
                "objective must not be empty".to_string(),
            ));
        }

        let agent_impl = self
            .agents
            .get(&request.agent)
            .cloned()
            .ok_or_else(|| SandboxError::AgentUnavailable(request.agent.to_string()))?;

        let context_size = request.context.total_bytes()?;
        if context_size > self.config.max_context_bytes {
            return Err(SandboxError::ContextTooLarge {
                provided: context_size,
                limit: self.config.max_context_bytes,
            });
        }

        let parameters = request.parameters.unwrap_or_default();
        let id = Uuid::new_v4();
        let model = request
            .model
            .unwrap_or_else(|| self.config.default_model.clone());
        let state = Arc::new(Mutex::new(AgentTaskState::new(
            id,
            request.agent,
            request.objective.clone(),
            model.clone(),
            request.metadata.clone(),
            parameters.clone(),
        )));
        let entry = AgentTaskEntry {
            agent: request.agent,
            state: state.clone(),
            cancellation: CancellationToken::new(),
        };
        self.tasks.lock().insert(id, entry.clone());

        let tasks_map = self.tasks.clone();
        let history = self.history.clone();
        let history_capacity = self.config.history_capacity;
        let invocation = AgentInvocation {
            id,
            agent: request.agent,
            objective: request.objective,
            context: request.context,
            model,
            metadata: request.metadata,
            parameters,
        };
        let state_for_task = state.clone();
        let cancellation = entry.cancellation.clone();
        task::spawn(async move {
            {
                let mut guard = state_for_task.lock();
                if guard.status == AgentTaskStatus::Pending {
                    guard.status = AgentTaskStatus::Running;
                    guard.started_at = Some(Utc::now());
                }
            }
            let outcome = agent_impl.execute(invocation, cancellation.clone()).await;
            let mut guard = state_for_task.lock();
            if guard.status == AgentTaskStatus::Cancelled {
                guard.finished_at.get_or_insert_with(Utc::now);
            } else {
                match outcome {
                    Ok(result) => {
                        guard.status = AgentTaskStatus::Completed;
                        guard.finished_at = Some(Utc::now());
                        guard.outcome = Some(result);
                    }
                    Err(err) => match err {
                        SandboxError::Cancelled => {
                            guard.status = AgentTaskStatus::Cancelled;
                            guard.finished_at = Some(Utc::now());
                        }
                        other => {
                            guard.status = AgentTaskStatus::Failed;
                            guard.finished_at = Some(Utc::now());
                            guard.error = Some(other.to_string());
                        }
                    },
                }
            }
            let snapshot = guard.snapshot();
            drop(guard);

            let mut tasks_guard = tasks_map.lock();
            tasks_guard.remove(&snapshot.id);
            drop(tasks_guard);

            let mut history_guard = history.lock();
            history_guard.push_back(snapshot.clone());
            while history_guard.len() > history_capacity {
                history_guard.pop_front();
            }
        });

        let snapshot = state.lock().snapshot();
        Ok(AgentTaskSubmission {
            id,
            status: snapshot,
        })
    }

    pub fn cancel(&self, id: &Uuid) -> Result<AgentTaskSnapshot> {
        let entry = {
            let guard = self.tasks.lock();
            guard
                .get(id)
                .cloned()
                .ok_or_else(|| SandboxError::AgentTaskNotFound(id.to_string()))?
        };
        entry.cancellation.cancel();
        {
            let mut state = entry.state.lock();
            if state.status.is_terminal() {
                return Ok(state.snapshot());
            }
            state.status = AgentTaskStatus::Cancelled;
            state.finished_at = Some(Utc::now());
            Ok(state.snapshot())
        }
    }

    pub fn status(&self, id: &Uuid) -> Option<AgentTaskSnapshot> {
        if let Some(entry) = self.tasks.lock().get(id) {
            return Some(entry.state.lock().snapshot());
        }
        self.history
            .lock()
            .iter()
            .rev()
            .find(|snapshot| &snapshot.id == id)
            .cloned()
    }

    pub fn history(&self, limit: usize) -> Vec<AgentTaskSnapshot> {
        let guard = self.history.lock();
        guard.iter().rev().take(limit).cloned().collect()
    }

    pub fn list_agents(&self) -> Vec<AgentMetadata> {
        let mut entries: Vec<_> = self.agents.values().map(|agent| agent.metadata()).collect();
        entries.sort_by_key(|meta| meta.agent);
        entries
    }
}

struct LlmClient {
    http: reqwest::Client,
    base_url: String,
    api_key: Option<String>,
}

impl LlmClient {
    fn new(base_url: String, timeout: Duration, api_key: Option<String>) -> Result<Self> {
        let http = reqwest::Client::builder()
            .timeout(timeout)
            .build()
            .map_err(|err| SandboxError::InvalidOperation(err.to_string()))?;
        Ok(Self {
            http,
            base_url,
            api_key,
        })
    }

    async fn chat(&self, request: ChatCompletionRequest) -> Result<ChatCompletionResponse> {
        let url = format!(
            "{}/v1/chat/completions",
            self.base_url.trim_end_matches('/')
        );
        let mut req = self.http.post(url).json(&request);
        if let Some(key) = &self.api_key {
            req = req.bearer_auth(key);
        }
        let response = req
            .send()
            .await
            .map_err(|err| SandboxError::Network(err.to_string()))?;
        if !response.status().is_success() {
            let status = response.status();
            let body = response
                .text()
                .await
                .unwrap_or_else(|_| "<unavailable>".to_string());
            return Err(SandboxError::AgentFailed(format!(
                "llm request failed with status {status}: {body}"
            )));
        }
        response
            .json::<ChatCompletionResponse>()
            .await
            .map_err(|err| {
                SandboxError::AgentFailed(format!("invalid llm response payload: {err}"))
            })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ChatCompletionRequest {
    pub model: String,
    pub messages: Vec<ChatMessage>,
    pub temperature: f32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u32>,
    pub top_p: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ChatMessage {
    pub role: String,
    pub content: String,
}

#[derive(Debug, Deserialize)]
struct ChatCompletionResponse {
    pub choices: Vec<ChatCompletionChoice>,
}

#[derive(Debug, Deserialize)]
struct ChatCompletionChoice {
    pub message: ChatMessage,
}

struct LlmBackedAgent {
    kind: AgentKind,
    name: String,
    description: String,
    system_prompt: String,
    capabilities: Vec<String>,
    default_model: String,
    default_parameters: AgentParameters,
    client: Arc<LlmClient>,
}

impl LlmBackedAgent {
    fn new(
        kind: AgentKind,
        name: impl Into<String>,
        description: impl Into<String>,
        system_prompt: impl Into<String>,
        capabilities: Vec<String>,
        default_model: impl Into<String>,
        client: Arc<LlmClient>,
    ) -> Arc<Self> {
        Arc::new(Self {
            kind,
            name: name.into(),
            description: description.into(),
            system_prompt: system_prompt.into(),
            capabilities,
            default_model: default_model.into(),
            default_parameters: AgentParameters::default(),
            client,
        })
    }

    fn build_user_prompt(&self, invocation: &AgentInvocation) -> String {
        let mut prompt = String::new();
        prompt.push_str("Objective:\n");
        prompt.push_str(invocation.objective.trim());
        prompt.push_str("\n\n");
        if !invocation.context.notes.is_empty() {
            prompt.push_str("Context notes:\n");
            for (idx, note) in invocation.context.notes.iter().enumerate() {
                prompt.push_str(&format!("{}. {}\n", idx + 1, note.trim()));
            }
            prompt.push('\n');
        }
        if !invocation.context.files.is_empty() {
            prompt.push_str("Files:\n");
            for file in &invocation.context.files {
                prompt.push_str(&format!("- {}\n", file.title));
                if let Some(path) = &file.path {
                    prompt.push_str(&format!("  Path: {}\n", path));
                }
                match &file.content {
                    AgentFileContent::Utf8(content) => {
                        let snippet: String = content.chars().take(2048).collect();
                        prompt.push_str("  Content snippet:\n");
                        prompt.push_str(&snippet);
                        prompt.push('\n');
                    }
                    AgentFileContent::Base64(_) => {
                        prompt.push_str("  Content provided as base64 (not expanded).\n");
                    }
                }
            }
            prompt.push('\n');
        }
        if let Some(metadata) = &invocation.metadata {
            prompt.push_str("Additional metadata:\n");
            prompt.push_str(&metadata.to_string());
            prompt.push('\n');
        }
        prompt
    }
}

#[async_trait]
impl Agent for LlmBackedAgent {
    fn metadata(&self) -> AgentMetadata {
        AgentMetadata {
            agent: self.kind,
            name: self.name.clone(),
            description: self.description.clone(),
            capabilities: self.capabilities.clone(),
            default_model: self.default_model.clone(),
            default_parameters: self.default_parameters.clone(),
        }
    }

    async fn execute(
        &self,
        invocation: AgentInvocation,
        cancellation: CancellationToken,
    ) -> Result<AgentOutcome> {
        if cancellation.is_cancelled() {
            return Err(SandboxError::Cancelled);
        }
        let model = if invocation.model.trim().is_empty() {
            self.default_model.clone()
        } else {
            invocation.model.clone()
        };
        let mut messages = vec![ChatMessage {
            role: "system".to_string(),
            content: self.system_prompt.clone(),
        }];
        messages.push(ChatMessage {
            role: "user".to_string(),
            content: self.build_user_prompt(&invocation),
        });
        let params = invocation.parameters;
        let request = ChatCompletionRequest {
            model,
            messages,
            temperature: params.temperature,
            max_tokens: params.max_tokens,
            top_p: params.top_p,
        };
        let response = self.client.chat(request).await?;
        let text = response
            .choices
            .first()
            .map(|choice| choice.message.content.clone())
            .unwrap_or_default();

        if cancellation.is_cancelled() {
            return Err(SandboxError::Cancelled);
        }

        let parsed: std::result::Result<LlmAgentPayload, _> = serde_json::from_str(&text);
        let mut outcome = AgentOutcome {
            summary: String::new(),
            insights: Vec::new(),
            actions: Vec::new(),
            raw_response: text.clone(),
        };
        match parsed {
            Ok(payload) => {
                outcome.summary = payload.summary;
                outcome.insights = payload.insights.unwrap_or_default();
                outcome.actions = payload.actions.unwrap_or_default();
            }
            Err(err) => {
                warn!("agent", kind = %self.kind, "failed to parse structured response: {err}");
                outcome.summary = text.trim().to_string();
            }
        }
        if outcome.summary.trim().is_empty() {
            outcome.summary = "agent completed without summary".to_string();
        }
        Ok(outcome)
    }
}

#[derive(Debug, Deserialize)]
struct LlmAgentPayload {
    pub summary: String,
    #[serde(default)]
    pub insights: Option<Vec<String>>,
    #[serde(default)]
    pub actions: Option<Vec<AgentAction>>,
}

fn default_agents(
    client: Arc<LlmClient>,
    default_model: String,
) -> HashMap<AgentKind, Arc<dyn Agent>> {
    let mut agents: HashMap<AgentKind, Arc<dyn Agent>> = HashMap::new();
    let entries = vec![
        (
            AgentKind::Code,
            "Code Synthesis Agent",
            "Generates and refactors application code with production-ready quality.",
            "You are a senior software engineer. Provide precise code changes. Respond as JSON {\"summary\": string, \"insights\": [string], \"actions\": [ { \"type\": \"file_patch\" | \"file_write\" | \"message\" | \"command\", ... } ] }.",
            vec!["code_generation", "refactoring", "analysis"],
        ),
        (
            AgentKind::Test,
            "Test Engineering Agent",
            "Designs and evaluates automated tests ensuring high coverage and reliability.",
            "You are a dedicated test engineer. Produce actionable testing guidance with structured JSON output matching {summary, insights, actions}.",
            vec!["test_design", "coverage_analysis", "ci_recommendation"],
        ),
        (
            AgentKind::Design,
            "Design Review Agent",
            "Creates UI/UX recommendations and design artifacts for the studio interface.",
            "You evaluate user experience, accessibility, and visual design. Respond with structured JSON including summary, insights, and actions.",
            vec!["ux_feedback", "component_layout", "theme_guidance"],
        ),
        (
            AgentKind::Debug,
            "Diagnostics Agent",
            "Performs root-cause analysis for defects and runtime failures.",
            "You act as a debugger focusing on logs, stack traces, and reproduction steps. Return JSON summary, insights, actions.",
            vec!["log_analysis", "failure_triage", "fix_recommendation"],
        ),
        (
            AgentKind::Security,
            "Security Analyst Agent",
            "Audits code for security vulnerabilities and recommends mitigations.",
            "You are an application security expert. Produce structured JSON {summary, insights, actions}.",
            vec!["threat_analysis", "dependency_review", "mitigation_plan"],
        ),
        (
            AgentKind::Doc,
            "Documentation Agent",
            "Produces technical documentation, changelogs, and onboarding guides.",
            "You create accurate documentation. Provide JSON with summary, insights, actions.",
            vec!["api_docs", "changelog", "guides"],
        ),
    ];

    for (kind, name, description, prompt, capabilities) in entries {
        agents.insert(
            kind,
            LlmBackedAgent::new(
                kind,
                name,
                description,
                prompt,
                capabilities,
                default_model.clone(),
                client.clone(),
            ),
        );
    }

    agents
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use tokio::time::sleep;

    struct StubAgent {
        metadata: AgentMetadata,
    }

    #[async_trait]
    impl Agent for StubAgent {
        fn metadata(&self) -> AgentMetadata {
            self.metadata.clone()
        }

        async fn execute(
            &self,
            invocation: AgentInvocation,
            cancellation: CancellationToken,
        ) -> Result<AgentOutcome> {
            if cancellation.is_cancelled() {
                return Err(SandboxError::Cancelled);
            }
            sleep(Duration::from_millis(10)).await;
            if cancellation.is_cancelled() {
                return Err(SandboxError::Cancelled);
            }
            Ok(AgentOutcome {
                summary: format!("handled: {}", invocation.objective),
                insights: vec!["stub insight".to_string()],
                actions: vec![AgentAction::Message {
                    title: "ok".to_string(),
                    body: "completed".to_string(),
                }],
                raw_response: "{}".to_string(),
            })
        }
    }

    fn stub_dispatcher() -> AgentDispatcher {
        let metadata = AgentMetadata {
            agent: AgentKind::Code,
            name: "stub".to_string(),
            description: "stub".to_string(),
            capabilities: vec!["stub".to_string()],
            default_model: "test".to_string(),
            default_parameters: AgentParameters::default(),
        };
        let mut agents: HashMap<AgentKind, Arc<dyn Agent>> = HashMap::new();
        agents.insert(
            AgentKind::Code,
            Arc::new(StubAgent { metadata }) as Arc<dyn Agent>,
        );
        AgentDispatcher::with_agents(
            AgentDispatcherConfig::new("http://localhost", "test"),
            agents,
        )
        .expect("stub dispatcher")
    }

    #[tokio::test]
    async fn dispatch_executes_agent() {
        let dispatcher = stub_dispatcher();
        let submission = dispatcher
            .dispatch(AgentDispatchRequest {
                agent: AgentKind::Code,
                objective: "build module".to_string(),
                context: AgentContext::default(),
                model: None,
                metadata: Some(json!({ "priority": "high" })),
                parameters: None,
            })
            .expect("dispatch success");
        assert_eq!(submission.status.status, AgentTaskStatus::Pending);
        sleep(Duration::from_millis(30)).await;
        let status = dispatcher.status(&submission.id).unwrap();
        assert_eq!(status.status, AgentTaskStatus::Completed);
        assert_eq!(status.outcome.unwrap().summary, "handled: build module");
    }

    #[tokio::test]
    async fn cancel_marks_task() {
        let dispatcher = stub_dispatcher();
        let submission = dispatcher
            .dispatch(AgentDispatchRequest {
                agent: AgentKind::Code,
                objective: "long task".to_string(),
                context: AgentContext::default(),
                model: None,
                metadata: None,
                parameters: None,
            })
            .expect("dispatch success");
        let snapshot = dispatcher.cancel(&submission.id).expect("cancel");
        assert_eq!(snapshot.status, AgentTaskStatus::Cancelled);
    }

    #[tokio::test]
    async fn history_captures_completed_tasks() {
        let dispatcher = stub_dispatcher();
        for idx in 0..3 {
            dispatcher
                .dispatch(AgentDispatchRequest {
                    agent: AgentKind::Code,
                    objective: format!("task-{idx}"),
                    context: AgentContext::default(),
                    model: None,
                    metadata: None,
                    parameters: None,
                })
                .expect("dispatch");
        }
        sleep(Duration::from_millis(80)).await;
        let history = dispatcher.history(5);
        assert!(history.len() >= 3);
        assert!(history.iter().all(|entry| entry.status.is_terminal()));
    }
}
