# lightbridge-repo-auth

The **control plane** for binding a GitHub organization to a Vymalo/Lightbridge
billing account, so CI runners can use AI coding through the gateway with nothing
but a standard GitHub Actions OIDC token.

> **Mental model.** The GitHub App is the *control plane* (rare events: install /
> uninstall / repo-scope changes). The GitHub Actions OIDC token is the *data
> plane* (every CI run). This service owns the **binding** the data plane checks
> against — it never mints tokens and never validates the runtime JWT signature.
> That is Authorino's job (issuer `https://token.actions.githubusercontent.com`).
>
> Full trust model + threat analysis: [`docs/auth-model.md`](docs/auth-model.md).
> Gateway-side enforcement: ai-helm **ADR-0047**.

## What it does

| Surface | Path | Caller | Purpose |
|---|---|---|---|
| Webhook | `POST /github/webhooks` | GitHub | `installation.*` / `installation_repositories.*` → write the Source store |
| Resolve | `POST /v1/resolve` | Authorino (in-cluster) | (owner_id, audience, repo_id) → `{allowed, account_id, billing_plan}` |
| Health | `GET /health`, `/health/ready`, `/health/startup` | k8s probes | liveness / readiness |

Reconcile sweep (default every 15 min) lists installations from GitHub and
disables any local Source whose `installation.deleted` webhook was missed.

## The binding (why it's secure)

A Source row is keyed on `repository_owner_id` — GitHub's **numeric, server-set,
immutable** org id, captured from the webhook payload (never a user-typed form).
At request time Authorino validates the GitHub OIDC JWT, then calls `/v1/resolve`;
this service confirms the token's `repository_owner_id` matches a **claimed,
active** Source (and, for `selected` scope, that `repository_id` is in the synced
set). An attacker's own org resolves to *their* account; a forged token fails JWT
validation upstream; a fork carries the fork-owner's id. See `docs/auth-model.md`.

## Architecture

```
crates/lightbridge-repo-auth-core   # domain: model, store (sqlx), github client, webhook verify
app/lightbridge-repo-auth           # axum binary: router, handlers, reconcile loop
migrations/                         # sqlx migrations (identity_source, allowed_repo)
charts/lightbridge-repo-auth/       # Helm chart (Deployment + Service + ExternalSecret)
.docker/Dockerfile                  # multi-stage → distroless
github-app-manifest.json            # the GitHub App definition (auth-only permissions)
```

Stack mirrors `lightbridge-authz`: axum 0.8 + axum-server (rustls), sqlx 0.8,
jsonwebtoken, clap(env) + dotenvy, tracing(JSON), cuid2, mimalloc.

## Local dev

```bash
cp .env.example .env          # fill in DATABASE_URL, webhook secret, etc.
# a Postgres + the GitHub App private key PEM at config/app-private-key.pem
cargo run -p lightbridge-repo-auth
cargo test --workspace
cargo clippy --workspace
```

Config is entirely env-driven (prefix `RA__`); see `.env.example` and `--help`.

## Registering the GitHub App (manifest flow)

1. Serve an auto-POSTing form with `github-app-manifest.json` as the `manifest`
   field to `https://github.com/settings/apps/new?state=<csrf>` (or the org form).
2. GitHub redirects to `redirect_url` with `?code=...`.
3. `POST https://api.github.com/app-manifests/<code>/conversions` → returns `id`,
   `pem`, `webhook_secret`, `client_id`, `client_secret`. Store them once (the
   `pem`/`webhook_secret` go to your secret manager; this service reads them via
   ESO). The `code` is single-use, ~1h TTL.

## Deploy

The Helm chart is consumed by ai-helm (an `Application` in `charts/apps`); secrets
come from ESO (`ClusterSecretStore ssegning-aws`). The `/v1/resolve` Service is
ClusterIP-only and additionally gated by the `X-Internal-Token` shared secret —
only Authorino should reach it.
