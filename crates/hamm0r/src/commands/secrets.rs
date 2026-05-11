//! Tauri commands that expose the OS-keychain secret store to the UI.
//!
//! The UI can store a token, forget a stored token, and ask whether one is
//! present — but it can never read the stored value back. The plaintext
//! crosses the JS→Rust bridge exactly once on save, then lives only in the
//! OS credential vault. The runner reads it directly from there at request
//! time.

use storage::secrets::{self, TokenStatus};

use crate::error::CommandError;

/// Persist `token` for the env-var-shadow account `var` in the OS keychain.
///
/// Overwrites any previously stored value. The caller (UI) must clear its
/// own input field after invoking this so the plaintext doesn't linger in
/// the renderer process.
#[tauri::command]
pub fn set_bearer_token(var: String, token: String) -> Result<(), CommandError> {
    if var.trim().is_empty() {
        return Err(CommandError::storage(anyhow::anyhow!(
            "env var name must not be empty"
        )));
    }
    if token.is_empty() {
        return Err(CommandError::storage(anyhow::anyhow!(
            "token must not be empty"
        )));
    }
    secrets::set_token(&var, &token).map_err(CommandError::storage)
}

/// Persist `token` for an arbitrary keychain account reference.
#[tauri::command]
pub fn set_secret_ref(secret_ref: String, token: String) -> Result<(), CommandError> {
    if secret_ref.trim().is_empty() {
        return Err(CommandError::storage(anyhow::anyhow!(
            "secret reference must not be empty"
        )));
    }
    if token.is_empty() {
        return Err(CommandError::storage(anyhow::anyhow!(
            "token must not be empty"
        )));
    }
    secrets::set_token(&secret_ref, &token).map_err(CommandError::storage)
}

/// Remove the stored token for `var`. Idempotent.
#[tauri::command]
pub fn forget_bearer_token(var: String) -> Result<(), CommandError> {
    secrets::remove_token(&var).map_err(CommandError::storage)
}

/// Remove the stored secret for `secret_ref`. Idempotent.
#[tauri::command]
pub fn forget_secret_ref(secret_ref: String) -> Result<(), CommandError> {
    secrets::remove_token(&secret_ref).map_err(CommandError::storage)
}

/// Report whether a token is stored in the keychain and whether the
/// matching env var is currently set in the running process.
///
/// Never returns the token value itself.
#[tauri::command]
pub fn bearer_token_status(var: String) -> Result<TokenStatus, CommandError> {
    secrets::token_status(&var).map_err(CommandError::storage)
}

/// Report whether a secret is stored for the given keychain account reference.
#[tauri::command]
pub fn secret_ref_status(secret_ref: String) -> Result<TokenStatus, CommandError> {
    secrets::token_status(&secret_ref).map_err(CommandError::storage)
}
