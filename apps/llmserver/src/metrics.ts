import client from "prom-client";

const register = new client.Registry();

export const requestCounter = new client.Counter({
  name: "llm_requests_total",
  help: "Total number of LLM requests processed",
  labelNames: ["endpoint", "model"]
});

export const tokenCounter = new client.Counter({
  name: "llm_tokens_generated",
  help: "Total tokens generated across prompts and completions",
  labelNames: ["model", "type"]
});

export const inferenceHistogram = new client.Histogram({
  name: "llm_inference_duration_seconds",
  help: "Duration of inference operations",
  labelNames: ["endpoint", "model"],
  buckets: [0.1, 0.25, 0.5, 1, 2, 3, 5, 8, 13]
});

export const activeSessions = new client.Gauge({
  name: "llm_active_sessions",
  help: "Active chat sessions"
});

register.registerMetric(requestCounter);
register.registerMetric(tokenCounter);
register.registerMetric(inferenceHistogram);
register.registerMetric(activeSessions);

client.collectDefaultMetrics({ register });

export function metricsHandler(): Promise<string> {
  return register.metrics();
}
