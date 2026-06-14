-- Operator block flag — separate from the webhook-driven `status` so it survives
-- reinstalls / new_permissions_accepted (which reset status to active). resolve()
-- denies a blocked Source regardless of status.
alter table identity_source add column if not exists blocked boolean not null default false;
