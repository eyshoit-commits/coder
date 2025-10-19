export interface ModelMetadata {
  readonly name: string;
  readonly displayName: string;
  readonly huggingFaceRepo: string;
  readonly file: string;
  readonly context: number;
  readonly costPerToken: number;
}

export const MODEL_CATALOG: readonly ModelMetadata[] = [
  {
    name: "deepseek-coder-1.3b",
    displayName: "DeepSeek Coder 1.3B Instruct",
    huggingFaceRepo: "deepseek-ai/deepseek-coder-1.3b-instruct-GGUF",
    file: "deepseek-coder-1.3b-instruct.Q4_K_M.gguf",
    context: 4096,
    costPerToken: 0.00045
  },
  {
    name: "nous-hermes-2-3b.Q4",
    displayName: "Nous Hermes 2 3B Q4",
    huggingFaceRepo: "TheBloke/Nous-Hermes-2-3B-GGUF",
    file: "nous-hermes-llama2-3b.Q4_K_M.gguf",
    context: 4096,
    costPerToken: 0.00055
  },
  {
    name: "bge-small-en-v1.5",
    displayName: "BGE Small English v1.5",
    huggingFaceRepo: "BAAI/bge-small-en-v1.5",
    file: "bge-small-en-v1.5-q4_0.gguf",
    context: 2048,
    costPerToken: 0.00035
  },
  {
    name: "tinyllama-1.1b-func",
    displayName: "TinyLlama 1.1B Function",
    huggingFaceRepo: "cognitivecomputations/TinyLlama-1.1B-Function-Call-GGUF",
    file: "tinyllama-1.1b-chat-v1.0.Q4_K_M.gguf",
    context: 2048,
    costPerToken: 0.00030
  }
];

export function getModelMetadata(name: string): ModelMetadata | undefined {
  return MODEL_CATALOG.find((model) => model.name === name);
}
