//! Postgres-backed Source store. The control plane writes here on webhooks; the
//! data plane (`/v1/resolve`) reads here on every CI request.

use sqlx::postgres::{PgConnectOptions, PgPoolOptions, PgSslMode};
use sqlx::PgPool;

use crate::config::DatabaseConfig;
use crate::error::{Error, Result};
use crate::model::{IdentitySource, Repo, ResolveRequest, ResolveResponse, SourceStatus};

fn parse_sslmode(s: &str) -> PgSslMode {
    match s.to_ascii_lowercase().as_str() {
        "disable" => PgSslMode::Disable,
        "allow" => PgSslMode::Allow,
        "require" => PgSslMode::Require,
        "verify-ca" => PgSslMode::VerifyCa,
        "verify-full" => PgSslMode::VerifyFull,
        _ => PgSslMode::Prefer,
    }
}

#[derive(Clone)]
pub struct Store {
    pool: PgPool,
}

impl Store {
    pub async fn connect(cfg: &DatabaseConfig) -> Result<Self> {
        let opts = PgPoolOptions::new().max_connections(cfg.max_connections);
        let pool = match &cfg.url {
            Some(url) => opts.connect(url).await?,
            // Build from parts — lets the CNPG role-Secret password be passed
            // verbatim (no URL-encoding of special characters).
            None => {
                let missing = |f: &str| Error::BadRequest(format!("database.{f} required when database.url is unset"));
                let conn = PgConnectOptions::new()
                    .host(cfg.host.as_deref().ok_or_else(|| missing("host"))?)
                    .port(cfg.port)
                    .username(cfg.user.as_deref().ok_or_else(|| missing("user"))?)
                    .password(cfg.password.as_deref().unwrap_or_default())
                    .database(cfg.name.as_deref().ok_or_else(|| missing("name"))?)
                    .ssl_mode(parse_sslmode(&cfg.sslmode));
                opts.connect_with(conn).await?
            }
        };
        Ok(Self { pool })
    }

    /// Apply migrations embedded from `./migrations`.
    pub async fn migrate(&self) -> Result<()> {
        sqlx::migrate!("../../migrations")
            .run(&self.pool)
            .await
            .map_err(|e| crate::error::Error::Db(e.into()))?;
        Ok(())
    }

    /// Upsert the binding on `installation.created` (and `new_permissions_*`).
    /// Keyed on `repository_owner_id` (one Source per org). A reinstall reuses
    /// the row and just refreshes `installation_id` — and crucially does NOT
    /// clobber an already-`account_id` (the dashboard's claim survives a
    /// reinstall), nor flip status if it was disabled→active intentionally.
    #[allow(clippy::too_many_arguments)]
    pub async fn upsert_installation(
        &self,
        installation_id: i64,
        repository_owner_id: &str,
        account_login: Option<&str>,
        account_type: Option<&str>,
        repo_scope: &str,
    ) -> Result<IdentitySource> {
        let id = format!("src-{}", cuid::cuid2());
        let row = sqlx::query_as::<_, IdentitySource>(
            r#"
            insert into identity_source
                (id, repository_owner_id, installation_id, repo_scope, account_login, account_type, status)
            values ($1, $2, $3, $4, $5, $6, 'active')
            on conflict (repository_owner_id) do update set
                installation_id = excluded.installation_id,
                repo_scope      = excluded.repo_scope,
                account_login   = coalesce(excluded.account_login, identity_source.account_login),
                account_type    = coalesce(excluded.account_type, identity_source.account_type),
                status          = 'active',
                updated_at      = now()
            returning *
            "#,
        )
        .bind(&id)
        .bind(repository_owner_id)
        .bind(installation_id)
        .bind(repo_scope)
        .bind(account_login)
        .bind(account_type)
        .fetch_one(&self.pool)
        .await?;
        Ok(row)
    }

    /// Refresh just the account name/type for a live installation (the reconcile
    /// sweep uses this to backfill rows installed before account_type existed).
    /// Does NOT touch status/scope/account_id.
    pub async fn refresh_account_info(
        &self,
        installation_id: i64,
        account_login: Option<&str>,
        account_type: Option<&str>,
    ) -> Result<()> {
        sqlx::query(
            r#"update identity_source set
                 account_login = coalesce($2, account_login),
                 account_type  = coalesce($3, account_type),
                 updated_at    = now()
               where installation_id = $1"#,
        )
        .bind(installation_id)
        .bind(account_login)
        .bind(account_type)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Flip status by installation id (deleted → disabled, suspend → suspended).
    pub async fn set_status_by_installation(
        &self,
        installation_id: i64,
        status: SourceStatus,
    ) -> Result<()> {
        sqlx::query(
            "update identity_source set status = $1, updated_at = now() where installation_id = $2",
        )
        .bind(status.as_str())
        .bind(installation_id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Replace the allowed-repo set for a `selected`-scope install.
    pub async fn sync_allowed_repos(&self, installation_id: i64, repos: &[Repo]) -> Result<()> {
        let mut tx = self.pool.begin().await?;
        let source_id: Option<String> =
            sqlx::query_scalar("select id from identity_source where installation_id = $1")
                .bind(installation_id)
                .fetch_optional(&mut *tx)
                .await?;
        let Some(source_id) = source_id else {
            tx.rollback().await?;
            return Ok(());
        };
        sqlx::query("delete from allowed_repo where source_id = $1")
            .bind(&source_id)
            .execute(&mut *tx)
            .await?;
        for r in repos {
            sqlx::query(
                "insert into allowed_repo (source_id, repository_id, full_name) values ($1,$2,$3)
                 on conflict do nothing",
            )
            .bind(&source_id)
            .bind(r.id)
            .bind(r.full_name.as_deref())
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        Ok(())
    }

    /// All installation ids currently believed active — for the reconcile sweep
    /// to compare against GitHub's truth and disable ghosts.
    pub async fn active_installation_ids(&self) -> Result<Vec<i64>> {
        let ids =
            sqlx::query_scalar("select installation_id from identity_source where status = 'active'")
                .fetch_all(&self.pool)
                .await?;
        Ok(ids)
    }

    /// The data-plane lookup. Returns an `allowed` verdict with the account +
    /// tier when the (immutable) `repository_owner_id` is bound to a claimed,
    /// active Source — and, for `selected` scope, the repo is in the synced set.
    pub async fn resolve(&self, req: &ResolveRequest, expected_source_id: &str) -> Result<ResolveResponse> {
        let Some(src) = self.find_by_owner(&req.repository_owner_id).await? else {
            return Ok(ResolveResponse::deny("owner_not_bound"));
        };
        if src.blocked {
            return Ok(ResolveResponse::deny("blocked"));
        }
        if src.status != "active" {
            return Ok(ResolveResponse::deny("source_inactive"));
        }
        let Some(account_id) = src.account_id.clone() else {
            return Ok(ResolveResponse::deny("source_unclaimed"));
        };
        // The audience selects which Source the customer *intended*. If it names
        // a different Source than the one their owner_id is bound to, they used
        // the wrong audience — fail closed rather than silently authorize.
        if expected_source_id != src.id {
            return Ok(ResolveResponse::deny("audience_source_mismatch"));
        }
        if src.repo_scope == "selected" {
            match &req.repository_id {
                Some(repo_id) if self.repo_allowed(&src.id, repo_id).await? => {}
                _ => return Ok(ResolveResponse::deny("repo_not_in_scope")),
            }
        }
        Ok(ResolveResponse {
            allowed: true,
            reason: None,
            account_id: Some(account_id),
            source_id: Some(src.id),
            billing_plan: Some(src.billing_plan),
            repository: req.repository.clone(),
        })
    }

    /// Link a Source to a billing account (the "claim" step). Selected by the
    /// owner-id binding key; `billing_plan=None` leaves the current value.
    /// Returns the updated row, or None if no Source has that owner.
    pub async fn claim(
        &self,
        owner_id: &str,
        account_id: &str,
        billing_plan: Option<&str>,
    ) -> Result<Option<IdentitySource>> {
        let row = sqlx::query_as::<_, IdentitySource>(
            r#"
            update identity_source
               set account_id   = $2,
                   billing_plan = coalesce($3, billing_plan),
                   updated_at   = now()
             where repository_owner_id = $1
            returning *
            "#,
        )
        .bind(owner_id)
        .bind(account_id)
        .bind(billing_plan)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row)
    }

    /// Toggle the operator block on a Source (by owner id). Returns the updated
    /// row, or None if no Source has that owner. Independent of webhook `status`,
    /// so it survives reinstalls. Returns None if no Source matches.
    pub async fn set_blocked(&self, owner_id: &str, blocked: bool) -> Result<Option<IdentitySource>> {
        let row = sqlx::query_as::<_, IdentitySource>(
            "update identity_source set blocked = $2, updated_at = now() \
             where repository_owner_id = $1 returning *",
        )
        .bind(owner_id)
        .bind(blocked)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row)
    }

    /// All Sources (admin visibility).
    pub async fn list_sources(&self) -> Result<Vec<IdentitySource>> {
        let rows = sqlx::query_as::<_, IdentitySource>(
            "select * from identity_source order by created_at",
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    pub async fn find_by_owner(&self, owner_id: &str) -> Result<Option<IdentitySource>> {
        let row = sqlx::query_as::<_, IdentitySource>(
            "select * from identity_source where repository_owner_id = $1",
        )
        .bind(owner_id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row)
    }

    async fn repo_allowed(&self, source_id: &str, repository_id: &str) -> Result<bool> {
        // repository_id arrives as text from the JWT claim; compare as bigint.
        let Ok(rid) = repository_id.parse::<i64>() else {
            return Ok(false);
        };
        let exists: Option<i64> = sqlx::query_scalar(
            "select repository_id from allowed_repo where source_id = $1 and repository_id = $2",
        )
        .bind(source_id)
        .bind(rid)
        .fetch_optional(&self.pool)
        .await?;
        Ok(exists.is_some())
    }
}
