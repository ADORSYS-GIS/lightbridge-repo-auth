# GitHub App permissions (minimal: auth + reviewer posting)

> **TL;DR — grant `Repository → Metadata: Read-only` + `Issues: Read & write` +
> `Pull requests: Read & write`, and subscribe to the `installation` /
> `installation_repositories` events. NO `Contents` (no code write).** Metadata
> is the auth/binding floor; Issues+PR write let the App post AI code reviews as
> `camer-digital-ai[bot]` (ai-helm **ADR-0050**, which amends the original
> auth-only stance of ADR-0047). This matches
> [`github-app-manifest.json`](../github-app-manifest.json), the source of truth.

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

- **Repository permissions:**
  - **Metadata: `Read-only`** — the auth/binding floor; covers `/installation/repositories`.
  - **Issues: `Read & write`** + **Pull requests: `Read & write`** — so the App
    posts AI reviews/comments as `camer-digital-ai[bot]` (ADR-0050). ⚠️ Adding
    these requires every existing install to **re-approve** (GitHub fires
    `installation` → `new_permissions_accepted`).
  - **Contents: none** — the App still never reads or writes code; CI git
    operations use the workflow's own `GITHUB_TOKEN`, not the App.
- **Subscribe to events:** `Installation`, `Installation repositories`.
- **Account / Organization permissions:** none.
- **Webhook:** active, URL `https://repo-auth.ai.camer.digital/github/webhooks`,
  with the shared secret (`repo_auth_github_webhook_secret`) — HMAC-verified.

## Why Issues/PR write — but still NOT Contents

This App installs across **every org**, so its blast radius is its permissions ×
every install. ADR-0050 made a deliberate trade: we WANT reviews authored by a
single branded `camer-digital-ai[bot]`, which needs `Issues` + `Pull requests`
write — so a key compromise could spam issue/PR comments across installs
(annoying, recoverable). We still **refuse `Contents`** — code write is the
catastrophic, supply-chain scope, and nothing here needs it:

- **Posting reviews** → the App installation token (Issues/PR write) → bot author.
- **CI git push** (clone/commit/push) → the workflow's own `GITHUB_TOKEN`
  (per-repo, ephemeral), NOT the App. That's why the App needs no `Contents`.
- **A future agent that must push code as the bot** → reconsider in a new ADR
  before adding `Contents: write`; don't add it speculatively.

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
