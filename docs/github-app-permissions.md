# GitHub App permissions (minimal, auth-only)

> **TL;DR — grant only `Repository → Metadata: Read-only` + the `installation`
> and `installation_repositories` events. Nothing else.** No Contents, no Issues,
> no write of any kind. This matches [`github-app-manifest.json`](../github-app-manifest.json),
> which is the source of truth.

## Principle

The `camer-digital-ai` GitHub App is the **control plane** of the OIDC binding
model (ai-helm ADR-0047): its whole job is recording *which org is bound to which
account*, via install webhooks + a reconcile sweep. **It never touches a CI
request and never reads or writes code.** Coding in runners is done by the
workflow's own `GITHUB_TOKEN` (per-repo, ephemeral, scoped by `permissions:`),
not by this App.

So the App's permission set should be the smallest that lets it (a) receive
install events and (b) list an installation's repos for `selected`-scope sync.
That is `Metadata: Read` — which is also GitHub's mandatory floor for any App.

## Exactly what the service calls (and why metadata is enough)

Every GitHub API call the service makes lives in
[`crates/lightbridge-repo-auth-core/src/github.rs`](../crates/lightbridge-repo-auth-core/src/github.rs):

| Call | Endpoint | Auth | Permission |
|---|---|---|---|
| list installations (reconcile sweep) | `GET /app/installations` | App JWT | none — App-level |
| mint installation token | `POST /app/installations/{id}/access_tokens` | App JWT | none |
| list installation repos (`selected` sync) | `GET /installation/repositories` | installation token | **Metadata: Read** |

No endpoint reads file contents, issues, pull requests, or anything writable.

## Required settings

- **Repository permissions → Metadata: `Read-only`.** (The only one. GitHub
  auto-requires it; it covers `/installation/repositories`.)
- **Subscribe to events:** `Installation`, `Installation repositories`.
- **Account / Organization permissions:** none.
- **Webhook:** active, URL `https://repo-auth.ai.camer.digital/github/webhooks`,
  with the shared secret (`repo_auth_github_webhook_secret`) — HMAC-verified.

## Why NOT Contents / Issues / any write

This App installs across **every customer org**, so its blast radius is whatever
its permissions allow, multiplied by every install. With `Metadata: Read` only,
the worst case of a private-key compromise is "an attacker learns which orgs are
customers." Add `Contents: RW` / `Issues: RW` and the worst case becomes **write
access to every customer's code and issues** — a catastrophic, multi-tenant
escalation for permissions the service never even uses.

- **Coding in runners** → the default `GITHUB_TOKEN` (set `permissions:` in the
  workflow). Per-repo, ephemeral, never leaves the run.
- **A branded bot that pushes across repos** (if ever wanted) → a *separate* App
  with those scopes. Do not fold write permissions into the auth App.

## Operational note

**Reducing** an App's permissions takes effect immediately and needs **no
admin re-approval**. **Adding** a permission requires each org admin to accept it
(GitHub fires `installation` → `new_permissions_accepted`, which the service
treats as an upsert). So paring this App back from Contents/Issues RW to
Metadata-only is safe and instant.

## Source of truth

[`github-app-manifest.json`](../github-app-manifest.json) — the manifest used to
register the App declares exactly `default_permissions: { metadata: read }` +
`default_events: [installation, installation_repositories]`. Keep the live App in
sync with it; if they diverge, the manifest wins.
