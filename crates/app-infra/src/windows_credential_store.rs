//! Shared Windows Credential Manager backend for the per-account keychain
//! stores ([`crate::ai_provider_key_store`], [`crate::mcp_server_secret_store`]).
//!
//! Mirrors the conventions proven by `capture_index_key_store`: generic
//! credentials addressed as `"{service}:{account}"`, written under the app id
//! user name with `CRED_PERSIST_LOCAL_MACHINE`. (`capture_index_key_store`
//! keeps its own copy because its adapter differs — it has no delete path.)

use windows_sys::Win32::{
    Foundation::{GetLastError, ERROR_NOT_FOUND},
    Security::Credentials::{
        CredDeleteW, CredFree, CredReadW, CredWriteW, CREDENTIALW,
        CRED_MAX_CREDENTIAL_BLOB_SIZE, CRED_PERSIST_LOCAL_MACHINE, CRED_TYPE_GENERIC,
    },
};
use zeroize::Zeroizing;

use crate::error::{AppInfraError, Result};

const APP_ID: &str = "com.shaikzeeshan.mnema";

/// Read the credential for `service`/`account`, or `None` when absent.
/// `secret_kind` names the stored value in error messages (e.g. "ai provider
/// key"); `make_error` wraps them in the calling store's error variant.
pub(crate) fn load_credential(
    service: &str,
    account: &str,
    secret_kind: &str,
    make_error: fn(String) -> AppInfraError,
) -> Result<Option<String>> {
    let target = credential_target(service, account);
    let mut credential = std::ptr::null_mut::<CREDENTIALW>();

    let ok = unsafe { CredReadW(target.as_ptr(), CRED_TYPE_GENERIC, 0, &mut credential) };
    if ok == 0 {
        let code = unsafe { GetLastError() };
        if code == ERROR_NOT_FOUND {
            return Ok(None);
        }
        return Err(make_error(format!(
            "Windows Credential Manager failed to read {secret_kind} for {account}: error {code}"
        )));
    }

    if credential.is_null() {
        return Ok(None);
    }
    let _guard = WindowsCredentialGuard(credential);
    let credential = unsafe { &*credential };
    if credential.CredentialBlobSize == 0 {
        return Ok(None);
    }
    if credential.CredentialBlob.is_null() {
        return Err(make_error(format!(
            "Windows Credential Manager returned an empty {secret_kind} blob for {account}"
        )));
    }

    let bytes = unsafe {
        std::slice::from_raw_parts(
            credential.CredentialBlob,
            credential.CredentialBlobSize as usize,
        )
    };
    let value = std::str::from_utf8(bytes)
        .map_err(|error| {
            make_error(format!(
                "Windows Credential Manager returned a non-UTF-8 {secret_kind} for {account}: {error}"
            ))
        })?
        .trim()
        .to_string();
    if value.is_empty() {
        return Ok(None);
    }

    Ok(Some(value))
}

/// Create or update the credential for `service`/`account`.
pub(crate) fn store_credential(
    service: &str,
    account: &str,
    value: &str,
    secret_kind: &str,
    make_error: fn(String) -> AppInfraError,
) -> Result<()> {
    ensure_within_blob_limit(account, value, secret_kind, make_error)?;

    let mut target = credential_target(service, account);
    let mut user_name = credential_user_name();
    // Scrub the plaintext secret bytes once the credential write completes.
    let mut value_bytes = Zeroizing::new(value.as_bytes().to_vec());

    let mut credential = CREDENTIALW::default();
    credential.Type = CRED_TYPE_GENERIC;
    credential.TargetName = target.as_mut_ptr();
    // The blob-limit check above guarantees the length fits in a u32.
    credential.CredentialBlobSize = value_bytes.len() as u32;
    credential.CredentialBlob = value_bytes.as_mut_ptr();
    credential.Persist = CRED_PERSIST_LOCAL_MACHINE;
    credential.UserName = user_name.as_mut_ptr();

    let ok = unsafe { CredWriteW(&credential, 0) };
    if ok == 0 {
        let code = unsafe { GetLastError() };
        return Err(make_error(format!(
            "Windows Credential Manager failed to store {secret_kind} for {account}: error {code}"
        )));
    }

    Ok(())
}

/// Delete the credential for `service`/`account`. Deleting an absent
/// credential is a no-op, mirroring the macOS `errSecItemNotFound` handling.
pub(crate) fn delete_credential(
    service: &str,
    account: &str,
    secret_kind: &str,
    make_error: fn(String) -> AppInfraError,
) -> Result<()> {
    let target = credential_target(service, account);

    let ok = unsafe { CredDeleteW(target.as_ptr(), CRED_TYPE_GENERIC, 0) };
    if ok == 0 {
        let code = unsafe { GetLastError() };
        if code == ERROR_NOT_FOUND {
            return Ok(());
        }
        return Err(make_error(format!(
            "Windows Credential Manager failed to delete {secret_kind} for {account}: error {code}"
        )));
    }

    Ok(())
}

// Credential Manager caps a generic credential's blob at
// CRED_MAX_CREDENTIAL_BLOB_SIZE (2560 bytes); CredWriteW rejects anything
// larger, so fail up front with the named limit instead of surfacing a raw
// Win32 error code. (The MCP OAuth token-set JSON can approach this limit.)
fn ensure_within_blob_limit(
    account: &str,
    value: &str,
    secret_kind: &str,
    make_error: fn(String) -> AppInfraError,
) -> Result<()> {
    if value.len() > CRED_MAX_CREDENTIAL_BLOB_SIZE as usize {
        return Err(make_error(format!(
            "{secret_kind} for {account} is {} bytes, over the Windows Credential Manager limit of {CRED_MAX_CREDENTIAL_BLOB_SIZE} bytes",
            value.len()
        )));
    }
    Ok(())
}

struct WindowsCredentialGuard(*mut CREDENTIALW);

impl Drop for WindowsCredentialGuard {
    fn drop(&mut self) {
        unsafe { CredFree(self.0.cast()) };
    }
}

fn credential_target(service: &str, account: &str) -> Vec<u16> {
    format!("{service}:{account}")
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect()
}

fn credential_user_name() -> Vec<u16> {
    APP_ID.encode_utf16().chain(std::iter::once(0)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn credential_target_joins_service_and_account_with_colon() {
        let target = credential_target("com.example.service", "provider.id");

        assert_eq!(target.last(), Some(&0));
        let decoded = String::from_utf16(&target[..target.len() - 1])
            .expect("target name should be valid UTF-16");
        assert_eq!(decoded, "com.example.service:provider.id");
    }

    #[test]
    fn blob_limit_accepts_a_value_at_the_cap() {
        let at_limit = "x".repeat(CRED_MAX_CREDENTIAL_BLOB_SIZE as usize);

        ensure_within_blob_limit(
            "acct",
            &at_limit,
            "test secret",
            AppInfraError::McpServerSecretStore,
        )
        .expect("a value at the blob cap should be accepted");
    }

    #[test]
    fn blob_limit_rejects_a_value_over_the_cap() {
        let over_limit = "x".repeat(CRED_MAX_CREDENTIAL_BLOB_SIZE as usize + 1);

        let error = ensure_within_blob_limit(
            "acct",
            &over_limit,
            "test secret",
            AppInfraError::McpServerSecretStore,
        )
        .expect_err("a value over the blob cap should be rejected");

        let message = error.to_string();
        assert!(message.contains("test secret for acct"));
        assert!(message.contains("2560"));
    }
}
