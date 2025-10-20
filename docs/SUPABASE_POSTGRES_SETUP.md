# Supabase & PostgreSQL Hosting Guide

This guide walks you through provisioning the complete data layer for **CyberDevStudio**, including Supabase configuration, PostgreSQL hardening, and PostgresML enablement. Follow the steps sequentially—each section builds on the previous one.

---

## 1. Prerequisites

- Supabase account with project owner permissions
- Local tooling: [`psql`](https://www.postgresql.org/download/), [Supabase CLI](https://supabase.com/docs/guides/cli), [`curl`](https://curl.se/) for verification
- Optional: Docker (for local PostgresML testing)
- Copy of the repository so you can reference the SQL and environment files shipped with this guide

Before starting, duplicate `.env.local.example` into `.env.local` (frontend) and `.env` (backend/server processes) so you can collect credentials as you go.

---

## 2. Create a Supabase Project

1. Sign in to [Supabase](https://supabase.com/dashboard/projects).
2. Click **New project**, choose a strong database password, and pick the closest region to your users.
3. Wait for the project to provision (~2 minutes). Once ready, copy:
   - **Project URL**
   - **anon key**
   - **service_role key**
   - **JWT secret**
4. Paste the values into your environment file placeholders:
   ```bash
   cp .env.local.example .env.local
   cp .env.local .env
   ````
   Edit `.env.local` with the retrieved secrets.

> **Tip:** Enable **Point-in-time recovery** and set daily backups under *Database → Backups* for production projects.

---

## 3. Apply the Platform Schema

1. Open the Supabase dashboard and navigate to **SQL Editor**.
2. Paste the contents of [`database/init.sql`](../database/init.sql) and execute.
3. Confirm the script finishes without errors. It will:
   - Install required extensions (`uuid-ossp`, `pgcrypto`, `citext`, `vector`, `postgresml`)
   - Create `platform`, `telemetry`, and `analytics` schemas
   - Provision tables for users, API keys, projects, sessions, and token accounting
   - Register helpful RPC functions (`record_token_usage`, `grant_user_tokens`, `get_usage_overview`, `set_default_model`)
   - Enable row-level security (RLS) with default policies
   - Refresh the analytics materialized view for the first time

4. Verify the tables exist under **Table Editor**. You should see the three schemas populated with objects.

---

## 4. PostgresML Activation

Supabase ships with PostgresML in dedicated instances. To enable it:

1. Go to **Database → Extensions** and confirm `postgresml` is listed.
2. The SQL script already issues `CREATE EXTENSION IF NOT EXISTS postgresml;`. If it fails due to missing privileges, run the command manually as the `postgres` user.
3. Once loaded, test an inference call:
   ```sql
   SELECT postgresml.embed('bge-small-en-v1.5', ARRAY['CyberDevStudio makes agent workflows easy.']);
   ```
4. Store model metadata in `platform.models` for quick lookups. Example:
   ```sql
   INSERT INTO platform.models (name, provider, repo_url, context_size, cost_per_1k_tokens, metadata)
   VALUES ('bge-small-en-v1.5', 'postgresml', 'https://huggingface.co/BAAI/bge-small-en-v1.5', 1024, 0.0001,
           jsonb_build_object('type', 'embedding'))
   ON CONFLICT (name) DO NOTHING;
   ```

---

## 5. Configure Authentication

- Head to **Authentication → Providers** and enable the flows you need (Email, OAuth, Magic Links, etc.).
- Update redirect URLs to include your local dev URL (e.g. `http://localhost:3000`) and production domain.
- In **Authentication → Policies**, ensure email confirmations are enabled for user projects.

The `platform.users` table is designed to sync with Supabase Auth via [database triggers](https://supabase.com/docs/guides/auth/managing-user-data). Add the following SQL if you want automatic mirroring:

```sql
CREATE OR REPLACE FUNCTION public.handle_new_user()
RETURNS trigger AS $$
BEGIN
  INSERT INTO platform.users (id, username, email, role)
  VALUES (NEW.id, NEW.email, NEW.email, 'developer')
  ON CONFLICT (id) DO NOTHING;
  RETURN NEW;
END;
$$ LANGUAGE plpgsql SECURITY DEFINER;

DROP TRIGGER IF EXISTS on_auth_user_created ON auth.users;
CREATE TRIGGER on_auth_user_created
  AFTER INSERT ON auth.users
  FOR EACH ROW EXECUTE FUNCTION public.handle_new_user();
```

> Adjust the default role if you prefer new users to register as `viewer`.

---

## 6. Grant Service Role Privileges

To allow background workers (API, LLM server) to bypass RLS when required, connect with the `service_role` key and verify it has access:

```sql
-- As the service_role user
SET ROLE postgres;
SELECT count(*) FROM platform.users;
```

The initialization script already grants schema usage and table privileges to `postgres`. If you use a custom service role, repeat the `GRANT` statements with that role name.

---

## 7. Local Development Connection

1. Install the Supabase CLI and link the project:
   ```bash
   supabase login
   supabase link --project-ref <project-ref>
   ```
2. Pull secrets into `.env.local` automatically:
   ```bash
   supabase secrets pull --env-file .env.local
   ```
3. For offline development, start the local stack:
   ```bash
   supabase start
   ```
   This spins up Postgres, Auth, Storage, and the Edge runtime using Docker.

> Remember to apply `database/init.sql` to the local Postgres container as well using `supabase db reset` or `supabase db push`.

---

## 8. Observability & Maintenance

- **Backups:** Verify automated backups in Supabase or configure WAL-G for self-managed clusters.
- **Monitoring:** Forward metrics to Prometheus using the `telemetry.execution_metrics` table and OpenTelemetry exporters.
- **Vacuuming:** Schedule `VACUUM ANALYZE` during low-traffic windows, especially for `token_usage`.
- **Materialized views:** Automate `REFRESH MATERIALIZED VIEW CONCURRENTLY analytics.daily_token_summary;` via Supabase scheduled functions or external cron.

---

## 9. Troubleshooting

| Symptom | Possible Cause | Fix |
| ------- | -------------- | --- |
| `permission denied for schema platform` | RLS misconfiguration | Check that the caller uses the `service_role` key or add policies |
| `postgresml extension is not available` | Project running on shared plan | Upgrade to dedicated or open a support ticket |
| `duplicate key value violates unique constraint` | Running `database/init.sql` multiple times | The script uses `IF NOT EXISTS`—safe to re-run |
| `Function does not exist: auth.uid()` | Running outside Supabase | Replace `auth.uid()` with a session variable (e.g. `current_setting('request.jwt.claim.sub', true)`)

---

## 10. Next Steps

- Continue with the deployment options in [`docs/DEPLOYMENT.md`](./DEPLOYMENT.md)
- Follow the runtime checklist in [`docs/QUICK_START.md`](./QUICK_START.md)
- Keep the file inventory handy via [`docs/FILES_OVERVIEW.md`](./FILES_OVERVIEW.md)

With the database foundation ready, you can integrate the Supabase client helpers from `src/lib/supabaseClient.js` and launch CyberDevStudio confidently.
