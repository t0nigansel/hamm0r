use thiserror::Error;

#[derive(Debug, Error)]
pub enum RunnerError {
    #[error("template render failed: {0}")]
    Template(#[from] minijinja::Error),

    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),

    #[error("response extraction failed: {reason}")]
    Extraction { reason: String },

    #[error("storage error: {0}")]
    Storage(#[from] anyhow::Error),

    #[error("no token found for '{var}' — paste one in the target editor or export the env var, then retry")]
    MissingEnvVar { var: String },
}
