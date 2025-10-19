import { useMemo } from 'react';
import clsx from 'clsx';
import { useNavigate } from 'react-router-dom';
import { useStudioContext } from '../hooks/useStudioContext';

type TabDefinition = {
  name: string;
  path: string;
  protected?: boolean;
};

interface TopBarProps {
  tabs: readonly TabDefinition[];
  activeTab: string;
  onSelect: (path: string) => void;
}

export function TopBar({ tabs, activeTab, onSelect }: TopBarProps) {
  const navigate = useNavigate();
  const { profile, switchTheme } = useStudioContext();
  const availableTabs = useMemo(
    () => tabs.filter((tab) => !tab.protected || profile.role === 'admin'),
    [tabs, profile.role]
  );

  return (
    <header className="flex h-16 items-center justify-between border-b border-slate-800/60 bg-[color:var(--bg-secondary)]/70 px-6">
      <nav className="flex items-center space-x-2 overflow-x-auto">
        {availableTabs.map((tab) => (
          <button
            key={tab.path}
            onClick={() => {
              navigate(tab.path);
              onSelect(tab.path);
            }}
            className={clsx(
              'rounded-full px-4 py-2 text-sm font-medium transition',
              activeTab === tab.path
                ? 'bg-[color:var(--accent-1)] text-slate-950 shadow-glow'
                : 'bg-transparent text-[color:var(--text-secondary)] hover:bg-[color:var(--accent-1)]/10'
            )}
          >
            {tab.name}
          </button>
        ))}
      </nav>
      <div className="flex items-center space-x-3 text-xs uppercase tracking-wide text-[color:var(--text-secondary)]">
        <span>{profile.environment}</span>
        <span className="text-[color:var(--accent-1)]">Tokens {profile.tokenBalance}</span>
        <button
          onClick={switchTheme}
          className="rounded-full border border-[color:var(--accent-1)]/40 px-3 py-1 text-[color:var(--accent-1)] hover:bg-[color:var(--accent-1)]/10"
        >
          Toggle Theme
        </button>
      </div>
    </header>
  );
}
