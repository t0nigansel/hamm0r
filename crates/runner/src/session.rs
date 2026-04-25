use std::collections::HashMap;

use crate::client::build_http_client;
use crate::error::RunnerError;

/// Strategy for maintaining state across steps within one session.
///
/// For single-step runs (Milestone 3) only `None` is exercised.
/// Multi-step session state extraction is wired up in Milestone 5.
#[derive(Debug, Clone, Default)]
pub enum SessionStrategy {
    /// No shared state between steps.
    #[default]
    None,
    /// Share cookies across steps in the same session label.
    Cookie,
    /// Carry a token in a named request header.
    Header { header_name: String },
    /// Inject a token into the request body as a named field.
    BodyField { field_name: String },
}

/// Manages one `reqwest::Client` per session label within a run.
///
/// Steps that share a session label get the same client (and therefore the
/// same cookie jar). Steps with different labels are isolated.
pub struct SessionManager {
    strategy: SessionStrategy,
    timeout_seconds: u32,
    clients: HashMap<String, reqwest::Client>,
}

impl SessionManager {
    pub fn new(strategy: SessionStrategy, timeout_seconds: u32) -> Self {
        Self { strategy, timeout_seconds, clients: HashMap::new() }
    }

    /// Get or create the `reqwest::Client` for the given session label.
    pub fn client_for(&mut self, session: &str) -> Result<&reqwest::Client, RunnerError> {
        if !self.clients.contains_key(session) {
            let client = match &self.strategy {
                SessionStrategy::Cookie => reqwest::Client::builder()
                    .connect_timeout(std::time::Duration::from_secs(10))
                    .timeout(std::time::Duration::from_secs(u64::from(self.timeout_seconds)))
                    .cookie_store(true)
                    .use_rustls_tls()
                    .build()
                    .map_err(RunnerError::Http)?,
                _ => build_http_client(self.timeout_seconds)?,
            };
            self.clients.insert(session.to_owned(), client);
        }
        Ok(self.clients.get(session).unwrap())
    }
}
