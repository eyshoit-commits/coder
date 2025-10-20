import { createClient } from '@supabase/supabase-js';

const SUPABASE_URL = process.env.NEXT_PUBLIC_SUPABASE_URL || process.env.SUPABASE_URL || '';
const SUPABASE_ANON_KEY = process.env.NEXT_PUBLIC_SUPABASE_ANON_KEY || process.env.SUPABASE_ANON_KEY || '';
const SUPABASE_SERVICE_ROLE_KEY = process.env.SUPABASE_SERVICE_ROLE_KEY;

let cachedClient;
let cachedServiceRoleClient;

function ensureUrlAndKey(url, key, { serviceRole = false } = {}) {
  if (!url) {
    throw new Error('Supabase URL is missing. Set NEXT_PUBLIC_SUPABASE_URL or SUPABASE_URL.');
  }
  if (!key) {
    throw new Error(
      serviceRole
        ? 'Supabase service-role key is missing. Set SUPABASE_SERVICE_ROLE_KEY.'
        : 'Supabase anon key is missing. Set NEXT_PUBLIC_SUPABASE_ANON_KEY or SUPABASE_ANON_KEY.'
    );
  }
}

/**
 * Returns the singleton Supabase client configured for the `platform` schema.
 * @param {object} [options]
 * @param {import('@supabase/supabase-js').SupabaseClient} [options.override]
 * @returns {import('@supabase/supabase-js').SupabaseClient}
 */
export function getSupabaseClient(options = {}) {
  if (options.override) {
    return options.override;
  }

  if (!cachedClient) {
    ensureUrlAndKey(SUPABASE_URL, SUPABASE_ANON_KEY);
    cachedClient = createClient(SUPABASE_URL, SUPABASE_ANON_KEY, {
      auth: {
        persistSession: true,
        autoRefreshToken: true,
      },
      db: {
        schema: 'platform',
      },
    });
  }

  return cachedClient;
}

/**
 * Returns a service-role Supabase client (no session persistence).
 * @returns {import('@supabase/supabase-js').SupabaseClient}
 */
export function getServiceRoleClient() {
  ensureUrlAndKey(SUPABASE_URL, SUPABASE_SERVICE_ROLE_KEY, { serviceRole: true });

  if (!cachedServiceRoleClient) {
    cachedServiceRoleClient = createClient(SUPABASE_URL, SUPABASE_SERVICE_ROLE_KEY, {
      auth: {
        persistSession: false,
        autoRefreshToken: false,
      },
      db: {
        schema: 'platform',
      },
    });
  }

  return cachedServiceRoleClient;
}

export const supabaseClient = getSupabaseClient();

async function resolveClient({ serviceRole = false, client } = {}) {
  if (client) {
    return client;
  }
  return serviceRole ? getServiceRoleClient() : getSupabaseClient();
}

/** Auth -------------------------------------------------------------------- */
export async function signInWithEmail({ email, password }) {
  const client = await resolveClient();
  const { data, error } = await client.auth.signInWithPassword({ email, password });
  if (error) throw error;
  return data.user;
}

export async function signInWithOtp({ email }) {
  const client = await resolveClient();
  const { error } = await client.auth.signInWithOtp({ email });
  if (error) throw error;
  return true;
}

export async function signOut() {
  const client = await resolveClient();
  const { error } = await client.auth.signOut();
  if (error) throw error;
  return true;
}

export async function getCurrentUser() {
  const client = await resolveClient();
  const { data, error } = await client.auth.getUser();
  if (error) throw error;
  return data.user ?? null;
}

export async function refreshSession() {
  const client = await resolveClient();
  const { data, error } = await client.auth.refreshSession();
  if (error) throw error;
  return data.session;
}

/** Profiles & users -------------------------------------------------------- */
export async function upsertProfile(payload) {
  const client = await resolveClient();
  const { data, error } = await client.from('users').upsert(payload).select().single();
  if (error) throw error;
  return data;
}

export async function getUserBalance(userId, { serviceRole = false } = {}) {
  const client = await resolveClient({ serviceRole });
  const { data, error } = await client
    .from('users')
    .select('id, balance_tokens, role, metadata')
    .eq('id', userId)
    .maybeSingle();
  if (error) throw error;
  return data;
}

export async function grantTokens(userId, amount, reason = null) {
  const client = await resolveClient({ serviceRole: true });
  return callRpc('grant_user_tokens', {
    p_user_id: userId,
    p_amount: amount,
    p_reason: reason,
  }, { client });
}

export async function fetchUsageOverview(userId, limit = 50) {
  return callRpc('get_usage_overview', {
    p_user_id: userId,
    p_limit: limit,
  });
}

/** Projects ---------------------------------------------------------------- */
export async function listProjects({ ownerId } = {}) {
  const client = await resolveClient();
  let query = client
    .from('projects')
    .select('id, name, description, visibility, metadata, created_at, updated_at', { count: 'exact' })
    .order('created_at', { ascending: false });

  if (ownerId) {
    query = query.eq('owner_id', ownerId);
  }

  const { data, error } = await query;
  if (error) throw error;
  return data;
}

export async function createProject(payload) {
  const client = await resolveClient();
  const { data, error } = await client.from('projects').insert(payload).select().single();
  if (error) throw error;
  return data;
}

export async function updateProject(id, patch) {
  const client = await resolveClient();
  const { data, error } = await client.from('projects').update(patch).eq('id', id).select().single();
  if (error) throw error;
  return data;
}

export async function deleteProject(id) {
  const client = await resolveClient();
  const { error } = await client.from('projects').delete().eq('id', id);
  if (error) throw error;
  return true;
}

/** Models ------------------------------------------------------------------ */
export async function listModels({ onlyActive = false } = {}) {
  const client = await resolveClient();
  let query = client
    .from('models')
    .select('id, name, provider, context_size, cost_per_1k_tokens, is_default, metadata')
    .order('is_default', { ascending: false })
    .order('name');
  if (onlyActive) {
    query = query.eq('metadata->>status', 'active');
  }
  const { data, error } = await query;
  if (error) throw error;
  return data;
}

export async function setDefaultModel(modelId) {
  return callRpc('set_default_model', { p_model_id: modelId }, { serviceRole: true });
}

export async function recordTokenUsage({ userId, modelName, requestId, promptTokens, completionTokens, metadata = {} }) {
  return callRpc('record_token_usage', {
    p_user_id: userId,
    p_model_name: modelName,
    p_request_id: requestId,
    p_prompt_tokens: promptTokens,
    p_completion_tokens: completionTokens,
    p_metadata: metadata,
  }, { serviceRole: true });
}

export async function fetchTokenUsage({ userId, limit = 100 } = {}) {
  const client = await resolveClient();
  let query = client
    .from('token_usage')
    .select('id, occurred_at, prompt_tokens, completion_tokens, cost_tokens, metadata, model_id')
    .order('occurred_at', { ascending: false })
    .limit(limit);

  if (userId) {
    query = query.eq('user_id', userId);
  }

  const { data, error } = await query;
  if (error) throw error;
  return data;
}

/** Sessions ---------------------------------------------------------------- */
export async function getProjectSessions(projectId) {
  const client = await resolveClient();
  const { data, error } = await client
    .from('sessions')
    .select('id, status, model_id, last_activity, metadata, created_at, user_id')
    .eq('project_id', projectId)
    .order('last_activity', { ascending: false });
  if (error) throw error;
  return data;
}

export async function closeSession(sessionId) {
  const client = await resolveClient();
  const { data, error } = await client
    .from('sessions')
    .update({ status: 'closed', updated_at: new Date().toISOString() })
    .eq('id', sessionId)
    .select()
    .single();
  if (error) throw error;
  return data;
}

/** Telemetry --------------------------------------------------------------- */
export async function logAgentEvent(payload) {
  const client = await resolveClient({ serviceRole: true });
  const { data, error } = await client
    .schema('telemetry')
    .from('agent_events')
    .insert(payload)
    .select()
    .single();
  if (error) throw error;
  return data;
}

export async function fetchExecutionMetrics({ projectId, limit = 50 } = {}) {
  const client = await resolveClient({ serviceRole: true });
  let query = client
    .schema('telemetry')
    .from('execution_metrics')
    .select('id, collected_at, cpu_percent, memory_mb, duration_ms, outcome, metadata, project_id')
    .order('collected_at', { ascending: false })
    .limit(limit);
  if (projectId) {
    query = query.eq('project_id', projectId);
  }
  const { data, error } = await query;
  if (error) throw error;
  return data;
}

/** Analytics --------------------------------------------------------------- */
export async function refreshAnalyticsSummary() {
  return callRpc('refresh_daily_token_summary', {}, { serviceRole: true });
}

export async function fetchDailyTokenSummary({ day, userId, limit = 30 } = {}) {
  const client = await resolveClient({ serviceRole: true });
  let query = client
    .schema('analytics')
    .from('daily_token_summary')
    .select('day, user_id, prompt_tokens, completion_tokens, cost_tokens')
    .order('day', { ascending: false })
    .limit(limit);
  if (day) {
    query = query.eq('day', day);
  }
  if (userId) {
    query = query.eq('user_id', userId);
  }
  const { data, error } = await query;
  if (error) throw error;
  return data;
}

/** Utilities --------------------------------------------------------------- */
export async function callRpc(fn, params = {}, options = {}) {
  const client = await resolveClient(options);
  const { data, error } = await client.rpc(fn, params);
  if (error) throw error;
  return data;
}

export async function withServiceRole(fn) {
  const client = await resolveClient({ serviceRole: true });
  return fn(client);
}

export function resetSupabaseClients() {
  cachedClient = undefined;
  cachedServiceRoleClient = undefined;
}

/**
 * Convenience helper that wraps a fetch call with Supabase auth header if a session exists.
 * Useful for calling CyberDevStudio APIs that require authenticated Supabase tokens.
 */
export async function withAuthFetch(input, init = {}) {
  const client = await resolveClient();
  const session = await client.auth.getSession();
  const headers = new Headers(init.headers || {});
  if (session?.data?.session?.access_token) {
    headers.set('Authorization', `Bearer ${session.data.session.access_token}`);
  }
  return fetch(input, { ...init, headers });
}

export default supabaseClient;
