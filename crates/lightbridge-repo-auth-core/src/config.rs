//! Plain configuration structs. The `app` binary populates these from
//! environment / flags (clap), so this crate stays free of CLI concerns.

#[derive(Debug, Clone)]
pub struct Config {
    pub server: ServerConfig,
    pub database: DatabaseConfig,
    pub github: GithubConfig,
    pub resolve: ResolveConfig,
    pub source: SourceConfig,
}

#[derive(Debug, Clone)]
pub struct ServerConfig {
    /// e.g. `0.0.0.0:3000`
    pub bind: String,
    /// Optional TLS (cert + key PEM paths). When absent, serves plain HTTP —
    /// fine in-cluster behind the gateway / a service mesh.
    pub tls_cert_path: Option<String>,
    pub tls_key_path: Option<String>,
}

#[derive(Debug, Clone)]
pub struct DatabaseConfig {
    pub url: String,
    pub max_connections: u32,
}

#[derive(Debug, Clone)]
pub struct GithubConfig {
    /// Numeric GitHub App id (from the manifest conversion).
    pub app_id: u64,
    /// HMAC secret used to verify inbound webhook signatures.
    pub webhook_secret: String,
    /// RS256 private key (PEM) used to mint installation tokens for the
    /// reconcile sweep. Read once at startup.
    pub private_key_pem: String,
    /// GitHub API base — overridable for GHES.
    pub api_base: String,
}

#[derive(Debug, Clone)]
pub struct ResolveConfig {
    /// Shared secret Authorino must present on `/v1/resolve` via
    /// `X-Internal-Token`. The endpoint trusts the claims in the request body
    /// only because (a) it is ClusterIP-only and (b) this token matches.
    pub internal_token: String,
}

#[derive(Debug, Clone)]
pub struct SourceConfig {
    /// Base URL each Source audience is built from: `<base>/sources/<source_id>`.
    /// This is the string a customer puts in their workflow's `audience:`.
    pub audience_base: String,
}

impl SourceConfig {
    /// The canonical audience for a given source id.
    pub fn audience_for(&self, source_id: &str) -> String {
        format!("{}/sources/{}", self.audience_base.trim_end_matches('/'), source_id)
    }
}
