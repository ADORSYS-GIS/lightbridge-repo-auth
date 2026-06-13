//! Core domain for `lightbridge-repo-auth`.
//!
//! The service is the **control plane** of the GitHub-OIDC → AI-gateway trust
//! model: a GitHub App webhook handler that records which GitHub org
//! (`repository_owner_id`) is bound to which billing account, plus a `/v1/resolve`
//! endpoint that Authorino calls at request time to turn an (already
//! JWT-validated) GitHub Actions identity into an account + billing plan.
//!
//! It never mints tokens and never validates the runtime JWT signature — that is
//! Authorino's job (issuer `https://token.actions.githubusercontent.com`). This
//! crate only owns the *binding* the data plane checks against.

pub mod config;
pub mod error;
pub mod github;
pub mod model;
pub mod store;
pub mod webhook;

pub use config::Config;
pub use error::{Error, Result};
pub use store::Store;
