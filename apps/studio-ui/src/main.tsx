import React from 'react';
import ReactDOM from 'react-dom/client';

const App: React.FC = () => {
  return (
    <div className="min-h-screen bg-slate-950 text-cyan-100">
      <header className="p-6 border-b border-cyan-500/40">
        <h1 className="text-2xl font-semibold tracking-[0.35em] uppercase">
          CyberDevStudio
        </h1>
        <p className="mt-2 text-sm text-cyan-300/70">
          Placeholder UI shell – Monaco IDE, Agent Chat und Admin Dashboard folgen.
        </p>
      </header>
      <main className="p-6 grid gap-6">
        <section className="rounded-lg border border-cyan-500/30 bg-cyan-950/20 p-6">
          <h2 className="text-lg font-medium text-cyan-200">Agent Console</h2>
          <p className="mt-2 text-cyan-300/60">
            Streaming Chat, Toolaufrufe und Tokenverbrauch werden hier dargestellt.
          </p>
        </section>
        <section className="rounded-lg border border-cyan-500/30 bg-cyan-950/20 p-6">
          <h2 className="text-lg font-medium text-cyan-200">Sandbox Status</h2>
          <p className="mt-2 text-cyan-300/60">
            Prozessausführungen, FS-Operationen und Logs erscheinen in diesem Panel.
          </p>
        </section>
      </main>
    </div>
  );
};

ReactDOM.createRoot(document.getElementById('root') as HTMLElement).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>
);
