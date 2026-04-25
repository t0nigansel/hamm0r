use std::collections::HashMap;
use std::path::Path;

use anyhow::Context as _;

use crate::types::Request;
use crate::write::atomic_write;

/// Load all request templates from `dir`, keyed by filename stem.
pub fn load_all(dir: &Path) -> anyhow::Result<HashMap<String, Request>> {
    if !dir.exists() {
        return Ok(HashMap::new());
    }

    let mut map = HashMap::new();

    for entry in std::fs::read_dir(dir)
        .with_context(|| format!("cannot read requests directory: {}", dir.display()))?
    {
        let entry = entry?;
        let path = entry.path();

        if path.extension().and_then(|e| e.to_str()) != Some("yaml") {
            continue;
        }

        let stem = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_owned();

        let raw = std::fs::read_to_string(&path)
            .with_context(|| format!("cannot read {}", path.display()))?;

        let request: Request = serde_yaml::from_str(&raw)
            .with_context(|| format!("cannot parse {}", path.display()))?;

        map.insert(stem, request);
    }

    Ok(map)
}

/// Remove a request YAML file by id. Returns Ok even if the file did not exist.
pub fn delete(dir: &Path, id: &str) -> anyhow::Result<()> {
    let path = dir.join(format!("{id}.yaml"));
    if path.exists() {
        std::fs::remove_file(&path)
            .with_context(|| format!("cannot delete {}", path.display()))?;
    }
    Ok(())
}

/// Persist a request template. The file is named `<request.id>.yaml`.
pub fn save(dir: &Path, request: &Request) -> anyhow::Result<()> {
    let path = dir.join(format!("{}.yaml", request.id));
    let yaml = serde_yaml::to_string(request).context("cannot serialise request")?;
    atomic_write(&path, yaml.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{AuthConfig, BodyConfig, BodyFormat, ExtractConfig, ResponseConfig};
    use tempfile::TempDir;

    fn sample_request() -> Request {
        Request {
            version: 1,
            id: "openai-chat".into(),
            name: "OpenAI Chat Completion".into(),
            method: "POST".into(),
            url: "https://api.openai.com/v1/chat/completions".into(),
            auth: AuthConfig::Bearer { token_env: "OPENAI_API_KEY".into() },
            headers: [("Content-Type".into(), "application/json".into())].into(),
            body: BodyConfig {
                format: BodyFormat::Json,
                content: serde_json::json!({
                    "model": "gpt-4",
                    "messages": [{"role": "user", "content": "{{prompt}}"}]
                }),
            },
            response: ResponseConfig {
                extract: ExtractConfig::Jsonpath {
                    path: "$.choices[0].message.content".into(),
                },
            },
            timeout_seconds: 30,
            adapter: Default::default(),
        }
    }

    #[test]
    fn save_then_load_all() {
        let dir = TempDir::new().unwrap();
        let req = sample_request();
        save(dir.path(), &req).unwrap();

        let map = load_all(dir.path()).unwrap();
        assert_eq!(map.len(), 1);
        assert_eq!(map["openai-chat"], req);
    }

    #[test]
    fn load_all_empty_dir() {
        let dir = TempDir::new().unwrap();
        assert!(load_all(dir.path()).unwrap().is_empty());
    }

    #[test]
    fn save_overwrites() {
        let dir = TempDir::new().unwrap();
        let mut req = sample_request();
        save(dir.path(), &req).unwrap();
        req.name = "Updated".into();
        save(dir.path(), &req).unwrap();

        let map = load_all(dir.path()).unwrap();
        assert_eq!(map["openai-chat"].name, "Updated");
    }
}
