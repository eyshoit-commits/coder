# Deployment Playbook

This document compiles the most common hosting strategies for CyberDevStudio. Each section assumes you have completed the database setup in [`docs/SUPABASE_POSTGRES_SETUP.md`](./SUPABASE_POSTGRES_SETUP.md) and populated your environment variables from `.env.local.example`.

---

## 1. Shared Prerequisites

- Node.js 18+ and pnpm/npm/yarn (for the Studio UI and server builds)
- Docker 24+ (for container-based options)
- Supabase project credentials & `DATABASE_URL`
- Access to the LLM server host (local machine or remote VM with `node-llama-cpp`)

Before deploying, build the applications locally:

```bash
pnpm install
pnpm run build --filter studio-ui
pnpm run build --filter api
```

Commit the optimized build artifacts before deploying to immutable targets (Netlify/Vercel).

---

## 2. Vercel (Studio UI)

1. Push your repository to GitHub/GitLab/Bitbucket.
2. Import the project in [Vercel](https://vercel.com/import) and select the `apps/studio-ui` directory as the root.
3. Set environment variables under **Settings → Environment Variables** using `.env.local` values (`NEXT_PUBLIC_SUPABASE_URL`, `NEXT_PUBLIC_SUPABASE_ANON_KEY`, etc.).
4. Configure build command: `pnpm install && pnpm run build --filter studio-ui` and output directory: `.vercel/output` or `apps/studio-ui/.next` depending on your setup.
5. Add a custom domain and enforce HTTPS.
6. Optional: create a [cron job](https://vercel.com/docs/cron-jobs) to refresh analytics via the `/api/cron/refresh-analytics` route (if implemented).

---

## 3. Netlify (Static marketing or docs)

1. Point Netlify to the repository and select the `docs/` directory or your marketing site root.
2. Build command: `pnpm run docs:build` (create this script if missing).
3. Publish directory: `docs/dist`.
4. Inject read-only Supabase credentials if your documentation references live data.
5. Enable [Netlify Identity](https://docs.netlify.com/visitor-access/identity/) only if you need gated documentation; main authentication remains with Supabase.

---

## 4. Docker & Docker Compose (Full stack)

The repository includes `docker/` manifests. To launch everything locally or on a VM:

```bash
docker compose -f docker/docker-compose.yml --env-file .env up -d --build
```

Key services:

| Service | Port | Description |
| ------- | ---- | ----------- |
| api | 6813 | JSON-RPC gateway, token accounting |
| llmserver | 6988 | `node-llama-cpp` wrapper with token throttling |
| studio-ui | 6711 | Web interface (Next.js / React) |
| auth | 6971 | Authentication microservice |
| db | 6472 | PostgresML database (externalized for Supabase in production) |

**Production tips**

- Replace the `db` service with Supabase by removing it from Compose and pointing `DATABASE_URL` to your managed instance.
- Add Traefik or Caddy in front of the stack for HTTPS termination and path routing.
- Configure persistent volumes (`models/`, `logs/`, `pgdata/`).

---

## 5. Heroku (API + Background workers)

1. Create two Heroku apps: `cyberdevstudio-api` and `cyberdevstudio-workers`.
2. Provision the **Heroku Postgres** add-on only for ephemeral staging; production should continue using Supabase via `DATABASE_URL`.
3. Define buildpacks:
   ```bash
   heroku buildpacks:add --app cyberdevstudio-api heroku/nodejs
   heroku buildpacks:add --app cyberdevstudio-workers heroku/nodejs
   ```
4. Push code using the `heroku` remote or the Container Registry.
5. Configure config vars copied from `.env` (never commit secrets).
6. Scale workers for scheduled refresh jobs (`heroku ps:scale cron=1:standard-1x`).

---

## 6. Bare-metal / VPS

1. Provision an Ubuntu 22.04 VM with at least 8 GB RAM for LLM inference.
2. Install Docker, docker-compose-plugin, Node.js (via `nvm`), and `pm2`.
3. Clone the repository and copy `.env`.
4. Start services:
   ```bash
   pnpm install --frozen-lockfile
   pnpm run build --filter api
   pm2 start apps/api/dist/main.js --name cyberdevstudio-api -- --port 6813
   pm2 start "node apps/llmserver/index.js" --name cyberdevstudio-llm
   pnpm --filter studio-ui start
   ```
5. Use Nginx as a reverse proxy and set up automatic renewals with Certbot.
6. Configure system monitoring (Prometheus Node Exporter, Grafana dashboards) to consume metrics emitted from the platform.

---

## 7. GitHub Pages (Docs only)

1. Build the documentation to a static directory (`docs/dist`).
2. Commit the artifacts or generate them in the CI workflow.
3. Push to the `gh-pages` branch or use GitHub Actions with the `peaceiris/actions-gh-pages` action.
4. Remember that GitHub Pages is static—do **not** expose Supabase service role secrets here.

---

## 8. Continuous Deployment Workflow

Here is a sample GitHub Actions workflow that coordinates the deployments:

```yaml
name: deploy

on:
  push:
    branches: [main]
  workflow_dispatch:

jobs:
  build:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: pnpm/action-setup@v3
        with:
          version: 8
      - run: pnpm install --frozen-lockfile
      - run: pnpm run lint && pnpm run test
      - run: pnpm run build --filter studio-ui --filter api
      - uses: actions/upload-artifact@v4
        with:
          name: web-build
          path: apps/studio-ui/.next

  deploy-vercel:
    needs: build
    runs-on: ubuntu-latest
    environment: production
    steps:
      - uses: actions/download-artifact@v4
        with:
          name: web-build
      - name: Deploy to Vercel
        uses: amondnet/vercel-action@v25
        with:
          vercel-token: ${{ secrets.VERCEL_TOKEN }}
          vercel-org-id: ${{ secrets.VERCEL_ORG_ID }}
          vercel-project-id: ${{ secrets.VERCEL_PROJECT_ID }}

  deploy-docker:
    needs: build
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: docker/login-action@v3
        with:
          registry: ghcr.io
          username: ${{ github.actor }}
          password: ${{ secrets.GITHUB_TOKEN }}
      - run: docker build -f docker/Dockerfile.api -t ghcr.io/org/cyberdevstudio-api:latest .
      - run: docker push ghcr.io/org/cyberdevstudio-api:latest
```

Extend the workflow with notifications, migrations (`supabase db push`), and health checks as needed.

---

## 9. Post-deployment Checklist

- [ ] Run database migrations (`supabase db push` or `pnpm run prisma:migrate` depending on your stack)
- [ ] Rotate API keys and service-role secrets regularly
- [ ] Validate CORS configuration on the API and Supabase Storage
- [ ] Ensure monitoring dashboards receive metrics from `telemetry.execution_metrics`
- [ ] Test LLM inference end-to-end (UI → API → LLM server → Postgres token log)

---

Continue with the fast onboarding steps in [`docs/QUICK_START.md`](./QUICK_START.md) to bring new team members online quickly.
