import { useEffect, useState } from 'react';
import { Outlet, Route, Routes, useLocation } from 'react-router-dom';
import { Sidebar } from './components/Sidebar';
import { EditorView } from './components/EditorView';
import { AgentChat } from './components/AgentChat';
import { ExecutionView } from './components/ExecutionView';
import { TerminalView } from './components/TerminalView';
import { LLMPlayground } from './components/LLMPlayground';
import { AdminPanel } from './components/AdminPanel';
import { TopBar } from './components/TopBar';
import { StudioContextProvider } from './hooks/useStudioContext';

const tabs = [
  { name: 'Code', path: '/code' },
  { name: 'Chat', path: '/chat' },
  { name: 'Run', path: '/run' },
  { name: 'Terminal', path: '/terminal' },
  { name: 'LLM', path: '/llm' },
  { name: 'Admin', path: '/admin', protected: true }
] as const;

type Tab = (typeof tabs)[number];

function Layout() {
  const location = useLocation();
  const [activeTab, setActiveTab] = useState<Tab['path']>('/code');

  useEffect(() => {
    const normalized = location.pathname === '/' ? '/code' : (location.pathname as Tab['path']);
    setActiveTab(normalized);
  }, [location.pathname]);

  return (
    <div className="flex h-screen w-full overflow-hidden text-[color:var(--text-primary)]">
      <Sidebar tabs={tabs} activeTab={activeTab} onSelect={setActiveTab} />
      <main className="flex flex-1 flex-col overflow-hidden">
        <TopBar tabs={tabs} activeTab={activeTab} onSelect={setActiveTab} />
        <div className="flex-1 overflow-hidden">
          <Outlet />
        </div>
      </main>
    </div>
  );
}

export default function App() {
  return (
    <StudioContextProvider>
      <Routes>
        <Route path="/" element={<Layout />}>
          <Route index element={<EditorView />} />
          <Route path="code" element={<EditorView />} />
          <Route path="chat" element={<AgentChat />} />
          <Route path="run" element={<ExecutionView />} />
          <Route path="terminal" element={<TerminalView />} />
          <Route path="llm" element={<LLMPlayground />} />
          <Route path="admin" element={<AdminPanel />} />
        </Route>
      </Routes>
    </StudioContextProvider>
  );
}
