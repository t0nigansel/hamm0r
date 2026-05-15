use std::collections::HashMap;
use std::path::Path;

use anyhow::Context as _;
use serde::{Deserialize, Serialize};

use crate::types::Request;
use crate::write::atomic_write;
use crate::{scenarios, targets};

/// A single place that references a Request. Used to warn the user before
/// deleting a Request that other artifacts depend on.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum RequestReference {
    Target { id: String, name: String },
    Scenario { id: String, name: String },
}

/// Scan Targets and Scenarios for references to `request_id`.
///
/// `targets_dir` and `scenarios_dir` are the canonical user-folder paths.
/// Either may be absent — missing directories produce no references.
pub fn references(
    targets_dir: &Path,
    scenarios_dir: &Path,
    request_id: &str,
) -> anyhow::Result<Vec<RequestReference>> {
    let mut refs = Vec::new();

    for target in targets::load_all(targets_dir)?.values() {
        let primary = target.request_id.trim() == request_id;
        let secondary = target.request_ids.iter().any(|id| id == request_id);
        if primary || secondary {
            refs.push(RequestReference::Target {
                id: target.id.clone(),
                name: target.name.clone(),
            });
        }
    }

    for scenario in scenarios::load_all(scenarios_dir)?.values() {
        if scenario.request_ids.iter().any(|id| id == request_id) {
            refs.push(RequestReference::Scenario {
                id: scenario.id.clone(),
                name: scenario.name.clone(),
            });
        }
    }

    Ok(refs)
}

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

        let raw = match std::fs::read_to_string(&path) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("[storage] skipping {}: cannot read: {}", path.display(), e);
                continue;
            }
        };

        match serde_yaml::from_str::<Request>(&raw) {
            Ok(request) => {
                map.insert(stem, request);
            }
            Err(e) => {
                eprintln!("[storage] skipping {}: cannot parse: {}", path.display(), e);
            }
        }
    }

    Ok(map)
}

/// Remove a request YAML file by id. Returns Ok even if the file did not exist.
pub fn delete(dir: &Path, id: &str) -> anyhow::Result<()> {
    let path = dir.join(format!("{id}.yaml"));
    if path.exists() {
        std::fs::remove_file(&path).with_context(|| format!("cannot delete {}", path.display()))?;
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
    use crate::types::{
        AuthConfig, BodyConfig, BodyFormat, ExtractConfig, ResponseConfig, Scenario, Target,
    };
    use tempfile::TempDir;

    fn sample_request() -> Request {
        Request {
            version: 1,
            id: "openai-chat".into(),
            name: "OpenAI Chat Completion".into(),
            method: "POST".into(),
            url: "https://api.openai.com/v1/chat/completions".into(),
            auth: AuthConfig::Bearer {
                token_env: "OPENAI_API_KEY".into(),
            },
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
                result_columns: Vec::new(),
                bind: None,
            },
            timeout_seconds: 50,
            adapter: Default::default(),
            tag: None,
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
    fn load_all_skips_malformed_files() {
        let dir = TempDir::new().unwrap();
        save(dir.path(), &sample_request()).unwrap();
        atomic_write(
            &dir.path().join("broken.yaml"),
            b"this: is: not: valid: yaml: at: all\n",
        )
        .unwrap();
        atomic_write(
            &dir.path().join("missing-auth.yaml"),
            b"version: 1\nid: x\nname: x\nmethod: GET\nurl: http://x\n",
        )
        .unwrap();

        let map = load_all(dir.path()).unwrap();

        assert_eq!(map.len(), 1);
        assert!(map.contains_key("openai-chat"));
        assert!(!map.contains_key("broken"));
        assert!(!map.contains_key("missing-auth"));
    }

    #[test]
    fn bundled_requests_parse() {
        let requests_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../requests");

        let map = load_all(&requests_dir).unwrap();

        assert!(map.contains_key("ollama-chat-local"));
    }

    #[test]
    fn references_finds_target_and_scenario() {
        let root = TempDir::new().unwrap();
        let targets_dir = root.path().join("targets");
        let scenarios_dir = root.path().join("scenarios");
        std::fs::create_dir_all(&targets_dir).unwrap();
        std::fs::create_dir_all(&scenarios_dir).unwrap();

        let target = Target {
            version: 1,
            id: "acme".into(),
            name: "Acme staging".into(),
            request_ids: vec!["openai-chat".into(), "other".into()],
            request_id: "openai-chat".into(),
            session_config: Default::default(),
            auth_acquisition: Default::default(),
            notes: None,
        };
        crate::targets::save(&targets_dir, &target).unwrap();

        let scenario = Scenario {
            version: 1,
            id: "flow-1".into(),
            name: "Flow 1".into(),
            repeat: 1,
            description: None,
            request_ids: vec!["openai-chat".into()],
            library: None,
            shared_session: false,
        };
        crate::scenarios::save(&scenarios_dir, &scenario).unwrap();

        let refs = references(&targets_dir, &scenarios_dir, "openai-chat").unwrap();
        assert_eq!(refs.len(), 2);
        assert!(matches!(&refs[0], RequestReference::Target { id, .. } if id == "acme"));
        assert!(matches!(
            &refs[1],
            RequestReference::Scenario { id, .. } if id == "flow-1"
        ));

        let none = references(&targets_dir, &scenarios_dir, "nonexistent").unwrap();
        assert!(none.is_empty());
    }

    #[test]
    fn references_tolerates_missing_dirs() {
        let root = TempDir::new().unwrap();
        let targets_dir = root.path().join("targets");
        let scenarios_dir = root.path().join("scenarios");
        // dirs do not exist
        let refs = references(&targets_dir, &scenarios_dir, "anything").unwrap();
        assert!(refs.is_empty());
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
