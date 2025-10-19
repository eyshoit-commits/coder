import { FormEvent, useEffect, useMemo, useState } from 'react';
import { useStudioContext } from '../hooks/useStudioContext';

interface LlmModel {
  id: string;
  name: string;
  status: 'available' | 'loaded';
  context_size: number;
}

interface ChatMessage {
  role: 'system' | 'user' | 'assistant';
  content: string;
}

interface ChatResponse {
  content: string;
  tokens: {
    prompt: number;
    completion: number;
  };
}

export function LLMPlayground() {
  const { rpc, refreshTokenUsage } = useStudioContext();
  const [models, setModels] = useState<LlmModel[]>([]);
  const [selectedModel, setSelectedModel] = useState<string>('');
  const [systemPrompt, setSystemPrompt] = useState('You are CyberDevStudio, a diligent AI pair programmer.');
  const [userPrompt, setUserPrompt] = useState('Write a Rust function that returns the Fibonacci sequence up to n.');
  const [temperature, setTemperature] = useState(0.2);
  const [topP, setTopP] = useState(0.95);
  const [topK, setTopK] = useState(40);
  const [maxTokens, setMaxTokens] = useState(512);
  const [response, setResponse] = useState<ChatResponse | null>(null);
  const [isLoading, setIsLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    const loadModels = async () => {
      try {
        const result = await rpc.call<{ models: LlmModel[] }>('llm.list_models');
        setModels(result.models);
        setSelectedModel((previous) => previous || result.models.find((model) => model.status === 'loaded')?.id || result.models[0]?.id || '');
      } catch (err) {
        console.warn('Unable to load LLM models', err);
      }
    };
    loadModels().catch((err) => console.error(err));
  }, [rpc]);

  const handleSubmit = async (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    setIsLoading(true);
    setError(null);
    setResponse(null);
    try {
      const payload: ChatMessage[] = [];
      if (systemPrompt.trim()) {
        payload.push({ role: 'system', content: systemPrompt });
      }
      payload.push({ role: 'user', content: userPrompt });
      const completion = await rpc.call<ChatResponse>('llm.chat', {
        model: selectedModel,
        messages: payload,
        temperature,
        top_p: topP,
        top_k: topK,
        max_tokens: maxTokens
      });
      setResponse(completion);
      await refreshTokenUsage();
    } catch (err) {
      if (err instanceof Error) {
        setError(err.message);
      } else {
        setError('LLM invocation failed');
      }
    } finally {
      setIsLoading(false);
    }
  };

  const activeModel = useMemo(() => models.find((model) => model.id === selectedModel), [models, selectedModel]);

  return (
    <div className="flex h-full flex-col bg-[color:var(--bg-primary)]/80">
      <form onSubmit={handleSubmit} className="space-y-4 border-b border-slate-800/60 bg-[color:var(--panel)] px-6 py-4 text-sm">
        <div className="grid gap-4 md:grid-cols-3">
          <label className="flex flex-col text-xs uppercase tracking-wide text-[color:var(--text-secondary)]">
            Model
            <select
              value={selectedModel}
              onChange={(event) => setSelectedModel(event.target.value)}
              className="mt-1 rounded-md border border-[color:var(--accent-1)]/40 bg-transparent px-3 py-2 text-sm text-[color:var(--text-primary)] focus:outline-none"
            >
              {models.map((model) => (
                <option key={model.id} value={model.id} className="text-slate-900">
                  {model.name} · {model.status}
                </option>
              ))}
            </select>
          </label>
          <label className="flex flex-col text-xs uppercase tracking-wide text-[color:var(--text-secondary)]">
            Temperature ({temperature.toFixed(2)})
            <input
              type="range"
              min="0"
              max="1"
              step="0.01"
              value={temperature}
              onChange={(event) => setTemperature(Number(event.target.value))}
            />
          </label>
          <label className="flex flex-col text-xs uppercase tracking-wide text-[color:var(--text-secondary)]">
            Top P ({topP.toFixed(2)})
            <input
              type="range"
              min="0"
              max="1"
              step="0.01"
              value={topP}
              onChange={(event) => setTopP(Number(event.target.value))}
            />
          </label>
          <label className="flex flex-col text-xs uppercase tracking-wide text-[color:var(--text-secondary)]">
            Top K ({topK})
            <input
              type="range"
              min="1"
              max="200"
              step="1"
              value={topK}
              onChange={(event) => setTopK(Number(event.target.value))}
            />
          </label>
          <label className="flex flex-col text-xs uppercase tracking-wide text-[color:var(--text-secondary)]">
            Max Tokens ({maxTokens})
            <input
              type="range"
              min="64"
              max="2048"
              step="64"
              value={maxTokens}
              onChange={(event) => setMaxTokens(Number(event.target.value))}
            />
          </label>
        </div>
        {activeModel && (
          <div className="rounded-lg border border-[color:var(--accent-1)]/30 bg-[color:var(--bg-secondary)]/40 p-3 text-xs text-[color:var(--text-secondary)]">
            Context window: {activeModel.context_size} tokens · Status: {activeModel.status}
          </div>
        )}
        <div className="grid gap-4 md:grid-cols-2">
          <label className="flex flex-col text-xs uppercase tracking-wide text-[color:var(--text-secondary)]">
            System Prompt
            <textarea
              value={systemPrompt}
              onChange={(event) => setSystemPrompt(event.target.value)}
              className="mt-1 h-24 rounded-md border border-transparent bg-[color:var(--bg-secondary)]/70 px-3 py-2 text-sm text-[color:var(--text-primary)] focus:border-[color:var(--accent-1)]/60 focus:outline-none"
            />
          </label>
          <label className="flex flex-col text-xs uppercase tracking-wide text-[color:var(--text-secondary)]">
            User Prompt
            <textarea
              value={userPrompt}
              onChange={(event) => setUserPrompt(event.target.value)}
              className="mt-1 h-24 rounded-md border border-transparent bg-[color:var(--bg-secondary)]/70 px-3 py-2 text-sm text-[color:var(--text-primary)] focus:border-[color:var(--accent-1)]/60 focus:outline-none"
            />
          </label>
        </div>
        <div className="flex items-center justify-between">
          <span className="text-xs text-[color:var(--text-secondary)]">
            Adjust inference parameters to explore model behavior. Token usage updates after each completion.
          </span>
          <button
            type="submit"
            disabled={isLoading || !selectedModel}
            className="btn-primary rounded-md px-4 py-2 text-xs font-semibold uppercase tracking-wide"
          >
            {isLoading ? 'Generating…' : 'Generate'}
          </button>
        </div>
      </form>
      <div className="flex-1 overflow-y-auto bg-[color:var(--bg-primary)]/70 p-6">
        {error && <p className="mb-3 text-sm text-red-400">{error}</p>}
        {response ? (
          <div className="space-y-4">
            <article className="panel rounded-lg p-4 text-sm text-[color:var(--text-primary)] whitespace-pre-wrap">
              {response.content}
            </article>
            <div className="grid grid-cols-2 gap-3 text-xs text-[color:var(--text-secondary)]">
              <div>
                <p className="font-semibold uppercase tracking-wide text-[color:var(--text-primary)]">Prompt Tokens</p>
                <p>{response.tokens.prompt}</p>
              </div>
              <div>
                <p className="font-semibold uppercase tracking-wide text-[color:var(--text-primary)]">Completion Tokens</p>
                <p>{response.tokens.completion}</p>
              </div>
            </div>
          </div>
        ) : (
          <p className="text-sm text-[color:var(--text-secondary)]">
            Submit a prompt to generate a completion using the selected model.
          </p>
        )}
      </div>
    </div>
  );
}
