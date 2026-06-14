//! Domain types + the wire shapes for (a) GitHub webhooks we consume and
//! (b) the `/v1/resolve` contract Authorino calls.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

// ─────────────────────────── stored entities ───────────────────────────

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct IdentitySource {
    pub id: String,
    /// External billing-account reference (the account lives in the billing
    /// system, not here). NULL until the dashboard *claims* the install — an
    /// unclaimed source never resolves to `allowed`.
    pub account_id: Option<String>,
    /// GitHub numeric org/user id — the unforgeable binding. Stored as text
    /// because GitHub ids exceed i32 and text avoids signedness games.
    pub repository_owner_id: String,
    pub installation_id: i64,
    /// `all` | `selected`
    pub repo_scope: String,
    /// Tier selector stamped downstream as `x-billing-plan` (free|pro|service).
    /// Set by the dashboard/billing sync; quota enforcement stays in Authorino.
    pub billing_plan: String,
    /// `active` | `disabled` (uninstalled) | `suspended` (GitHub-suspended)
    pub status: String,
    /// Operator block (separate from `status`; survives reinstalls). `true` →
    /// resolve denies regardless of status. Set via `/v1/admin/block`.
    #[serde(default)]
    pub blocked: bool,
    #[serde(default)]
    pub account_login: Option<String>,
    /// `Organization` | `User` (from the install webhook's `account.type`).
    #[serde(default)]
    pub account_type: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceStatus {
    Active,
    Disabled,
    Suspended,
}

impl SourceStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            SourceStatus::Active => "active",
            SourceStatus::Disabled => "disabled",
            SourceStatus::Suspended => "suspended",
        }
    }
}

// ─────────────────────────── /v1/resolve ───────────────────────────

/// Body Authorino POSTs (built from the validated GitHub-Actions JWT claims via
/// CEL). The service trusts these claims because the caller proved itself with
/// `X-Internal-Token` and the endpoint is ClusterIP-only — it does NOT re-verify
/// the JWT signature (Authorino already did).
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct ResolveRequest {
    /// `aud` claim — the Source URL the customer set in their workflow.
    pub audience: String,
    /// `repository_owner_id` claim — server-set by GitHub, immutable.
    pub repository_owner_id: String,
    /// `repository_id` claim — used only for `selected`-repo scoping.
    #[serde(default)]
    pub repository_id: Option<String>,
    /// `repository` claim — `org/repo`, for attribution/logging.
    #[serde(default)]
    pub repository: Option<String>,
    /// `sub` claim — `repo:org/repo:ref:...`, the emergent identity instance.
    #[serde(default)]
    pub subject: Option<String>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ResolveResponse {
    /// The single field Authorino's authorization step gates on.
    pub allowed: bool,
    /// Machine reason when `allowed=false` (logged, never user-facing).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub account_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub billing_plan: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub repository: Option<String>,
}

impl ResolveResponse {
    pub fn deny(reason: &str) -> Self {
        Self {
            allowed: false,
            reason: Some(reason.to_string()),
            account_id: None,
            source_id: None,
            billing_plan: None,
            repository: None,
        }
    }
}

// ─────────────────────────── /v1/admin/claim ───────────────────────────

/// Link a Source to a billing account (the step a dashboard would do on the
/// post-install redirect). Selected by `owner_id` (the binding key).
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ClaimRequest {
    /// GitHub numeric org/user id of the Source to claim.
    pub owner_id: String,
    /// External billing-account reference to stamp as `x-account-id`.
    pub account_id: String,
    /// Tier (`free|pro|service|internal`); omitted → leave the current value.
    #[serde(default)]
    pub billing_plan: Option<String>,
}

// ─────────────────────────── /v1/admin/block ───────────────────────────

/// Toggle the operator block on a Source (selected by owner id).
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct BlockRequest {
    pub owner_id: String,
    pub blocked: bool,
}

// ─────────────────────────── GitHub webhooks ───────────────────────────
// Only the fields we act on are typed; the rest are ignored. octocrab's webhook
// types are still beta, so these are hand-rolled and stable.

#[derive(Debug, Clone, Deserialize)]
pub struct InstallationEvent {
    /// created | deleted | suspend | unsuspend | new_permissions_accepted
    pub action: String,
    pub installation: Installation,
    #[serde(default)]
    pub repositories: Vec<Repo>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct InstallationReposEvent {
    /// added | removed
    pub action: String,
    pub installation: Installation,
    #[serde(default)]
    pub repositories_added: Vec<Repo>,
    #[serde(default)]
    pub repositories_removed: Vec<Repo>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Installation {
    pub id: i64,
    pub account: Account,
    /// `all` | `selected`
    #[serde(default)]
    pub repository_selection: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Account {
    /// Numeric id — the binding anchor. Works for both Organization and User
    /// installs, so we never branch on `account.type`.
    pub id: i64,
    #[serde(default)]
    pub login: Option<String>,
    #[serde(rename = "type", default)]
    pub account_type: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Repo {
    pub id: i64,
    #[serde(default)]
    pub full_name: Option<String>,
}
