import { NavLink, useNavigate } from 'react-router-dom';
import clsx from 'clsx';
import { useStudioContext } from '../hooks/useStudioContext';

type TabDefinition = {
  name: string;
  path: string;
  protected?: boolean;
};

interface SidebarProps {
  tabs: readonly TabDefinition[];
  activeTab: string;
  onSelect: (path: string) => void;
}

export function Sidebar({ tabs, activeTab, onSelect }: SidebarProps) {
  const navigate = useNavigate();
  const { profile } = useStudioContext();

  return (
    <aside className="hidden h-full w-64 flex-col border-r border-slate-800/60 bg-[color:var(--bg-secondary)]/70 p-4 text-sm lg:flex">
      <div className="mb-6 flex items-center justify-between">
        <div>
          <p className="text-lg font-semibold tracking-tight text-[color:var(--accent-1)]">CyberDevStudio</p>
          <p className="text-xs text-[color:var(--text-secondary)]">{profile.role.toUpperCase()} MODE</p>
        </div>
        <button
          onClick={() => navigate('/admin')}
          className="rounded-md border border-[color:var(--accent-1)]/40 px-2 py-1 text-xs text-[color:var(--accent-1)] hover:bg-[color:var(--accent-1)]/10"
        >
          Admin
        </button>
      </div>
      <nav className="flex-1 space-y-1">
        {tabs.map((tab) => {
          if (tab.protected && profile.role !== 'admin') {
            return null;
          }
          return (
            <NavLink
              key={tab.path}
              to={tab.path}
              onClick={() => onSelect(tab.path)}
              className={({ isActive }) =>
                clsx(
                  'flex items-center rounded-md px-3 py-2 transition',
                  isActive || activeTab === tab.path
                    ? 'bg-[color:var(--accent-1)]/20 text-[color:var(--accent-1)] shadow-glow'
                    : 'text-[color:var(--text-secondary)] hover:bg-[color:var(--accent-1)]/10'
                )
              }
            >
              {tab.name}
            </NavLink>
          );
        })}
      </nav>
      <div className="mt-auto rounded-lg border border-slate-700/40 bg-[color:var(--panel)] p-3">
        <p className="text-xs uppercase tracking-wide text-[color:var(--text-secondary)]">Session</p>
        <p className="text-sm font-semibold text-[color:var(--text-primary)]">{profile.username}</p>
        <p className="text-xs text-[color:var(--accent-1)]">Token Balance: {profile.tokenBalance}</p>
      </div>
    </aside>
  );
}
