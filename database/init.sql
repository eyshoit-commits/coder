-- CyberDevStudio database bootstrap for Supabase + PostgreSQL + PostgresML
-- Run this script once inside your Supabase SQL editor or any psql session
-- connected to the project database.

BEGIN;

-- Required extensions
CREATE EXTENSION IF NOT EXISTS "uuid-ossp";
CREATE EXTENSION IF NOT EXISTS pgcrypto;
CREATE EXTENSION IF NOT EXISTS citext;
CREATE EXTENSION IF NOT EXISTS vector;
CREATE EXTENSION IF NOT EXISTS postgresml;

-- Dedicated schemas to keep responsibilities separated
CREATE SCHEMA IF NOT EXISTS platform AUTHORIZATION CURRENT_USER;
CREATE SCHEMA IF NOT EXISTS telemetry AUTHORIZATION CURRENT_USER;
CREATE SCHEMA IF NOT EXISTS analytics AUTHORIZATION CURRENT_USER;

SET search_path TO platform, public;

-- Shared helper to automatically bump updated_at columns
CREATE OR REPLACE FUNCTION platform.touch_updated_at()
RETURNS TRIGGER AS $$
BEGIN
  NEW.updated_at := NOW();
  RETURN NEW;
END;
$$ LANGUAGE plpgsql;

-- Users & auth ---------------------------------------------------------------
CREATE TABLE IF NOT EXISTS platform.users (
  id                UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  created_at        TIMESTAMPTZ NOT NULL DEFAULT NOW(),
  updated_at        TIMESTAMPTZ NOT NULL DEFAULT NOW(),
  username          CITEXT UNIQUE NOT NULL,
  email             CITEXT UNIQUE NOT NULL,
  password_hash     TEXT,
  role              TEXT NOT NULL CHECK (role IN ('admin', 'developer', 'viewer')),
  api_key_hash      TEXT,
  balance_tokens    BIGINT NOT NULL DEFAULT 0,
  last_login_at     TIMESTAMPTZ,
  metadata          JSONB NOT NULL DEFAULT '{}'::JSONB
);

CREATE TRIGGER trg_users_updated_at
BEFORE UPDATE ON platform.users
FOR EACH ROW EXECUTE FUNCTION platform.touch_updated_at();

COMMENT ON TABLE platform.users IS 'Primary user table shared between Supabase auth and the CyberDevStudio platform.';

-- API keys ------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS platform.api_keys (
  id             UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  created_at     TIMESTAMPTZ NOT NULL DEFAULT NOW(),
  updated_at     TIMESTAMPTZ NOT NULL DEFAULT NOW(),
  user_id        UUID NOT NULL REFERENCES platform.users(id) ON DELETE CASCADE,
  name           TEXT NOT NULL,
  api_key_hash   TEXT NOT NULL,
  expires_at     TIMESTAMPTZ,
  last_used_at   TIMESTAMPTZ,
  scopes         TEXT[] NOT NULL DEFAULT ARRAY['llm:read'],
  is_revoked     BOOLEAN NOT NULL DEFAULT FALSE
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_api_keys_user_name
  ON platform.api_keys (user_id, name);

CREATE TRIGGER trg_api_keys_updated_at
BEFORE UPDATE ON platform.api_keys
FOR EACH ROW EXECUTE FUNCTION platform.touch_updated_at();

-- Models --------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS platform.models (
  id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
  updated_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
  name            TEXT NOT NULL UNIQUE,
  provider        TEXT NOT NULL DEFAULT 'local',
  repo_url        TEXT,
  context_size    INTEGER NOT NULL DEFAULT 4096,
  cost_per_1k_tokens NUMERIC(12,6) NOT NULL DEFAULT 0,
  is_default      BOOLEAN NOT NULL DEFAULT FALSE,
  metadata        JSONB NOT NULL DEFAULT '{}'::JSONB
);

CREATE TRIGGER trg_models_updated_at
BEFORE UPDATE ON platform.models
FOR EACH ROW EXECUTE FUNCTION platform.touch_updated_at();

-- Token accounting -----------------------------------------------------------
CREATE TABLE IF NOT EXISTS platform.token_usage (
  id             BIGSERIAL PRIMARY KEY,
  occurred_at    TIMESTAMPTZ NOT NULL DEFAULT NOW(),
  user_id        UUID NOT NULL REFERENCES platform.users(id) ON DELETE CASCADE,
  model_id       UUID REFERENCES platform.models(id) ON DELETE SET NULL,
  request_id     UUID NOT NULL,
  prompt_tokens  INTEGER NOT NULL DEFAULT 0,
  completion_tokens INTEGER NOT NULL DEFAULT 0,
  cost_tokens    INTEGER NOT NULL DEFAULT 0,
  metadata       JSONB NOT NULL DEFAULT '{}'::JSONB
);

CREATE INDEX IF NOT EXISTS idx_token_usage_user_time
  ON platform.token_usage (user_id, occurred_at DESC);

CREATE INDEX IF NOT EXISTS idx_token_usage_request
  ON platform.token_usage (request_id);

-- Projects ------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS platform.projects (
  id             UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  created_at     TIMESTAMPTZ NOT NULL DEFAULT NOW(),
  updated_at     TIMESTAMPTZ NOT NULL DEFAULT NOW(),
  owner_id       UUID NOT NULL REFERENCES platform.users(id) ON DELETE CASCADE,
  name           TEXT NOT NULL,
  description    TEXT,
  visibility     TEXT NOT NULL DEFAULT 'private' CHECK (visibility IN ('private', 'internal', 'public')),
  metadata       JSONB NOT NULL DEFAULT '{}'::JSONB
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_projects_owner_name
  ON platform.projects (owner_id, name);

CREATE TRIGGER trg_projects_updated_at
BEFORE UPDATE ON platform.projects
FOR EACH ROW EXECUTE FUNCTION platform.touch_updated_at();

-- Sessions ------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS platform.sessions (
  id            UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  created_at    TIMESTAMPTZ NOT NULL DEFAULT NOW(),
  updated_at    TIMESTAMPTZ NOT NULL DEFAULT NOW(),
  user_id       UUID NOT NULL REFERENCES platform.users(id) ON DELETE CASCADE,
  project_id    UUID REFERENCES platform.projects(id) ON DELETE SET NULL,
  model_id      UUID REFERENCES platform.models(id) ON DELETE SET NULL,
  status        TEXT NOT NULL DEFAULT 'active' CHECK (status IN ('active', 'idle', 'closed')),
  last_activity TIMESTAMPTZ NOT NULL DEFAULT NOW(),
  metadata      JSONB NOT NULL DEFAULT '{}'::JSONB
);

CREATE INDEX IF NOT EXISTS idx_sessions_user_status
  ON platform.sessions (user_id, status);

CREATE TRIGGER trg_sessions_updated_at
BEFORE UPDATE ON platform.sessions
FOR EACH ROW EXECUTE FUNCTION platform.touch_updated_at();

-- Telemetry schema -----------------------------------------------------------
SET search_path TO telemetry, public;

CREATE TABLE IF NOT EXISTS telemetry.agent_events (
  id            BIGSERIAL PRIMARY KEY,
  created_at    TIMESTAMPTZ NOT NULL DEFAULT NOW(),
  session_id    UUID REFERENCES platform.sessions(id) ON DELETE CASCADE,
  user_id       UUID REFERENCES platform.users(id) ON DELETE CASCADE,
  event_type    TEXT NOT NULL,
  message       TEXT,
  payload       JSONB NOT NULL DEFAULT '{}'::JSONB
);

CREATE INDEX IF NOT EXISTS idx_agent_events_session
  ON telemetry.agent_events (session_id, created_at DESC);

CREATE TABLE IF NOT EXISTS telemetry.execution_metrics (
  id            BIGSERIAL PRIMARY KEY,
  collected_at  TIMESTAMPTZ NOT NULL DEFAULT NOW(),
  project_id    UUID REFERENCES platform.projects(id) ON DELETE CASCADE,
  cpu_percent   NUMERIC(5,2),
  memory_mb     NUMERIC(10,2),
  duration_ms   INTEGER,
  outcome       TEXT NOT NULL DEFAULT 'success',
  metadata      JSONB NOT NULL DEFAULT '{}'::JSONB
);

CREATE INDEX IF NOT EXISTS idx_execution_metrics_project
  ON telemetry.execution_metrics (project_id, collected_at DESC);

-- Analytics schema -----------------------------------------------------------
SET search_path TO analytics, public;

CREATE MATERIALIZED VIEW IF NOT EXISTS analytics.daily_token_summary AS
SELECT
  date_trunc('day', occurred_at) AS day,
  user_id,
  SUM(prompt_tokens) AS prompt_tokens,
  SUM(completion_tokens) AS completion_tokens,
  SUM(cost_tokens) AS cost_tokens
FROM platform.token_usage
GROUP BY 1, 2
WITH NO DATA;

CREATE UNIQUE INDEX IF NOT EXISTS idx_daily_token_summary_day_user
  ON analytics.daily_token_summary (day, user_id);

CREATE OR REPLACE FUNCTION analytics.refresh_daily_token_summary()
RETURNS VOID
LANGUAGE plpgsql
SECURITY DEFINER
AS $$
BEGIN
  REFRESH MATERIALIZED VIEW CONCURRENTLY analytics.daily_token_summary;
END;
$$;

-- RPC helpers (callable from Supabase client) --------------------------------
SET search_path TO platform, public;

CREATE OR REPLACE FUNCTION platform.record_token_usage(
  p_user_id UUID,
  p_model_name TEXT,
  p_request_id UUID,
  p_prompt_tokens INTEGER,
  p_completion_tokens INTEGER,
  p_metadata JSONB DEFAULT '{}'::JSONB
) RETURNS VOID
LANGUAGE plpgsql
SECURITY DEFINER
AS $$
DECLARE
  v_model_id UUID;
  v_cost_tokens INTEGER;
BEGIN
  SELECT id INTO v_model_id FROM platform.models WHERE name = p_model_name;
  v_cost_tokens := COALESCE(p_prompt_tokens, 0) + COALESCE(p_completion_tokens, 0);

  INSERT INTO platform.token_usage (user_id, model_id, request_id, prompt_tokens, completion_tokens, cost_tokens, metadata)
  VALUES (p_user_id, v_model_id, p_request_id, p_prompt_tokens, p_completion_tokens, v_cost_tokens, p_metadata);

  UPDATE platform.users
  SET balance_tokens = GREATEST(balance_tokens - v_cost_tokens, 0)
  WHERE id = p_user_id;
END;
$$;

CREATE OR REPLACE FUNCTION platform.grant_user_tokens(
  p_user_id UUID,
  p_amount BIGINT,
  p_reason TEXT DEFAULT NULL
) RETURNS BIGINT
LANGUAGE plpgsql
SECURITY DEFINER
AS $$
DECLARE
  v_new_balance BIGINT;
BEGIN
  UPDATE platform.users
  SET balance_tokens = balance_tokens + p_amount
  WHERE id = p_user_id
  RETURNING balance_tokens INTO v_new_balance;

  INSERT INTO platform.token_usage (user_id, request_id, prompt_tokens, completion_tokens, cost_tokens, metadata)
  VALUES (
    p_user_id,
    gen_random_uuid(),
    0,
    0,
    -p_amount,
    jsonb_build_object('event', 'grant', 'reason', p_reason)
  );

  RETURN v_new_balance;
END;
$$;

CREATE OR REPLACE FUNCTION platform.get_usage_overview(
  p_user_id UUID,
  p_limit INTEGER DEFAULT 50
) RETURNS TABLE (
  occurred_at TIMESTAMPTZ,
  model_name TEXT,
  prompt_tokens INTEGER,
  completion_tokens INTEGER,
  cost_tokens INTEGER,
  metadata JSONB
)
LANGUAGE sql
SECURITY DEFINER
AS $$
  SELECT
    tu.occurred_at,
    m.name AS model_name,
    tu.prompt_tokens,
    tu.completion_tokens,
    tu.cost_tokens,
    tu.metadata
  FROM platform.token_usage tu
  LEFT JOIN platform.models m ON m.id = tu.model_id
  WHERE tu.user_id = p_user_id
  ORDER BY tu.occurred_at DESC
  LIMIT p_limit;
$$;

CREATE OR REPLACE FUNCTION platform.set_default_model(p_model_id UUID)
RETURNS VOID
LANGUAGE plpgsql
SECURITY DEFINER
AS $$
BEGIN
  UPDATE platform.models SET is_default = FALSE;
  UPDATE platform.models SET is_default = TRUE WHERE id = p_model_id;
END;
$$;

-- Basic policies for Supabase row-level security (RLS) ----------------------
ALTER TABLE platform.users ENABLE ROW LEVEL SECURITY;
ALTER TABLE platform.projects ENABLE ROW LEVEL SECURITY;
ALTER TABLE platform.sessions ENABLE ROW LEVEL SECURITY;
ALTER TABLE platform.token_usage ENABLE ROW LEVEL SECURITY;
ALTER TABLE platform.api_keys ENABLE ROW LEVEL SECURITY;

CREATE POLICY users_self_access ON platform.users
  USING (auth.uid() = id)
  WITH CHECK (auth.uid() = id);

CREATE POLICY projects_owner_access ON platform.projects
  USING (auth.uid() = owner_id)
  WITH CHECK (auth.uid() = owner_id);

CREATE POLICY sessions_owner_access ON platform.sessions
  USING (auth.uid() = user_id)
  WITH CHECK (auth.uid() = user_id);

CREATE POLICY token_usage_owner_access ON platform.token_usage
  USING (auth.uid() = user_id);

CREATE POLICY api_keys_owner_access ON platform.api_keys
  USING (auth.uid() = user_id)
  WITH CHECK (auth.uid() = user_id);

-- Privileges for service role ------------------------------------------------
GRANT USAGE ON SCHEMA platform, telemetry, analytics TO postgres;
GRANT ALL PRIVILEGES ON ALL TABLES IN SCHEMA platform TO postgres;
GRANT ALL PRIVILEGES ON ALL TABLES IN SCHEMA telemetry TO postgres;
GRANT ALL PRIVILEGES ON ALL TABLES IN SCHEMA analytics TO postgres;
GRANT ALL PRIVILEGES ON ALL FUNCTIONS IN SCHEMA platform TO postgres;

-- Refresh analytics after initial load
REFRESH MATERIALIZED VIEW CONCURRENTLY analytics.daily_token_summary;

COMMIT;
