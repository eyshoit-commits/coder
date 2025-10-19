# CyberDevStudio Acceptance Checklist

This document enumerates the end-to-end criteria used to certify a release of CyberDevStudio. Every build must satisfy the checks below before promotion to production.

## Platform Readiness

1. **Workspace Initialization**
   - `docker-compose` stack boots cleanly with `api`, `auth`, `llmserver`, `studio-ui`, `db`, `otel-collector`, `prometheus`, and `grafana` healthy.
   - Database migrations apply automatically on first boot.

2. **Authentication Lifecycle**
   - `POST /auth/register` creates a developer account.
   - `POST /auth/login` issues a JWT valid for 24 hours.
   - Admin can create and revoke API keys via `/auth/api-keys` endpoints.

3. **Project Operations**
   - JSON-RPC `project.create` provisions a project and returns identifiers.
   - Files can be written, listed, read, moved, and deleted within the sandbox root.

4. **Execution Engines**
   - `run.exec` executes `/bin/sh` commands with enforced timeouts and output caps.
   - `wasm.invoke` executes a WebAssembly module with enforced fuel and memory guards.
   - `micro.start` + `micro.execute` runs Python code inside an isolated workspace.

5. **Agent Coordination**
   - `agent.list` exposes all registered agents.
   - `agent.dispatch` triggers execution, and `agent.status` transitions `pending → running → completed`.
   - Cancellation via `agent.cancel` terminates active workloads.

6. **LLM Operations**
   - Admin can `llm.download` a GGUF model, `llm.start` it, and receive completions via `llm.chat` within three seconds for short prompts.
   - Token usage is persisted to `tokens_used` with per-user accounting.

7. **UI Verification**
   - Studio loads in both NeonCyberNight and SerialSteel themes.
   - Editor, terminal, agent chat, execution view, LLM playground, and admin dashboards render and interact with backend services.

8. **Observability**
   - `/metrics` on API and LLM services exports Prometheus counters and histograms.
   - OpenTelemetry collector forwards traces from API requests.
   - Grafana dashboards display token usage, sandbox activity, and request latency.

## Regression Suite

| Scenario | Command | Expectation |
| --- | --- | --- |
| Filesystem Write | `cargo test --test fs_write` | All assertions pass. |
| Runtime Execution | `cargo test --test run_exec` | Whitelisted commands succeed; disallowed commands fail. |
| Sandbox E2E | `cargo test --test e2e` | Combined filesystem, run, wasm, and micro flows succeed. |
| API Health | `curl http://localhost:6813/health` | Returns `{"status":"ok"}`. |
| Auth Health | `curl http://localhost:6971/health` | Returns `{"status":"ok"}`. |
| LLM Metrics | `curl http://localhost:6988/metrics` | Contains `llm_requests_total`. |

## Release Gate

A release is approved when:

- All regression scenarios succeed in CI.
- Token balance enforcement blocks negative balances in staging.
- Admin panel displays real-time Prometheus data for the last 15 minutes.
- Documentation (API reference, deployment, operator playbook) is updated for the current tag.

