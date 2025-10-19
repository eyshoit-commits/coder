import path from "node:path";
import fs from "node:fs";

export interface ServerConfig {
  readonly port: number;
  readonly host: string;
  readonly modelsDir: string;
  readonly downloadsDir: string;
  readonly llmThreadCount: number;
  readonly llmBatchSize: number;
  readonly databaseUrl: string;
  readonly maxStreamingSeconds: number;
  readonly adminJwtSecret: string;
}

function ensureDirectory(dir: string): void {
  if (!fs.existsSync(dir)) {
    fs.mkdirSync(dir, { recursive: true });
  }
}

export function loadConfig(): ServerConfig {
  const port = parseInt(process.env.LLM_PORT ?? "6988", 10);
  const host = process.env.LLM_HOST ?? "0.0.0.0";
  const modelsDir = process.env.LLM_MODELS_DIR ?? path.resolve(process.cwd(), "models");
  const downloadsDir = process.env.LLM_DOWNLOAD_DIR ?? path.join(modelsDir, "downloads");
  const llmThreadCount = parseInt(process.env.LLM_THREADS ?? "8", 10);
  const llmBatchSize = parseInt(process.env.LLM_BATCH_SIZE ?? "1024", 10);
  const maxStreamingSeconds = parseInt(process.env.LLM_MAX_STREAM_SECONDS ?? "30", 10);
  const databaseUrl = process.env.DATABASE_URL;
  const adminJwtSecret = process.env.LLM_ADMIN_JWT_SECRET ?? "";

  if (!databaseUrl) {
    throw new Error("DATABASE_URL environment variable is required for LLM server");
  }
  if (!Number.isFinite(port) || port <= 0) {
    throw new Error("LLM_PORT must be a positive integer");
  }
  if (llmThreadCount <= 0) {
    throw new Error("LLM_THREADS must be positive");
  }
  if (llmBatchSize <= 0) {
    throw new Error("LLM_BATCH_SIZE must be positive");
  }
  if (!adminJwtSecret) {
    throw new Error("LLM_ADMIN_JWT_SECRET must be configured");
  }

  ensureDirectory(modelsDir);
  ensureDirectory(downloadsDir);

  return {
    port,
    host,
    modelsDir,
    downloadsDir,
    llmThreadCount,
    llmBatchSize,
    databaseUrl,
    maxStreamingSeconds,
    adminJwtSecret
  };
}
