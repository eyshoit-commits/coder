# CyberDevStudio

CyberDevStudio is an ambitious, modular developer platform that combines a Rust backend, a TypeScript front-end, and an embedded node-llama-cpp inference service. This repository currently provides the project scaffold, documentation, and configuration needed to begin implementing the full system described in the high-level specification.

The workspace is organized to support multiple services (API, Auth, LLM server, Studio UI) as well as shared components such as sandboxed execution, metrics, schemas, and database migrations. Each module is intended to be developed independently while sharing a coherent deployment story through Docker Compose.

For architectural guidance, planned milestones, and module-specific expectations, see [`docs/Projektplan.md`](docs/Projektplan.md).
