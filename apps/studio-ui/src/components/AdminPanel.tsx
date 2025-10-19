import { useEffect, useMemo, useState } from 'react';
import { useStudioContext } from '../hooks/useStudioContext';

interface ModelInfo {
  id: string;
  name: string;
  status: 'available' | 'loaded';
  huggingface_url?: string;
  context_size: number;
  memory_mb?: number;
}

interface UserInfo {
  id: number;
  username: string;
  role: string;
  token_balance: number;
}

interface TokenUsage {
  timestamp: string;
  tokens: number;
  model: string;
}

export function AdminPanel() {
  const { rpc, profile } = useStudioContext();
  const [models, setModels] = useState<ModelInfo[]>([]);
  const [users, setUsers] = useState<UserInfo[]>([]);
  const [usage, setUsage] = useState<TokenUsage[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const canAccess = profile.role === 'admin';

  const loadData = async () => {
    setLoading(true);
    setError(null);
    try {
      const [modelsResponse, usersResponse, usageResponse] = await Promise.all([
        rpc.call<{ models: ModelInfo[] }>('llm.list_models'),
        rpc.call<{ users: UserInfo[] }>('admin.users.list'),
        rpc.call<{ usage: TokenUsage[] }>('admin.tokens.recent')
      ]);
      setModels(modelsResponse.models);
      setUsers(usersResponse.users);
      setUsage(usageResponse.usage);
    } catch (err) {
      if (err instanceof Error) {
        setError(err.message);
      } else {
        setError('Unable to load admin data');
      }
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => {
    if (canAccess) {
      loadData().catch((err) => console.error(err));
    }
  }, [canAccess]);

  const handleModelAction = async (model: ModelInfo, action: 'load' | 'unload' | 'download') => {
    setLoading(true);
    setError(null);
    try {
      if (action === 'download') {
        await rpc.call('llm.download', { id: model.id });
      } else if (action === 'load') {
        await rpc.call('llm.start', { id: model.id, options: { temperature: 0.2, top_k: 40, top_p: 0.95 } });
      } else if (action === 'unload') {
        await rpc.call('llm.stop', { id: model.id });
      }
      await loadData();
    } catch (err) {
      if (err instanceof Error) {
        setError(err.message);
      } else {
        setError('Unable to update model state');
      }
    } finally {
      setLoading(false);
    }
  };

  const totalTokens = useMemo(() => usage.reduce((sum, entry) => sum + entry.tokens, 0), [usage]);

  if (!canAccess) {
    return (
      <div className="flex h-full items-center justify-center bg-[color:var(--bg-primary)]/80 text-sm text-[color:var(--text-secondary)]">
        Admin access required.
      </div>
    );
  }

  return (
    <div className="flex h-full flex-col bg-[color:var(--bg-primary)]/80">
      <div className="border-b border-slate-800/60 bg-[color:var(--panel)] px-6 py-4">
        <div className="flex items-center justify-between">
          <h1 className="text-lg font-semibold text-[color:var(--text-primary)]">Admin Control Center</h1>
          <button
            onClick={() => loadData()}
            className="btn-secondary rounded-md px-4 py-2 text-xs font-semibold uppercase tracking-wide"
          >
            Refresh
          </button>
        </div>
        {loading && <p className="mt-2 text-xs text-[color:var(--text-secondary)]">Refreshing data…</p>}
        {error && <p className="mt-2 text-xs text-red-400">{error}</p>}
      </div>
      <div className="grid flex-1 gap-4 overflow-y-auto bg-[color:var(--bg-primary)]/70 p-6 xl:grid-cols-2">
        <section className="panel rounded-lg p-4">
          <header className="mb-4 flex items-center justify-between">
            <h2 className="text-sm font-semibold uppercase tracking-wide text-[color:var(--text-secondary)]">Models</h2>
            <span className="text-xs text-[color:var(--accent-1)]">{models.length} available</span>
          </header>
          <div className="space-y-3">
            {models.map((model) => (
              <article key={model.id} className="rounded-lg border border-slate-700/40 bg-[color:var(--bg-secondary)]/50 p-4">
                <header className="flex items-center justify-between">
                  <div>
                    <h3 className="text-sm font-semibold text-[color:var(--text-primary)]">{model.name}</h3>
                    <p className="text-xs text-[color:var(--text-secondary)]">Context {model.context_size} · Status {model.status}</p>
                  </div>
                  <div className="flex space-x-2 text-xs">
                    <button
                      onClick={() => handleModelAction(model, 'download')}
                      className="rounded-md border border-[color:var(--accent-1)]/40 px-3 py-1 text-[color:var(--accent-1)] hover:bg-[color:var(--accent-1)]/10"
                    >
                      Download
                    </button>
                    <button
                      onClick={() => handleModelAction(model, 'load')}
                      className="rounded-md border border-[color:var(--accent-1)]/40 px-3 py-1 text-[color:var(--accent-1)] hover:bg-[color:var(--accent-1)]/10"
                    >
                      Load
                    </button>
                    <button
                      onClick={() => handleModelAction(model, 'unload')}
                      className="rounded-md border border-red-400/60 px-3 py-1 text-red-300 hover:bg-red-500/20"
                    >
                      Unload
                    </button>
                  </div>
                </header>
                {model.huggingface_url && (
                  <p className="mt-2 text-xs text-[color:var(--text-secondary)]">Source: {model.huggingface_url}</p>
                )}
              </article>
            ))}
          </div>
        </section>
        <section className="panel rounded-lg p-4">
          <header className="mb-4 flex items-center justify-between">
            <h2 className="text-sm font-semibold uppercase tracking-wide text-[color:var(--text-secondary)]">Users</h2>
            <span className="text-xs text-[color:var(--accent-1)]">{users.length} accounts</span>
          </header>
          <div className="space-y-3">
            {users.map((user) => (
              <article key={user.id} className="rounded-lg border border-slate-700/40 bg-[color:var(--bg-secondary)]/50 p-4">
                <header className="flex items-center justify-between">
                  <div>
                    <h3 className="text-sm font-semibold text-[color:var(--text-primary)]">{user.username}</h3>
                    <p className="text-xs text-[color:var(--text-secondary)]">Role {user.role}</p>
                  </div>
                  <span className="text-xs text-[color:var(--accent-1)]">Tokens {user.token_balance}</span>
                </header>
              </article>
            ))}
          </div>
        </section>
        <section className="panel rounded-lg p-4 xl:col-span-2">
          <header className="mb-4 flex items-center justify-between">
            <h2 className="text-sm font-semibold uppercase tracking-wide text-[color:var(--text-secondary)]">Recent Token Usage</h2>
            <span className="text-xs text-[color:var(--accent-1)]">Total tokens {totalTokens}</span>
          </header>
          <div className="space-y-2 text-xs text-[color:var(--text-secondary)]">
            {usage.map((entry) => (
              <div key={`${entry.timestamp}-${entry.model}`} className="flex items-center justify-between rounded-md border border-slate-700/40 bg-[color:var(--bg-secondary)]/50 px-3 py-2">
                <span className="text-[color:var(--text-primary)]">{entry.model}</span>
                <span>{new Date(entry.timestamp).toLocaleString()}</span>
                <span className="text-[color:var(--accent-1)]">{entry.tokens} tokens</span>
              </div>
            ))}
            {usage.length === 0 && <p>No usage records available.</p>}
          </div>
        </section>
      </div>
    </div>
  );
}
