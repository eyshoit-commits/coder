import { createContext, useContext, useEffect, useMemo, useState } from 'react';
import { RpcClient } from '../api/rpc';

export type UserRole = 'admin' | 'developer' | 'viewer';

export interface StudioProfile {
  username: string;
  role: UserRole;
  tokenBalance: number;
  environment: string;
  accessToken?: string;
  apiKey?: string;
}

interface StudioContextValue {
  profile: StudioProfile;
  updateProfile: (profile: Partial<StudioProfile>) => void;
  rpc: RpcClient;
  switchTheme: () => void;
  refreshTokenUsage: () => Promise<void>;
}

const defaultProfile: StudioProfile = {
  username: 'developer',
  role: 'developer',
  tokenBalance: 0,
  environment: 'DEV'
};

const StudioContext = createContext<StudioContextValue | undefined>(undefined);

const STORAGE_KEY = 'cyberdevstudio.profile';

function loadStoredProfile(): StudioProfile {
  try {
    const serialized = window.localStorage.getItem(STORAGE_KEY);
    if (!serialized) {
      return defaultProfile;
    }
    const parsed = JSON.parse(serialized) as StudioProfile;
    return { ...defaultProfile, ...parsed };
  } catch (error) {
    console.warn('Unable to parse profile from storage', error);
    return defaultProfile;
  }
}

function persistProfile(profile: StudioProfile) {
  try {
    window.localStorage.setItem(STORAGE_KEY, JSON.stringify(profile));
  } catch (error) {
    console.warn('Unable to persist profile', error);
  }
}

export function StudioContextProvider({ children }: { children: React.ReactNode }) {
  const [profile, setProfile] = useState<StudioProfile>(() => {
    if (typeof window === 'undefined') {
      return defaultProfile;
    }
    return loadStoredProfile();
  });

  const endpoint = import.meta.env.VITE_RPC_URL ?? '/rpc';
  const rpc = useMemo(() => new RpcClient(endpoint), [endpoint]);

  useEffect(() => {
    rpc.setBearerToken(profile.accessToken);
    rpc.setApiKey(profile.apiKey);
    if (typeof window !== 'undefined') {
      persistProfile(profile);
    }
  }, [profile, rpc]);

  const switchTheme = () => {
    if (typeof document === 'undefined') {
      return;
    }
    const body = document.body;
    if (body.classList.contains('theme-neon')) {
      body.classList.remove('theme-neon');
      body.classList.add('theme-steel');
      window.localStorage.setItem('cyberdevstudio.theme', 'theme-steel');
    } else {
      body.classList.remove('theme-steel');
      body.classList.add('theme-neon');
      window.localStorage.setItem('cyberdevstudio.theme', 'theme-neon');
    }
  };

  const refreshTokenUsage = async () => {
    try {
      const response = await rpc.call<{ remaining: number }>('session.tokens.remaining');
      if (typeof response?.remaining === 'number') {
        setProfile((previous) => ({ ...previous, tokenBalance: response.remaining }));
      }
    } catch (error) {
      console.warn('Failed to refresh token usage', error);
    }
  };

  const updateProfile = (patch: Partial<StudioProfile>) => {
    setProfile((current) => ({ ...current, ...patch }));
  };

  const value: StudioContextValue = {
    profile,
    updateProfile,
    rpc,
    switchTheme,
    refreshTokenUsage
  };

  return <StudioContext.Provider value={value}>{children}</StudioContext.Provider>;
}

export function useStudioContext(): StudioContextValue {
  const ctx = useContext(StudioContext);
  if (!ctx) {
    throw new Error('useStudioContext must be used within StudioContextProvider');
  }
  return ctx;
}
