//! `repo-auth-ctl` — tiny operator CLI for lightbridge-repo-auth.
//!
//! The admin API (`/v1/admin/*`) is ClusterIP-only + X-Internal-Token-guarded —
//! there is intentionally no public self-serve claim (ADR-0049: we onboard a
//! handful of first-party orgs by hand, not arbitrary customers). This is the
//! operator's front door to it.
//!
//! Usage (port-forward the in-cluster service first):
//!   kubectl -n converse port-forward svc/lightbridge-repo-auth 3000:3000 &
//!   export RA__CTL__TOKEN=$(kubectl -n converse get secret lightbridge-repo-auth \
//!     -o jsonpath='{.data.internal-token}' | base64 -d)
//!   repo-auth-ctl sources
//!   repo-auth-ctl claim --owner-id 139577169 --account-id adorsys-gis --plan pro

use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};
use lightbridge_repo_auth_core::model::ClaimRequest;
use reqwest::Client;
use serde_json::Value;

#[derive(Parser)]
#[command(name = "repo-auth-ctl", version, about = "Operator CLI for lightbridge-repo-auth (claim/list Sources).")]
struct Cli {
    /// Base URL of the repo-auth service. Port-forward locally — it is ClusterIP-only.
    #[arg(long, env = "RA__CTL__BASE_URL", default_value = "http://localhost:3000", global = true)]
    base_url: String,
    /// The X-Internal-Token (same value as the service's RA__RESOLVE__INTERNAL_TOKEN).
    #[arg(long, env = "RA__CTL__TOKEN", global = true)]
    token: Option<String>,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// List all Sources (id, owner, account, plan, scope, status).
    Sources,
    /// Link a Source to a billing account (the "claim" step), selected by owner id.
    Claim {
        /// GitHub numeric org/user id (the binding key; from the install webhook).
        #[arg(long)]
        owner_id: String,
        /// Billing-account reference to stamp as x-account-id.
        #[arg(long)]
        account_id: String,
        /// Tier: free | pro | service | internal (omit to leave unchanged).
        #[arg(long)]
        plan: Option<String>,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let token = cli
        .token
        .as_deref()
        .filter(|t| !t.is_empty())
        .context("--token (or RA__CTL__TOKEN) is required")?;
    let http = Client::new();
    let base = cli.base_url.trim_end_matches('/');

    match cli.cmd {
        Cmd::Sources => {
            let rows: Vec<Value> = http
                .get(format!("{base}/v1/admin/sources"))
                .header("X-Internal-Token", token)
                .send()
                .await
                .context("request failed")?
                .error_for_status()
                .context("server returned an error (check the token / port-forward)")?
                .json()
                .await
                .context("decoding response")?;
            print_sources(&rows);
        }
        Cmd::Claim { owner_id, account_id, plan } => {
            let body = ClaimRequest { owner_id, account_id, billing_plan: plan };
            let resp = http
                .post(format!("{base}/v1/admin/claim"))
                .header("X-Internal-Token", token)
                .json(&body)
                .send()
                .await
                .context("request failed")?;
            if resp.status() == reqwest::StatusCode::NOT_FOUND {
                bail!("no Source with that owner_id — has the org installed the App yet? (`sources` to list)");
            }
            let row: Value = resp
                .error_for_status()
                .context("server returned an error (check the token / port-forward)")?
                .json()
                .await
                .context("decoding response")?;
            println!("claimed:");
            print_sources(std::slice::from_ref(&row));
        }
    }
    Ok(())
}

fn print_sources(rows: &[Value]) {
    if rows.is_empty() {
        println!("(no sources)");
        return;
    }
    let g = |r: &Value, k: &str| r.get(k).and_then(Value::as_str).unwrap_or("-").to_string();
    println!(
        "{:<30} {:<14} {:<16} {:<8} {:<9} STATUS",
        "SOURCE_ID", "OWNER_ID", "ACCOUNT_ID", "PLAN", "SCOPE"
    );
    for r in rows {
        let account = r.get("account_id").and_then(Value::as_str).unwrap_or("<unclaimed>");
        println!(
            "{:<30} {:<14} {:<16} {:<8} {:<9} {}",
            g(r, "id"),
            g(r, "repository_owner_id"),
            account,
            g(r, "billing_plan"),
            g(r, "repo_scope"),
            g(r, "status"),
        );
    }
}
