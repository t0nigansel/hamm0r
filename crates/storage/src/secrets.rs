//! OS-keychain-backed storage for user-supplied secrets (bearer tokens,
//! basic-auth credentials, custom-header values).
//!
//! Tokens are keyed by the env-var name they shadow — e.g. an entry stored
//! under the account `"PROFILER_BEARER_TOKEN"` is consulted whenever the
//! runner would otherwise read `std::env::var("PROFILER_BEARER_TOKEN")`.
//! Storage is the OS-native credential vault (Windows Credential Manager,
//! macOS Keychain, Linux Secret Service), so plaintext never lands in
//! `config.yaml`, in run logs, or in the diagnostic export.
//!
//! When the OS vault is unreachable (typically Linux/WSL without a Secret
//! Service daemon and D-Bus session), reads gracefully fall through to the
//! env-var path — so the runner keeps working — while writes surface a
//! clear "keychain unavailable" error pointing the user at the env var.
//!
//! Honors CLAUDE.md Invariant 11 ("API keys come from environment variables
//! or user-provided inputs at runtime") by treating the keychain as the
//! at-rest form of a runtime-provided input. Token values are never logged.

use anyhow::{anyhow, Context as _, Result};
use keyring::{Entry, Error as KeyringError};
use serde::{Deserialize, Serialize};

const SERVICE: &str = "hamm0r";

/// Snapshot of where a given env-var-shadow secret can be resolved from.
/// Used by the UI to render a status line; never carries the token value.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TokenStatus {
    /// Whether a value is stored in the OS keychain under this account.
    pub stored_in_keychain: bool,
    /// Whether the environment variable of the same name is currently set
    /// in the running process.
    pub env_var_set: bool,
    /// Whether the OS keychain backend is reachable at all. False on
    /// Linux/WSL without a Secret Service daemon, on headless servers,
    /// or whenever the credential vault refuses to respond. The UI uses
    /// this to disable the Set / Replace button and explain why.
    pub keychain_available: bool,
}

fn entry(account: &str) -> Result<Entry> {
    if account.trim().is_empty() {
        return Err(anyhow!("account name (env-var) must not be empty"));
    }
    Entry::new(SERVICE, account).context("keychain entry handle")
}

/// Did this keyring error come from the backend itself being unreachable
/// (no Secret Service, locked vault that won't initialise, etc.) rather
/// than from a missing entry or other in-band condition?
fn is_backend_unavailable(err: &KeyringError) -> bool {
    matches!(
        err,
        KeyringError::PlatformFailure(_) | KeyringError::NoStorageAccess(_)
    )
}

/// User-facing message for the "backend not reachable" case. Hints at the
/// most common cause on the current platform and points at the env-var
/// fallback.
fn unavailable_message(account: &str) -> String {
    let advice = if cfg!(target_os = "linux") {
        "no Secret Service daemon (e.g. gnome-keyring) seems to be running, \
         or there's no D-Bus session — common in WSL and headless setups"
    } else {
        "the OS credential vault couldn't be reached"
    };
    format!(
        "OS keychain not available — {advice}. \
         Use the env var fallback: export {account} in the shell that launches hamm0r."
    )
}

/// Store `token` for the given env-var-shadow account. Overwrites any
/// existing value. The token value is not logged.
pub fn set_token(account: &str, token: &str) -> Result<()> {
    let entry = entry(account)?;
    match entry.set_password(token) {
        Ok(()) => Ok(()),
        Err(e) if is_backend_unavailable(&e) => Err(anyhow!("{}", unavailable_message(account))),
        Err(e) => Err(anyhow::Error::from(e))
            .with_context(|| format!("Couldn't save token to OS keychain for {account}")),
    }
}

/// Read the token stored for `account`. Returns `Ok(None)` for both "no
/// entry" and "backend unreachable" — the latter so the runner falls
/// through to the env-var path transparently on WSL/headless Linux.
pub fn get_token(account: &str) -> Result<Option<String>> {
    match entry(account)?.get_password() {
        Ok(value) => Ok(Some(value)),
        Err(KeyringError::NoEntry) => Ok(None),
        Err(e) if is_backend_unavailable(&e) => Ok(None),
        Err(e) => Err(anyhow::Error::from(e))
            .with_context(|| format!("Couldn't read token from OS keychain for {account}")),
    }
}

/// Remove the keychain entry for `account`. Idempotent — succeeds when no
/// entry exists.
pub fn remove_token(account: &str) -> Result<()> {
    match entry(account)?.delete_credential() {
        Ok(()) => Ok(()),
        Err(KeyringError::NoEntry) => Ok(()),
        Err(e) if is_backend_unavailable(&e) => Err(anyhow!("{}", unavailable_message(account))),
        Err(e) => Err(anyhow::Error::from(e))
            .with_context(|| format!("Couldn't remove token from OS keychain for {account}")),
    }
}

/// Report whether a token is stored in the keychain, whether the matching
/// env var is set, and whether the keychain backend is reachable at all.
/// Does not return the token value.
pub fn token_status(account: &str) -> Result<TokenStatus> {
    let env_var_set = std::env::var(account).is_ok();

    let (stored_in_keychain, keychain_available) = match entry(account)?.get_password() {
        Ok(_) => (true, true),
        Err(KeyringError::NoEntry) => (false, true),
        Err(e) if is_backend_unavailable(&e) => (false, false),
        Err(e) => {
            return Err(anyhow::Error::from(e))
                .with_context(|| format!("Couldn't check OS keychain for {account}"));
        }
    };

    Ok(TokenStatus {
        stored_in_keychain,
        env_var_set,
        keychain_available,
    })
}

/// Resolve a token using the documented precedence: keychain first, then
/// environment variable. Returns `Ok(None)` when neither source has a
/// value, and treats an unreachable keychain backend as "no value" so the
/// env var is consulted regardless. Used by the runner's auth layer.
pub fn resolve_token(account: &str) -> Result<Option<String>> {
    if let Some(v) = get_token(account)? {
        return Ok(Some(v));
    }
    Ok(std::env::var(account).ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Round-trip against the real OS keychain. Marked `#[ignore]` because
    /// headless CI (Linux without a Secret Service daemon) cannot exercise
    /// the backend. Run locally with `cargo test -p storage -- --ignored`.
    #[test]
    #[ignore]
    fn keychain_round_trip() {
        let acct = "HAMM0R_TEST_ROUND_TRIP";
        let _ = remove_token(acct);

        assert!(get_token(acct).unwrap().is_none());
        set_token(acct, "secret-value").unwrap();
        assert_eq!(get_token(acct).unwrap().as_deref(), Some("secret-value"));

        set_token(acct, "rotated").unwrap();
        assert_eq!(get_token(acct).unwrap().as_deref(), Some("rotated"));

        let status = token_status(acct).unwrap();
        assert!(status.stored_in_keychain);
        assert!(status.keychain_available);

        remove_token(acct).unwrap();
        assert!(get_token(acct).unwrap().is_none());
        remove_token(acct).unwrap();
    }

    #[test]
    #[ignore]
    fn resolve_prefers_keychain_over_env() {
        let acct = "HAMM0R_TEST_PRECEDENCE";
        let _ = remove_token(acct);

        std::env::set_var(acct, "from-env");
        assert_eq!(resolve_token(acct).unwrap().as_deref(), Some("from-env"));

        set_token(acct, "from-keychain").unwrap();
        assert_eq!(
            resolve_token(acct).unwrap().as_deref(),
            Some("from-keychain")
        );

        remove_token(acct).unwrap();
        assert_eq!(resolve_token(acct).unwrap().as_deref(), Some("from-env"));

        std::env::remove_var(acct);
        assert!(resolve_token(acct).unwrap().is_none());
    }

    #[test]
    fn empty_account_rejected() {
        assert!(set_token("", "x").is_err());
        assert!(get_token("   ").is_err());
        assert!(remove_token("").is_err());
        assert!(token_status("").is_err());
    }
}
