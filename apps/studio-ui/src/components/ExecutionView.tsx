import { FormEvent, useState } from 'react';
import { useStudioContext } from '../hooks/useStudioContext';

interface RunResult {
  stdout: string;
  stderr: string;
  exit_code: number;
  duration_ms: number;
}

const engines = [
  { id: 'run', label: 'Runtime' },
  { id: 'wasm', label: 'WebAssembly' },
  { id: 'micro', label: 'Micro VM' }
];

export function ExecutionView() {
  const { rpc } = useStudioContext();
  const [command, setCommand] = useState('python3 main.py');
  const [args, setArgs] = useState('');
  const [engine, setEngine] = useState(engines[0].id);
  const [result, setResult] = useState<RunResult | null>(null);
  const [isRunning, setIsRunning] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const handleSubmit = async (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    setIsRunning(true);
    setError(null);
    setResult(null);
    try {
      if (engine === 'run') {
        const response = await rpc.call<RunResult>('run.exec', {
          command,
          args: args ? args.split(' ') : [],
          env: {}
        });
        setResult(response);
      } else if (engine === 'wasm') {
        const response = await rpc.call<RunResult>('wasm.invoke', {
          module: command,
          function: 'main',
          args: args ? args.split(' ') : []
        });
        setResult(response);
      } else if (engine === 'micro') {
        const response = await rpc.call<RunResult>('micro.execute', {
          image: 'python',
          code: command,
          args: args ? args.split(' ') : []
        });
        setResult(response);
      }
    } catch (err) {
      if (err instanceof Error) {
        setError(err.message);
      } else {
        setError('Execution failed');
      }
    } finally {
      setIsRunning(false);
    }
  };

  return (
    <div className="flex h-full flex-col bg-[color:var(--bg-primary)]/80">
      <form onSubmit={handleSubmit} className="border-b border-slate-800/60 bg-[color:var(--panel)] px-4 py-3">
        <div className="grid gap-3 md:grid-cols-4">
          <label className="flex flex-col text-xs uppercase tracking-wide text-[color:var(--text-secondary)]">
            Engine
            <select
              value={engine}
              onChange={(event) => setEngine(event.target.value)}
              className="mt-1 rounded-md border border-[color:var(--accent-1)]/40 bg-transparent px-3 py-2 text-sm text-[color:var(--text-primary)] focus:outline-none"
            >
              {engines.map((item) => (
                <option key={item.id} value={item.id} className="text-slate-900">
                  {item.label}
                </option>
              ))}
            </select>
          </label>
          <label className="md:col-span-2 flex flex-col text-xs uppercase tracking-wide text-[color:var(--text-secondary)]">
            Command / Module
            <input
              value={command}
              onChange={(event) => setCommand(event.target.value)}
              className="mt-1 rounded-md border border-transparent bg-[color:var(--bg-secondary)]/70 px-3 py-2 text-sm text-[color:var(--text-primary)] focus:border-[color:var(--accent-1)]/60 focus:outline-none"
            />
          </label>
          <label className="flex flex-col text-xs uppercase tracking-wide text-[color:var(--text-secondary)]">
            Arguments
            <input
              value={args}
              onChange={(event) => setArgs(event.target.value)}
              placeholder="space separated"
              className="mt-1 rounded-md border border-transparent bg-[color:var(--bg-secondary)]/70 px-3 py-2 text-sm text-[color:var(--text-primary)] focus:border-[color:var(--accent-1)]/60 focus:outline-none"
            />
          </label>
        </div>
        <div className="mt-3 flex items-center justify-between">
          <span className="text-xs text-[color:var(--text-secondary)]">
            Provide a path for WebAssembly modules or inline code for Micro VMs.
          </span>
          <button
            type="submit"
            disabled={isRunning}
            className="btn-primary rounded-md px-4 py-2 text-xs font-semibold uppercase tracking-wide"
          >
            {isRunning ? 'Running…' : 'Run'}
          </button>
        </div>
      </form>
      <div className="grid flex-1 gap-4 overflow-y-auto bg-[color:var(--bg-primary)]/70 p-4 md:grid-cols-2">
        <div className="panel rounded-lg p-4">
          <h2 className="mb-2 text-sm font-semibold uppercase tracking-wide text-[color:var(--text-secondary)]">Stdout</h2>
          <pre className="max-h-96 overflow-auto text-xs text-[color:var(--text-primary)]">
            {result?.stdout ?? 'Awaiting execution…'}
          </pre>
        </div>
        <div className="panel rounded-lg p-4">
          <h2 className="mb-2 text-sm font-semibold uppercase tracking-wide text-[color:var(--text-secondary)]">Stderr</h2>
          <pre className="max-h-96 overflow-auto text-xs text-[color:var(--text-primary)]">
            {result?.stderr ?? 'No stderr captured'}
          </pre>
        </div>
        <div className="panel rounded-lg p-4 md:col-span-2">
          <h2 className="mb-2 text-sm font-semibold uppercase tracking-wide text-[color:var(--text-secondary)]">Summary</h2>
          {error ? (
            <p className="text-sm text-red-400">{error}</p>
          ) : result ? (
            <div className="grid grid-cols-3 gap-2 text-sm text-[color:var(--text-primary)]">
              <div>
                <p className="text-[color:var(--text-secondary)]">Exit Code</p>
                <p>{result.exit_code}</p>
              </div>
              <div>
                <p className="text-[color:var(--text-secondary)]">Duration</p>
                <p>{result.duration_ms} ms</p>
              </div>
              <div>
                <p className="text-[color:var(--text-secondary)]">Output Size</p>
                <p>{result.stdout.length + result.stderr.length} bytes</p>
              </div>
            </div>
          ) : (
            <p className="text-sm text-[color:var(--text-secondary)]">Awaiting execution result…</p>
          )}
        </div>
      </div>
    </div>
  );
}
