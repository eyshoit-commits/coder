# CyberDevStudio Quick Start (≤ 5 minutes)

This checklist brings a new developer from zero to a functional CyberDevStudio environment backed by Supabase and PostgresML.

---

## 0. Clone & Install

```bash
git clone <repo-url>
cd cyberdevstudio
pnpm install
```

---

## 1. Configure Environment

1. Copy the template and fill in Supabase credentials:
   ```bash
   cp .env.local.example .env.local
   cp .env.local .env
   ```
2. Update the following keys at minimum:
   - `NEXT_PUBLIC_SUPABASE_URL`
   - `NEXT_PUBLIC_SUPABASE_ANON_KEY`
   - `SUPABASE_SERVICE_ROLE_KEY`
   - `DATABASE_URL`

---

## 2. Bootstrap the Database

1. Open the Supabase SQL editor.
2. Paste [`database/init.sql`](../database/init.sql) and run it.
3. Verify tables in the `platform`, `telemetry`, and `analytics` schemas appear.

> Optional: run the trigger snippet from [`docs/SUPABASE_POSTGRES_SETUP.md`](./SUPABASE_POSTGRES_SETUP.md#5-configure-authentication) to mirror Supabase Auth users automatically.

---

## 3. Launch Local Services

```bash
# 1. Start the Supabase local stack (optional but recommended)
supabase start

# 2. Start the API + LLM server + Studio UI
docker compose -f docker/docker-compose.yml --env-file .env up -d api llmserver
pnpm --filter studio-ui dev
```

- API available at `http://localhost:6813`
- Studio UI available at `http://localhost:3000` (or the port defined by Next.js)
- LLM server at `http://localhost:6988`

---

## 4. Validate the Setup

1. **Authentication** – Sign up using the Studio UI and ensure the new user record exists in `platform.users`.
2. **Token accounting** – Trigger a completion and inspect `platform.token_usage` for the logged event.
3. **PostgresML** – Run:
   ```sql
   SELECT postgresml.complete('deepseek-coder-1.3b', 'Say hello to CyberDevStudio');
   ```
4. **Telemetry** – Check `telemetry.agent_events` for sandbox activity.

---

## 5. Next Steps

- Follow deployment strategies in [`docs/DEPLOYMENT.md`](./DEPLOYMENT.md)
- Review Supabase best practices in [`docs/SUPABASE_POSTGRES_SETUP.md`](./SUPABASE_POSTGRES_SETUP.md)
- Keep the file index from [`docs/FILES_OVERVIEW.md`](./FILES_OVERVIEW.md) handy when onboarding teammates

You are ready to build! 🚀
