//! `lightbridge-repo-auth` — GitHub App webhook control plane + the `/v1/resolve`
//! endpoint the AI gateway's Authorino calls to bind a CI identity to an account.
//!
//! See `docs/` and ai-helm ADR-0047 for the trust model.

use std::sync::Arc;
use std::time::Duration;

use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::routing::{get, post};
use axum::{Json, Router};
use clap::Parser;
use serde_json::{json, Value};

use lightbridge_repo_auth_core::config::{
    Config, DatabaseConfig, GithubConfig, ResolveConfig, ServerConfig, SourceConfig,
};
use lightbridge_repo_auth_core::error::{Error, Result};
use lightbridge_repo_auth_core::github::GithubClient;
use lightbridge_repo_auth_core::model::{
    InstallationEvent, InstallationReposEvent, ResolveRequest, SourceStatus,
};
use lightbridge_repo_auth_core::store::Store;
use lightbridge_repo_auth_core::webhook::verify_signature;

#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

#[derive(Parser, Debug)]
#[command(name = "lightbridge-repo-auth", version, about)]
struct Args {
    #[arg(long, env = "RA__SERVER__BIND", default_value = "0.0.0.0:3000")]
    bind: String,
    #[arg(long, env = "RA__SERVER__TLS_CERT_PATH")]
    tls_cert_path: Option<String>,
    #[arg(long, env = "RA__SERVER__TLS_KEY_PATH")]
    tls_key_path: Option<String>,

    #[arg(long, env = "RA__DATABASE__URL")]
    database_url: String,
    #[arg(long, env = "RA__DATABASE__MAX_CONNECTIONS", default_value_t = 10)]
    db_max_connections: u32,

    #[arg(long, env = "RA__GITHUB__APP_ID")]
    github_app_id: u64,
    #[arg(long, env = "RA__GITHUB__WEBHOOK_SECRET")]
    github_webhook_secret: String,
    #[arg(long, env = "RA__GITHUB__PRIVATE_KEY_PATH")]
    github_private_key_path: String,
    #[arg(long, env = "RA__GITHUB__API_BASE", default_value = "https://api.github.com")]
    github_api_base: String,

    #[arg(long, env = "RA__RESOLVE__INTERNAL_TOKEN")]
    resolve_internal_token: String,

    #[arg(long, env = "RA__SOURCE__AUDIENCE_BASE")]
    source_audience_base: String,

    /// Reconcile sweep interval (seconds). 0 disables the sweep.
    #[arg(long, env = "RA__RECONCILE__INTERVAL_SECS", default_value_t = 900)]
    reconcile_interval_secs: u64,
}

#[derive(Clone)]
struct AppState {
    store: Store,
    github: Arc<GithubClient>,
    config: Arc<Config>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let _ = dotenvy::dotenv();
    let args = Args::parse();
    init_tracing();
    rustls::crypto::ring::default_provider()
        .install_default()
        .ok();

    let private_key_pem = std::fs::read_to_string(&args.github_private_key_path)
        .map_err(|e| anyhow::anyhow!("reading GitHub App private key {}: {e}", args.github_private_key_path))?;

    let config = Arc::new(Config {
        server: ServerConfig {
            bind: args.bind.clone(),
            tls_cert_path: args.tls_cert_path.clone(),
            tls_key_path: args.tls_key_path.clone(),
        },
        database: DatabaseConfig {
            url: args.database_url.clone(),
            max_connections: args.db_max_connections,
        },
        github: GithubConfig {
            app_id: args.github_app_id,
            webhook_secret: args.github_webhook_secret.clone(),
            private_key_pem: private_key_pem.clone(),
            api_base: args.github_api_base.clone(),
        },
        resolve: ResolveConfig {
            internal_token: args.resolve_internal_token.clone(),
        },
        source: SourceConfig {
            audience_base: args.source_audience_base.clone(),
        },
    });

    let store = Store::connect(&config.database).await?;
    store.migrate().await?;
    let github = Arc::new(GithubClient::new(
        config.github.api_base.clone(),
        config.github.app_id,
        &config.github.private_key_pem,
    )?);

    let state = AppState {
        store: store.clone(),
        github: github.clone(),
        config: config.clone(),
    };

    if args.reconcile_interval_secs > 0 {
        spawn_reconcile(state.clone(), args.reconcile_interval_secs);
    }

    let app = Router::new()
        .route("/github/webhooks", post(webhook))
        .route("/v1/resolve", post(resolve))
        .route("/health", get(health))
        .route("/health/ready", get(ready))
        .route("/health/startup", get(health))
        .with_state(state);

    let addr: std::net::SocketAddr = config.server.bind.parse()?;
    tracing::info!(%addr, "lightbridge-repo-auth listening");

    match (&config.server.tls_cert_path, &config.server.tls_key_path) {
        (Some(cert), Some(key)) => {
            let tls = axum_server::tls_rustls::RustlsConfig::from_pem_file(cert, key).await?;
            axum_server::bind_rustls(addr, tls)
                .serve(app.into_make_service())
                .await?;
        }
        _ => {
            let listener = tokio::net::TcpListener::bind(addr).await?;
            axum::serve(listener, app)
                .with_graceful_shutdown(shutdown_signal())
                .await?;
        }
    }
    Ok(())
}

fn init_tracing() {
    use tracing_subscriber::{fmt, EnvFilter};
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    fmt().with_env_filter(filter).json().init();
}

async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c().await.ok();
    };
    #[cfg(unix)]
    let term = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut s) => {
                s.recv().await;
            }
            Err(_) => std::future::pending::<()>().await,
        }
    };
    #[cfg(not(unix))]
    let term = std::future::pending::<()>();
    tokio::select! {
        _ = ctrl_c => {},
        _ = term => {},
    }
    tracing::info!("shutdown signal received");
}

// ─────────────────────────── webhook ───────────────────────────

async fn webhook(State(st): State<AppState>, headers: HeaderMap, body: Bytes) -> Result<StatusCode> {
    let sig = headers.get("x-hub-signature-256").and_then(|v| v.to_str().ok());
    verify_signature(&st.config.github.webhook_secret, &body, sig)?;

    let event = headers
        .get("x-github-event")
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_string();

    match event.as_str() {
        "installation" => handle_installation(&st, &body).await?,
        "installation_repositories" => handle_installation_repos(&st, &body).await?,
        "ping" => tracing::info!("github ping"),
        other => tracing::debug!(event = other, "ignored webhook event"),
    }
    Ok(StatusCode::NO_CONTENT)
}

async fn handle_installation(st: &AppState, body: &[u8]) -> Result<()> {
    let ev: InstallationEvent = serde_json::from_slice(body)?;
    let owner_id = ev.installation.account.id.to_string();
    let scope = ev.installation.repository_selection.as_deref().unwrap_or("all");
    tracing::info!(action = %ev.action, owner_id = %owner_id, "installation event");

    match ev.action.as_str() {
        "created" | "unsuspend" | "new_permissions_accepted" => {
            let src = st
                .store
                .upsert_installation(
                    ev.installation.id,
                    &owner_id,
                    ev.installation.account.login.as_deref(),
                    scope,
                )
                .await?;
            if scope == "selected" && !ev.repositories.is_empty() {
                st.store.sync_allowed_repos(ev.installation.id, &ev.repositories).await?;
            }
            tracing::info!(source_id = %src.id, audience = %st.config.source.audience_for(&src.id), "source upserted");
        }
        "deleted" => st.store.set_status_by_installation(ev.installation.id, SourceStatus::Disabled).await?,
        "suspend" => st.store.set_status_by_installation(ev.installation.id, SourceStatus::Suspended).await?,
        other => tracing::debug!(action = other, "unhandled installation action"),
    }
    Ok(())
}

async fn handle_installation_repos(st: &AppState, body: &[u8]) -> Result<()> {
    let ev: InstallationReposEvent = serde_json::from_slice(body)?;
    tracing::info!(action = %ev.action, installation_id = ev.installation.id, "installation_repositories event");
    // Webhooks can be lossy; re-list from GitHub for an authoritative set.
    let repos = st.github.list_installation_repos(ev.installation.id).await?;
    st.store.sync_allowed_repos(ev.installation.id, &repos).await?;
    Ok(())
}

// ─────────────────────────── resolve (data plane) ───────────────────────────

async fn resolve(
    State(st): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<ResolveRequest>,
) -> Result<(StatusCode, Json<Value>)> {
    // Only Authorino may call this; the body's claims are trusted on that basis.
    let presented = headers.get("x-internal-token").and_then(|v| v.to_str().ok());
    if presented != Some(st.config.resolve.internal_token.as_str()) {
        return Err(Error::Unauthorized);
    }

    let expected_source_id = source_id_from_audience(&req.audience)
        .ok_or_else(|| Error::BadRequest("audience not a /sources/<id> url".into()))?;

    let outcome = st.store.resolve(&req, &expected_source_id).await?;
    // 403 on deny so Authorino's (non-optional) metadata fetch fails closed even
    // if its authorization step is ever misconfigured; 200 + `allowed:true` on
    // success carries the account/plan the response headers read. A 5xx (DB
    // down) propagates via `?` and also fails closed.
    let code = if outcome.allowed {
        StatusCode::OK
    } else {
        tracing::debug!(owner_id = %req.repository_owner_id, reason = ?outcome.reason, "resolve denied");
        StatusCode::FORBIDDEN
    };
    Ok((code, Json(json!(outcome))))
}

/// `https://api.vymalo.com/sources/src-7f3a8b` → `src-7f3a8b`.
fn source_id_from_audience(aud: &str) -> Option<String> {
    aud.rsplit_once("/sources/")
        .map(|(_, id)| id.trim_end_matches('/').to_string())
        .filter(|id| !id.is_empty())
}

// ─────────────────────────── reconcile sweep ───────────────────────────

fn spawn_reconcile(st: AppState, interval_secs: u64) {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs(interval_secs));
        loop {
            tick.tick().await;
            if let Err(e) = reconcile_once(&st).await {
                tracing::warn!(error = %e, "reconcile sweep failed (will retry)");
            }
        }
    });
}

/// Compare our active installs against GitHub's truth and disable ghosts whose
/// `installation.deleted` webhook we missed. Cheap insurance against webhook loss.
async fn reconcile_once(st: &AppState) -> Result<()> {
    let live: std::collections::HashSet<i64> = st
        .github
        .list_installations()
        .await?
        .into_iter()
        .map(|i| i.id)
        .collect();
    let mut disabled = 0u32;
    for id in st.store.active_installation_ids().await? {
        if !live.contains(&id) {
            st.store.set_status_by_installation(id, SourceStatus::Disabled).await?;
            disabled += 1;
        }
    }
    tracing::info!(live = live.len(), disabled, "reconcile sweep complete");
    Ok(())
}

// ─────────────────────────── health ───────────────────────────

async fn health() -> StatusCode {
    StatusCode::OK
}

async fn ready(State(st): State<AppState>) -> StatusCode {
    match st.store.active_installation_ids().await {
        Ok(_) => StatusCode::OK,
        Err(_) => StatusCode::SERVICE_UNAVAILABLE,
    }
}
