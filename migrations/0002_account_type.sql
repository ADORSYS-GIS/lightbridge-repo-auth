-- account_type: "Organization" | "User" (from the install webhook's account.type).
-- Surfaced by repo-auth-ctl; backfilled for live installs by the reconcile sweep.
alter table identity_source add column if not exists account_type text;
