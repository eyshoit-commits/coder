import "dotenv/config";
import http from "node:http";
import express, { Request, Response } from "express";
import cors from "cors";
import { Pool } from "pg";
import { v4 as uuidv4 } from "uuid";
import expressWs from "express-ws";
import { z } from "zod";

import { loadConfig } from "./config";
import { ModelManager } from "./modelManager";
import { TokenTracker } from "./tokenTracker";
import { requestCounter, tokenCounter, inferenceHistogram, activeSessions, metricsHandler } from "./metrics";
import { downloadModel, listAvailableDownloads } from "./downloader";
import { verifyAdminToken } from "./auth";

interface UserContext {
  userId?: number;
  requestId?: string;
}

const chatSchema = z.object({
  model: z.string(),
  messages: z
    .array(
      z.object({
        role: z.string().min(1),
        content: z.string().min(1)
      })
    )
    .min(1),
  temperature: z.number().min(0).max(2).optional(),
  top_k: z.number().int().min(1).max(200).optional(),
  top_p: z.number().min(0).max(1).optional(),
  repeat_penalty: z.number().min(0).max(2).optional(),
  max_tokens: z.number().int().min(1).max(4096).optional()
});

const completionSchema = z.object({
  model: z.string(),
  prompt: z.string().min(1),
  temperature: z.number().min(0).max(2).optional(),
  top_k: z.number().int().min(1).max(200).optional(),
  top_p: z.number().min(0).max(1).optional(),
  repeat_penalty: z.number().min(0).max(2).optional(),
  max_tokens: z.number().int().min(1).max(4096).optional()
});

const embeddingsSchema = z.object({
  model: z.string(),
  input: z.union([z.string(), z.array(z.string().min(1)).min(1)])
});

const adminLoadSchema = z.object({
  model: z.string(),
  temperature: z.number().min(0).max(2).optional(),
  top_k: z.number().int().min(1).max(200).optional(),
  top_p: z.number().min(0).max(1).optional(),
  repeat_penalty: z.number().min(0).max(2).optional(),
  max_tokens: z.number().int().min(1).max(4096).optional()
});

const config = loadConfig();
const pool = new Pool({ connectionString: config.databaseUrl });
const tokenTracker = new TokenTracker(pool);
const modelManager = new ModelManager(config);

const app = express();
const server = http.createServer(app);
expressWs(app, server);

app.use(cors());
app.use(express.json({ limit: "2mb" }));

app.get("/health", (_req, res) => {
  res.json({ status: "ok", uptime: process.uptime() });
});

app.get("/metrics", async (_req, res) => {
  const metrics = await metricsHandler();
  res.setHeader("Content-Type", "text/plain; version=0.0.4");
  res.send(metrics);
});

app.get("/admin/models", (_req, res) => {
  res.json({ models: modelManager.listModels(), downloads: listAvailableDownloads() });
});

app.get("/admin/status", (_req, res) => {
  res.json({
    uptime: process.uptime(),
    memory: process.memoryUsage(),
    models: modelManager.listModels()
  });
});

app.post("/admin/download", async (req, res) => {
  try {
    requireAdmin(req.headers.authorization);
    const { model } = z.object({ model: z.string() }).parse(req.body);
    const progress = await downloadModel(config, model);
    res.json({ status: "downloaded", progress });
  } catch (error) {
    respondError(res, error);
  }
});

app.post("/admin/load", async (req, res) => {
  try {
    requireAdmin(req.headers.authorization);
    const payload = adminLoadSchema.parse(req.body);
    const status = await modelManager.loadModel(payload.model, {
      temperature: payload.temperature,
      topK: payload.top_k,
      topP: payload.top_p,
      repeatPenalty: payload.repeat_penalty,
      maxTokens: payload.max_tokens
    });
    res.json({ status: "loaded", model: status });
  } catch (error) {
    respondError(res, error);
  }
});

app.post("/admin/unload", async (req, res) => {
  try {
    requireAdmin(req.headers.authorization);
    const { model } = z.object({ model: z.string() }).parse(req.body);
    await modelManager.unloadModel(model);
    res.json({ status: "unloaded", model });
  } catch (error) {
    respondError(res, error);
  }
});

app.post("/v1/chat/completions", async (req, res) => {
  const context = extractUserContext(req);
  const parsed = chatSchema.safeParse(req.body);
  if (!parsed.success) {
    return res.status(400).json({ error: parsed.error.flatten() });
  }
  const payload = parsed.data;
  requestCounter.inc({ endpoint: "chat", model: payload.model });
  const stopTimer = inferenceHistogram.startTimer({ endpoint: "chat", model: payload.model });
  activeSessions.inc();
  try {
    const result = await modelManager.chat(payload.model, payload.messages, {
      temperature: payload.temperature,
      topK: payload.top_k,
      topP: payload.top_p,
      repeatPenalty: payload.repeat_penalty,
      maxTokens: payload.max_tokens
    });
    await recordTokens(context, payload.model, "chat", result.promptTokens, result.completionTokens);
    tokenCounter.inc({ model: payload.model, type: "prompt" }, result.promptTokens);
    tokenCounter.inc({ model: payload.model, type: "completion" }, result.completionTokens);
    res.json(buildChatResponse(payload.model, result.text, result.promptTokens, result.completionTokens));
  } catch (error) {
    respondError(res, error);
  } finally {
    stopTimer();
    activeSessions.dec();
  }
});

app.post("/v1/completions", async (req, res) => {
  const context = extractUserContext(req);
  const parsed = completionSchema.safeParse(req.body);
  if (!parsed.success) {
    return res.status(400).json({ error: parsed.error.flatten() });
  }
  const payload = parsed.data;
  requestCounter.inc({ endpoint: "completion", model: payload.model });
  const stopTimer = inferenceHistogram.startTimer({ endpoint: "completion", model: payload.model });
  try {
    const result = await modelManager.complete(payload.model, payload.prompt, {
      temperature: payload.temperature,
      topK: payload.top_k,
      topP: payload.top_p,
      repeatPenalty: payload.repeat_penalty,
      maxTokens: payload.max_tokens
    });
    await recordTokens(context, payload.model, "completion", result.promptTokens, result.completionTokens);
    tokenCounter.inc({ model: payload.model, type: "prompt" }, result.promptTokens);
    tokenCounter.inc({ model: payload.model, type: "completion" }, result.completionTokens);
    res.json({
      id: uuidv4(),
      object: "text_completion",
      created: Math.floor(Date.now() / 1000),
      model: payload.model,
      choices: [
        {
          index: 0,
          text: result.text,
          finish_reason: "stop"
        }
      ],
      usage: {
        prompt_tokens: result.promptTokens,
        completion_tokens: result.completionTokens,
        total_tokens: result.promptTokens + result.completionTokens
      }
    });
  } catch (error) {
    respondError(res, error);
  } finally {
    stopTimer();
  }
});

app.post("/v1/embeddings", async (req, res) => {
  const context = extractUserContext(req);
  const parsed = embeddingsSchema.safeParse(req.body);
  if (!parsed.success) {
    return res.status(400).json({ error: parsed.error.flatten() });
  }
  const payload = parsed.data;
  requestCounter.inc({ endpoint: "embeddings", model: payload.model });
  const stopTimer = inferenceHistogram.startTimer({ endpoint: "embeddings", model: payload.model });
  try {
    const result = await modelManager.embed(payload.model, payload.input);
    await recordTokens(context, payload.model, "embeddings", result.tokens, 0);
    tokenCounter.inc({ model: payload.model, type: "prompt" }, result.tokens);
    res.json({
      object: "list",
      data: [
        {
          object: "embedding",
          embedding: result.embedding,
          index: 0
        }
      ],
      model: payload.model,
      usage: {
        prompt_tokens: result.tokens,
        completion_tokens: 0,
        total_tokens: result.tokens
      }
    });
  } catch (error) {
    respondError(res, error);
  } finally {
    stopTimer();
  }
});

app.ws("/v1/stream", async (ws, req) => {
  const context = extractUserContext(req as Request);
  ws.on("message", async (raw) => {
    try {
      const payload = chatSchema.parse(JSON.parse(raw.toString()));
      requestCounter.inc({ endpoint: "stream", model: payload.model });
      activeSessions.inc();
      const stopTimer = inferenceHistogram.startTimer({ endpoint: "stream", model: payload.model });
      try {
        const result = await modelManager.chat(payload.model, payload.messages, {
          temperature: payload.temperature,
          topK: payload.top_k,
          topP: payload.top_p,
          repeatPenalty: payload.repeat_penalty,
          maxTokens: payload.max_tokens
        });
        await recordTokens(context, payload.model, "stream", result.promptTokens, result.completionTokens);
        tokenCounter.inc({ model: payload.model, type: "prompt" }, result.promptTokens);
        tokenCounter.inc({ model: payload.model, type: "completion" }, result.completionTokens);
        ws.send(
          JSON.stringify({
            type: "chunk",
            data: result.text,
            usage: {
              prompt_tokens: result.promptTokens,
              completion_tokens: result.completionTokens,
              total_tokens: result.promptTokens + result.completionTokens
            }
          })
        );
        ws.send(JSON.stringify({ type: "done" }));
      } catch (error) {
        ws.send(JSON.stringify({ type: "error", message: messageFromError(error) }));
      } finally {
        stopTimer();
        activeSessions.dec();
      }
    } catch (error) {
      ws.send(JSON.stringify({ type: "error", message: messageFromError(error) }));
    }
  });
});

server.listen(config.port, config.host, () => {
  // eslint-disable-next-line no-console
  console.log(`LLM server listening on ${config.host}:${config.port}`);
});

async function recordTokens(
  context: UserContext,
  model: string,
  endpoint: string,
  promptTokens: number,
  completionTokens: number
): Promise<void> {
  if (!context.userId) {
    return;
  }
  try {
    await tokenTracker.recordUsage({
      userId: context.userId,
      model,
      endpoint,
      promptTokens,
      completionTokens,
      requestId: context.requestId
    });
  } catch (error) {
    if (messageFromError(error).includes("Insufficient token")) {
      throw Object.assign(new Error("insufficient tokens"), { status: 429 });
    }
    throw error;
  }
}

function extractUserContext(req: Request): UserContext {
  const userHeader = req.headers["x-user-id"];
  const requestId = req.headers["x-request-id"];
  let userId: number | undefined;
  if (typeof userHeader === "string") {
    userId = parseInt(userHeader, 10);
  }
  return {
    userId: Number.isFinite(userId) ? userId : undefined,
    requestId: typeof requestId === "string" ? requestId : undefined
  };
}

function requireAdmin(authorization?: string): void {
  if (!authorization) {
    throw Object.assign(new Error("missing authorization header"), { status: 401 });
  }
  const token = authorization.replace(/^Bearer\s+/i, "");
  verifyAdminToken(config, token);
}

function buildChatResponse(model: string, text: string, promptTokens: number, completionTokens: number) {
  return {
    id: uuidv4(),
    object: "chat.completion",
    created: Math.floor(Date.now() / 1000),
    model,
    choices: [
      {
        index: 0,
        finish_reason: "stop",
        message: {
          role: "assistant",
          content: text
        }
      }
    ],
    usage: {
      prompt_tokens: promptTokens,
      completion_tokens: completionTokens,
      total_tokens: promptTokens + completionTokens
    }
  };
}

function respondError(res: Response, error: unknown): void {
  const status = (error as { status?: number }).status ?? 500;
  res.status(status).json({ error: messageFromError(error) });
}

function messageFromError(error: unknown): string {
  if (error instanceof Error) {
    return error.message;
  }
  if (typeof error === "string") {
    return error;
  }
  return "unknown error";
}
