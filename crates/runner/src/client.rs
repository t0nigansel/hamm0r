use std::time::Duration;

use crate::error::RunnerError;

/// Build a `reqwest::Client` with explicit timeouts and rustls TLS.
///
/// Per Stack.md: connect_timeout 10 s, total timeout 30 s, rustls (no OpenSSL).
/// TLS verification is on by default; the request config can override per-call
/// once the per-request TLS flag is wired up (Milestone 3+).
pub fn build_http_client(timeout_seconds: u32) -> Result<reqwest::Client, RunnerError> {
    reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(u64::from(timeout_seconds)))
        .use_rustls_tls()
        .build()
        .map_err(RunnerError::Http)
}
