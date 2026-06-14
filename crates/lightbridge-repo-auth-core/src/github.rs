//! Thin GitHub App client — only what the control plane needs:
//!   * mint the App JWT (RS256) for control-plane API calls,
//!   * convert a manifest `code` into App credentials (one-time registration),
//!   * list installations + their repos (the reconcile sweep).
//!
//! No runtime-token path lives here: the App never touches a CI request.

use jsonwebtoken::{encode, Algorithm, EncodingKey, Header};
use reqwest::Client;
use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::model::{Installation, Repo};

const UA: &str = "lightbridge-repo-auth";
const API_VERSION: &str = "2022-11-28";

#[derive(Clone)]
pub struct GithubClient {
    http: Client,
    api_base: String,
    app_id: u64,
    encoding_key: EncodingKey,
}

#[derive(Debug, Serialize)]
struct AppClaims {
    iat: i64,
    exp: i64,
    iss: String,
}

/// Result of `POST /app-manifests/{code}/conversions` — store these once.
#[derive(Debug, Deserialize)]
pub struct ManifestConversion {
    pub id: u64,
    pub slug: String,
    pub client_id: String,
    pub client_secret: String,
    pub webhook_secret: String,
    pub pem: String,
}

#[derive(Debug, Deserialize)]
struct InstallationToken {
    token: String,
}

impl GithubClient {
    /// `private_key_pem` may be empty for the manifest-conversion-only path
    /// (registration happens before you have a key); in that case the JWT
    /// methods will error if called.
    pub fn new(api_base: String, app_id: u64, private_key_pem: &str) -> Result<Self> {
        let encoding_key = if private_key_pem.trim().is_empty() {
            // Placeholder; only the static `convert_manifest` is usable.
            EncodingKey::from_secret(b"unset")
        } else {
            EncodingKey::from_rsa_pem(private_key_pem.as_bytes())?
        };
        Ok(Self {
            http: Client::builder().user_agent(UA).build()?,
            api_base,
            app_id,
            encoding_key,
        })
    }

    /// Mint a short-lived (≤10 min) App JWT. `iat` is back-dated 60s to absorb
    /// clock skew between us and GitHub.
    pub(crate) fn app_jwt(&self) -> Result<String> {
        let now = chrono::Utc::now().timestamp();
        let claims = AppClaims {
            iat: now - 60,
            exp: now + 540,
            iss: self.app_id.to_string(),
        };
        Ok(encode(&Header::new(Algorithm::RS256), &claims, &self.encoding_key)?)
    }

    async fn get_json<T: serde::de::DeserializeOwned>(&self, url: &str, bearer: &str) -> Result<T> {
        let resp = self
            .http
            .get(url)
            .bearer_auth(bearer)
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", API_VERSION)
            .send()
            .await?;
        if !resp.status().is_success() {
            return Err(Error::Github(format!("GET {url} → {}", resp.status())));
        }
        Ok(resp.json::<T>().await?)
    }

    /// One-time App registration: exchange the manifest redirect `code`.
    /// `code` is single-use and expires ~1h after the redirect.
    pub async fn convert_manifest(&self, code: &str) -> Result<ManifestConversion> {
        let url = format!("{}/app-manifests/{}/conversions", self.api_base, code);
        let resp = self
            .http
            .post(&url)
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", API_VERSION)
            .send()
            .await?;
        if !resp.status().is_success() {
            return Err(Error::Github(format!("manifest conversion → {}", resp.status())));
        }
        Ok(resp.json::<ManifestConversion>().await?)
    }

    /// All current installations of the App (paginated; one page is plenty for
    /// our scale, extend with `?per_page=100&page=N` if needed).
    pub async fn list_installations(&self) -> Result<Vec<Installation>> {
        let jwt = self.app_jwt()?;
        let url = format!("{}/app/installations?per_page=100", self.api_base);
        self.get_json(&url, &jwt).await
    }

    async fn installation_token(&self, installation_id: i64) -> Result<String> {
        let jwt = self.app_jwt()?;
        let url = format!("{}/app/installations/{installation_id}/access_tokens", self.api_base);
        let resp = self
            .http
            .post(&url)
            .bearer_auth(&jwt)
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", API_VERSION)
            .send()
            .await?;
        if !resp.status().is_success() {
            return Err(Error::Github(format!("installation token → {}", resp.status())));
        }
        Ok(resp.json::<InstallationToken>().await?.token)
    }

    /// Repos visible to an installation (only needed for `selected` scope).
    pub async fn list_installation_repos(&self, installation_id: i64) -> Result<Vec<Repo>> {
        #[derive(Deserialize)]
        struct Wrap {
            repositories: Vec<Repo>,
        }
        let token = self.installation_token(installation_id).await?;
        let url = format!("{}/installation/repositories?per_page=100", self.api_base);
        let wrap: Wrap = self.get_json(&url, &token).await?;
        Ok(wrap.repositories)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Throwaway PKCS#1 key — proves App-JWT signing works with the configured
    // jsonwebtoken crypto backend (regression guard for the v10 "CryptoProvider
    // not installed" panic).
    const TEST_KEY: &str = include_str!("../testdata/test-rsa.pem");

    #[test]
    fn app_jwt_signs_without_panicking() {
        let c = GithubClient::new("https://api.github.com".into(), 12345, TEST_KEY).unwrap();
        let jwt = c.app_jwt().expect("RS256 sign must succeed with a crypto backend");
        assert_eq!(jwt.split('.').count(), 3, "a JWT has three parts");
    }
}
