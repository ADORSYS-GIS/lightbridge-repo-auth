-- Source store for the GitHub-OIDC → AI-gateway binding.
-- One row per bound GitHub org; the data plane resolves against it per request.

create table if not exists identity_source (
    id                  text primary key,                 -- src-<cuid2>
    account_id          text,                             -- external billing ref; null until claimed
    repository_owner_id text not null unique,             -- the binding (GitHub numeric id, immutable)
    installation_id     bigint not null,
    repo_scope          text not null default 'all',      -- all | selected
    billing_plan        text not null default 'free',     -- free | pro | service
    status              text not null default 'active',   -- active | disabled | suspended
    account_login       text,
    created_at          timestamptz not null default now(),
    updated_at          timestamptz not null default now()
);

create unique index if not exists identity_source_installation_id_idx
    on identity_source (installation_id);

-- Only populated for `selected` repo scope; webhook-synced, never hand-curated.
create table if not exists allowed_repo (
    source_id     text not null references identity_source (id) on delete cascade,
    repository_id bigint not null,
    full_name     text,
    primary key (source_id, repository_id)
);
