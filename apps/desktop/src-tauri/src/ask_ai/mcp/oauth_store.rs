//! Keychain-backed rmcp [`CredentialStore`] for an OAuth MCP connector (ADR 0051).
//!
//! rmcp's `AuthorizationManager` persists an OAuth **Token Set**
//! ([`StoredCredentials`]: client id + token response + granted scopes + issue
//! time) through a [`CredentialStore`]. This adapter is that store, backed by the
//! ONE keychain slot each connector already owns (keyed by instance id, the same
//! slot the bearer-secret path uses — a connector is either bearer OR oauth, so
//! the slot is never double-booked). The Token Set rides through as its JSON
//! serialization; the existing `has_mcp_server_secret(id)` — "is a token
//! present?", i.e. "is this connector authorized?" — then answers for free.
//!
//! No parallel token struct, no new keychain slot, no migration: `StoredCredentials`
//! IS the Token Set, and the opaque `String` slot already stores arbitrary bytes.

use async_trait::async_trait;
use rmcp::transport::auth::{AuthError, CredentialStore, StoredCredentials};

/// A [`CredentialStore`] over one connector's keychain slot, keyed by instance id.
pub(crate) struct OAuthCredentialStore {
    id: String,
}

impl OAuthCredentialStore {
    pub(crate) fn new(id: String) -> Self {
        Self { id }
    }
}

#[async_trait]
impl CredentialStore for OAuthCredentialStore {
    async fn load(&self) -> Result<Option<StoredCredentials>, AuthError> {
        // The slot holds the Token Set's JSON, or nothing when unauthorized.
        let Some(json) = app_infra::load_mcp_server_secret(&self.id)
            .map_err(|error| AuthError::InternalError(error.to_string()))?
        else {
            return Ok(None);
        };
        let creds = serde_json::from_str::<StoredCredentials>(&json)
            .map_err(|error| AuthError::InternalError(error.to_string()))?;
        Ok(Some(creds))
    }

    async fn save(&self, credentials: StoredCredentials) -> Result<(), AuthError> {
        let json = serde_json::to_string(&credentials)
            .map_err(|error| AuthError::InternalError(error.to_string()))?;
        app_infra::store_mcp_server_secret(&self.id, &json)
            .map_err(|error| AuthError::InternalError(error.to_string()))
    }

    async fn clear(&self) -> Result<(), AuthError> {
        app_infra::delete_mcp_server_secret(&self.id)
            .map_err(|error| AuthError::InternalError(error.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The Token Set round-trips through the vault slot as JSON, and once one
    /// is saved `has_mcp_server_secret` reports the connector authorized — that
    /// presence check is the entire "authorized?" signal Settings reads. Uses
    /// the shared file-key-backed test vault (no real keychain in CI).
    #[tokio::test]
    async fn token_set_round_trips_and_reports_authorized() {
        crate::secret_vault_test_support::install_shared_test_secret_vault();

        let store = OAuthCredentialStore::new("oauth-connector".to_string());
        assert!(
            store.load().await.expect("load should succeed").is_none(),
            "an unauthorized connector has no Token Set"
        );

        let creds = StoredCredentials::new("client-abc".to_string(), None, Vec::new(), None);
        store.save(creds).await.expect("save should succeed");

        let loaded = store
            .load()
            .await
            .expect("load should succeed")
            .expect("a Token Set was just saved");
        assert_eq!(loaded.client_id, "client-abc");
        assert!(
            app_infra::has_mcp_server_secret("oauth-connector").expect("has should succeed"),
            "a saved Token Set must read as authorized"
        );

        store.clear().await.expect("clear should succeed");
        assert!(
            !app_infra::has_mcp_server_secret("oauth-connector").expect("has should succeed"),
            "clearing the Token Set must read as unauthorized"
        );
    }
}
