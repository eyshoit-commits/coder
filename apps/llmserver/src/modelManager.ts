import fs from "node:fs";
import path from "node:path";
import EventEmitter from "node:events";
import type { LlamaModelOptions } from "node-llama-cpp";
import { MODEL_CATALOG, ModelMetadata, getModelMetadata } from "./catalog";
import { ServerConfig } from "./config";

// node-llama-cpp does not ship TypeScript definitions for runtime types.
// eslint-disable-next-line @typescript-eslint/no-var-requires
const llama: any = require("node-llama-cpp");

type ChatSession = any;

type LoadOverrides = {
  temperature?: number;
  topK?: number;
  topP?: number;
  repeatPenalty?: number;
  maxTokens?: number;
};

export interface CompletionResult {
  text: string;
  promptTokens: number;
  completionTokens: number;
}

export interface EmbeddingResult {
  embedding: number[];
  tokens: number;
}

export interface ManagedModel {
  metadata: ModelMetadata;
  modelPath: string;
  threads: number;
  batchSize: number;
  options: Required<LoadOverrides>;
  chat: ChatSession;
  context: any;
  model: any;
  loadedAt: Date;
}

export interface ModelStatus {
  name: string;
  displayName: string;
  loaded: boolean;
  contextSize: number;
  loadedAt?: string;
  threads?: number;
  batchSize?: number;
}

export class ModelManager extends EventEmitter {
  private readonly config: ServerConfig;
  private readonly models = new Map<string, ManagedModel>();

  constructor(config: ServerConfig) {
    super();
    this.config = config;
  }

  listModels(): ModelStatus[] {
    return MODEL_CATALOG.map((metadata) => {
      const loaded = this.models.get(metadata.name);
      return {
        name: metadata.name,
        displayName: metadata.displayName,
        loaded: Boolean(loaded),
        contextSize: metadata.context,
        loadedAt: loaded?.loadedAt.toISOString(),
        threads: loaded?.threads,
        batchSize: loaded?.batchSize
      };
    });
  }

  async loadModel(name: string, overrides?: LoadOverrides): Promise<ModelStatus> {
    const metadata = requireMetadata(name);
    const modelPath = this.resolveModelPath(metadata);
    if (!fs.existsSync(modelPath)) {
      throw new Error(`Model file not found at ${modelPath}. Download model before loading.`);
    }

    const threads = this.config.llmThreadCount;
    const batchSize = this.config.llmBatchSize;
    const options: Required<LoadOverrides> = {
      temperature: overrides?.temperature ?? 0.2,
      topK: overrides?.topK ?? 40,
      topP: overrides?.topP ?? 0.9,
      repeatPenalty: overrides?.repeatPenalty ?? 1.1,
      maxTokens: overrides?.maxTokens ?? 1024
    };

    const model = new llama.LlamaModel({
      modelPath,
      contextSize: metadata.context,
      gpuLayers: 0,
      seed: Date.now()
    } as LlamaModelOptions);
    const context = new llama.LlamaContext({ model, threads, batchSize });
    const chat = new llama.LlamaChatSession({ context });

    const managed: ManagedModel = {
      metadata,
      modelPath,
      threads,
      batchSize,
      options,
      chat,
      context,
      model,
      loadedAt: new Date()
    };
    this.models.set(metadata.name, managed);
    this.emit("loaded", metadata.name);
    return this.describe(metadata.name);
  }

  describe(name: string): ModelStatus {
    const metadata = requireMetadata(name);
    const loaded = this.models.get(name);
    return {
      name: metadata.name,
      displayName: metadata.displayName,
      loaded: Boolean(loaded),
      contextSize: metadata.context,
      loadedAt: loaded?.loadedAt.toISOString(),
      threads: loaded?.threads,
      batchSize: loaded?.batchSize
    };
  }

  async unloadModel(name: string): Promise<void> {
    const loaded = this.models.get(name);
    if (!loaded) {
      return;
    }
    safeDispose(loaded.chat);
    safeDispose(loaded.context);
    safeDispose(loaded.model);
    this.models.delete(name);
    this.emit("unloaded", name);
  }

  ensureLoaded(name: string): ManagedModel {
    const loaded = this.models.get(name);
    if (!loaded) {
      throw new Error(`Model '${name}' is not loaded`);
    }
    return loaded;
  }

  async complete(name: string, prompt: string, overrides?: LoadOverrides): Promise<CompletionResult> {
    const managed = this.ensureLoaded(name);
    const options = { ...managed.options, ...overrides };
    const promptTokens = countTokens(managed.context, prompt);
    const response: any = await managed.chat.prompt(prompt, {
      maxTokens: options.maxTokens,
      temperature: options.temperature,
      topK: options.topK,
      topP: options.topP,
      repeatPenalty: options.repeatPenalty
    });
    const text = typeof response === "string" ? response : String(response?.output ?? response);
    const completionTokens = countTokens(managed.context, text);
    return { text, promptTokens, completionTokens };
  }

  async chat(
    name: string,
    messages: { role: string; content: string }[],
    overrides?: LoadOverrides
  ): Promise<CompletionResult> {
    const prompt = messages
      .map((message) => `${message.role.toUpperCase()}: ${message.content}`)
      .join("\n\n");
    return this.complete(name, prompt, overrides);
  }

  async embed(name: string, input: string | string[]): Promise<EmbeddingResult> {
    const managed = this.ensureLoaded(name);
    if (typeof managed.context.createEmbedding !== "function") {
      throw new Error("Model does not support embeddings on this backend");
    }
    const items = Array.isArray(input) ? input : [input];
    const tokens = items.reduce((acc, item) => acc + countTokens(managed.context, item), 0);
    const embedding = await managed.context.createEmbedding(items);
    return { embedding: Array.isArray(embedding) ? embedding[0] : embedding, tokens };
  }

  resolveModelPath(metadata: ModelMetadata): string {
    return path.join(this.config.modelsDir, metadata.name, metadata.file);
  }
}

function safeDispose(target: any): void {
  if (target && typeof target.dispose === "function") {
    try {
      target.dispose();
    } catch (error) {
      console.warn("failed to dispose llama resource", error);
    }
  }
}

function countTokens(context: any, text: string): number {
  if (context && typeof context.tokenize === "function") {
    const tokens = context.tokenize(text);
    if (Array.isArray(tokens)) {
      return tokens.length;
    }
  }
  return Math.max(1, Math.ceil(text.length / 4));
}

function requireMetadata(name: string): ModelMetadata {
  const metadata = getModelMetadata(name);
  if (!metadata) {
    throw new Error(`Model '${name}' is not supported`);
  }
  return metadata;
}
