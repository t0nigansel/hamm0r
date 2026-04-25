use serde::Serialize;

/// Error type for Tauri commands.
///
/// Per Architecture.md: errors crossing the Tauri boundary must serialise as
/// `{ kind, message }` — the frontend never sees a Rust Debug representation.
#[derive(Debug, Serialize)]
pub struct CommandError {
    kind: &'static str,
    message: String,
}

impl CommandError {
    pub fn storage(e: anyhow::Error) -> Self {
        Self { kind: "storage", message: e.to_string() }
    }
}

impl From<anyhow::Error> for CommandError {
    fn from(e: anyhow::Error) -> Self {
        Self::storage(e)
    }
}
