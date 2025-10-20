# Files Overview

The Supabase + PostgreSQL configuration introduces seven primary artifacts. Use this document as an index when navigating the repository.

| File | Purpose | Key Highlights |
| ---- | ------- | -------------- |
| [`database/init.sql`](../database/init.sql) | Bootstraps database schemas (`platform`, `telemetry`, `analytics`) and installs PostgresML extensions | Adds core tables, RLS policies, RPC helpers (`record_token_usage`, `grant_user_tokens`, `get_usage_overview`, `set_default_model`) |
| [`.env.local.example`](../.env.local.example) | Environment template for local and production deployments | Contains placeholders for Supabase keys, Postgres connection, runtime URLs, and observability endpoints |
| [`docs/SUPABASE_POSTGRES_SETUP.md`](./SUPABASE_POSTGRES_SETUP.md) | Step-by-step data layer setup | Covers Supabase project creation, SQL import, PostgresML activation, auth sync, and maintenance |
| [`docs/DEPLOYMENT.md`](./DEPLOYMENT.md) | Hosting strategies reference | Details Vercel, Netlify, Docker Compose, Heroku, VPS, GitHub Pages, and CI/CD workflow |
| [`docs/QUICK_START.md`](./QUICK_START.md) | Five-minute onboarding checklist | Guides new contributors through env setup, SQL execution, local services, and validation |
| [`src/lib/supabaseClient.js`](../src/lib/supabaseClient.js) | Supabase client wrapper with rich helper set | Provides factory functions, typed RPC helpers, admin utilities, and telemetry logging |
| `docs/FILES_OVERVIEW.md` | You are here | Summarizes the created assets |

## Suggested Reading Order

1. [`docs/QUICK_START.md`](./QUICK_START.md) – fastest way to get running
2. [`docs/SUPABASE_POSTGRES_SETUP.md`](./SUPABASE_POSTGRES_SETUP.md) – deep dive into database steps
3. [`docs/DEPLOYMENT.md`](./DEPLOYMENT.md) – choose your hosting strategy
4. [`src/lib/supabaseClient.js`](../src/lib/supabaseClient.js) – integrate the client utilities into your codebase

## Keeping the Index Updated

- Add a new row whenever you introduce a supporting asset (migration, diagram, script).
- Keep descriptions concise (≤ 120 characters) but actionable.
- Cross-link related files to improve discoverability.

For questions or improvements, open an issue in the repository and tag the **Platform** team.
