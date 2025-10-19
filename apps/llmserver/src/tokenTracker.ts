import { Pool, PoolClient } from "pg";
import { getModelMetadata, ModelMetadata } from "./catalog";

export interface TokenUsage {
  userId: number;
  model: string;
  endpoint: string;
  promptTokens: number;
  completionTokens: number;
  requestId?: string;
}

export class TokenTracker {
  private readonly pool: Pool;

  constructor(pool: Pool) {
    this.pool = pool;
  }

  async recordUsage(usage: TokenUsage): Promise<void> {
    const totalTokens = usage.promptTokens + usage.completionTokens;
    if (totalTokens <= 0) {
      return;
    }
    const metadata = getModelMetadata(usage.model);
    if (!metadata) {
      throw new Error(`Unsupported model '${usage.model}'`);
    }
    const client = await this.pool.connect();
    try {
      await client.query("BEGIN");
      const modelId = await this.ensureModel(client, metadata);
      const balanceRow = await client.query(
        "SELECT token_balance FROM users WHERE id = $1 FOR UPDATE",
        [usage.userId]
      );
      if (balanceRow.rowCount === 0) {
        throw new Error("User not found for token accounting");
      }
      const balance = Number(balanceRow.rows[0].token_balance);
      if (balance < totalTokens) {
        throw new Error("Insufficient token balance");
      }
      await client.query(
        "INSERT INTO tokens_used (user_id, model_id, tokens, endpoint) VALUES ($1, $2, $3, $4)",
        [usage.userId, modelId, totalTokens, usage.endpoint]
      );
      await client.query(
        "UPDATE users SET token_balance = token_balance - $1 WHERE id = $2",
        [totalTokens, usage.userId]
      );
      await client.query("COMMIT");
    } catch (err) {
      await client.query("ROLLBACK");
      throw err;
    } finally {
      client.release();
    }
  }

  private async ensureModel(client: PoolClient, metadata: ModelMetadata): Promise<number> {
    const existing = await client.query("SELECT id FROM models WHERE name = $1", [metadata.name]);
    if (existing.rowCount > 0) {
      return Number(existing.rows[0].id);
    }
    const inserted = await client.query(
      "INSERT INTO models (name, huggingface_url, context_size, cost_per_token, is_loaded) VALUES ($1, $2, $3, $4, false) RETURNING id",
      [metadata.name, `https://huggingface.co/${metadata.huggingFaceRepo}`, metadata.context, metadata.costPerToken]
    );
    return Number(inserted.rows[0].id);
  }
}
