# Sys-admin guide: claiming an org with `repo-auth-ctl`

This is the operator runbook for onboarding a GitHub org to the AI gateway. We do
**not** offer self-serve onboarding and we do **not** distribute a binary
(ADR-0049): there are only a handful of first-party orgs, so a sys-admin claims
each one by hand with the `repo-auth-ctl` CLI, **built from this repo**.

## When you run this

An org admin installs the **`camer-digital-ai`** GitHub App on their org. That
fires a webhook → the service records a *Source* with `account_id = NULL`
(**unclaimed**). Until you claim it, the gateway denies that org's CI
(`/v1/resolve` → 403 `source_unclaimed`). Claiming links the Source to a billing
account + tier.

You also run `sources` any time you want to see who's installed and their state.

## One-time setup

You need:

- **`kubectl`** with access to the workload cluster (Hetzner `home-remote`):
  `export KUBECONFIG=/path/to/hetzner-k8s/kubeconfig` (the app runs in namespace
  `converse`).
- **Rust toolchain** (`cargo`) — the CLI is **built from source, not published**.

Build (or install onto your PATH) from a clone of this repo:

```bash
# one-off run:
cargo build --release -p repo-auth-ctl     # → target/release/repo-auth-ctl
# or install onto PATH:
cargo install --path app/repo-auth-ctl     # → ~/.cargo/bin/repo-auth-ctl
```

(macOS arm64, Linux x86_64 — both build natively; it's a pure-Rust HTTP client.)

## The runbook

The admin API is **ClusterIP-only** and guarded by a shared `X-Internal-Token`,
so you reach it over a port-forward, with the token pulled from the in-cluster
Secret. Run these from a shell with `KUBECONFIG` set to the Hetzner cluster:

```bash
# 1. port-forward the service (leave running in another shell / background it)
kubectl -n converse port-forward svc/lightbridge-repo-auth 3000:3000 &

# 2. load the internal token from the Secret
export RA__CTL__TOKEN=$(kubectl -n converse get secret lightbridge-repo-auth \
  -o jsonpath='{.data.internal-token}' | base64 -d)

# 3. see what's installed + claim status
repo-auth-ctl sources
# SOURCE_ID                      OWNER_ID    ACCOUNT_ID    PLAN  SCOPE  STATUS
# src-cz55gv0nljenr7bxw4lmd8at   139577169   <unclaimed>   free  all    active

# 4. claim the org → link it to a billing account + tier
repo-auth-ctl claim --owner-id 139577169 --account-id adorsys-gis --plan pro
```

That's it — the org's CI can now authenticate.

### Choosing the values

| Flag | What to put | Where it comes from |
|---|---|---|
| `--owner-id` | the org's **numeric** GitHub id | the `OWNER_ID` column of `sources` (it's the install's `account.id`) |
| `--account-id` | the billing-account reference | your billing system's account id for that org (stamped downstream as `x-account-id`) |
| `--plan` | `free` \| `pro` \| `service` \| `internal` | the tier they bill as (omit to leave the current value) |

### Repo scope

If the org installed with **"Only select repositories"**, only those repos can
authenticate (resolve denies others with `repo_not_in_scope`). Either have them
switch the install to **All repositories**, or add the repos on the
[installation settings](https://github.com/organizations/<org>/settings/installations);
GitHub then syncs the allowed set automatically. **All repositories** needs no
per-repo step.

### What to hand the org

One value — the **audience** for their workflows (the `SOURCE_ID` from `sources`):

```
https://api.ai.camer.digital/sources/<SOURCE_ID>
```

They set that as `audience` in the `@vymalo/opencode-oauth2` plugin config, with
`permissions: { id-token: write }` in the workflow. See the service repo
[`examples/`](../examples).

## Other operations

- **Re-claim / change tier / fix a typo** — just run `claim` again (it
  overwrites `account_id`, and `--plan` updates the tier).
- **Transfer to a different account** — `claim` with the new `--account-id`.
- **Block / revoke an org that's still installed** — cut its gateway access
  without waiting for an uninstall:
  ```bash
  repo-auth-ctl block   --owner-id 226188569   # resolve now denies (reason "blocked")
  repo-auth-ctl unblock --owner-id 226188569   # restore
  ```
  The block is a separate flag from `status`, so it **survives reinstalls /
  permission re-approvals** (which would otherwise reset status to active).
  `sources` shows such a Source with `STATUS = blocked`.
- **An org uninstalls** — the webhook flips the Source to `disabled` (and the
  reconcile sweep catches a missed one); resolve then denies. No action needed.

## Troubleshooting

| Symptom | Cause | Fix |
|---|---|---|
| `repo-auth-ctl` → `server returned an error` (401) | wrong / empty `RA__CTL__TOKEN` | re-pull the token (step 2); check the port-forward is up |
| `claim` → `no Source with that owner_id` | the org hasn't installed the App yet (or you have the wrong id) | run `sources`; confirm the install |
| org's CI → gateway 403 `source_unclaimed` | you haven't claimed it | run `claim` |
| org's CI → gateway 403 `repo_not_in_scope` | `selected` scope, repo not in the set | switch to All repositories or add the repo |
| connection refused | no port-forward | re-run step 1 |

## Why no self-serve / no distribution

See **ADR-0049** (ai-helm). Short version: ≤ ~5 first-party orgs, so a public
claim UI is needless attack surface and a published binary is needless supply
chain. A future self-serve dashboard (if we ever open up) would call the same
`/v1/admin/claim` endpoint after proving the claimer administers the install
(GitHub OAuth + `GET /user/installations`). Until then: this guide.
