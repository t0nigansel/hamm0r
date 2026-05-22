//! Phase 2 of `docs/RefactorPlan.md`: request dependencies (auth chains).
//!
//! A Request can declare a bind (`response.bind: <name>`) that names the
//! value extracted from its response. Other Requests reference that bind
//! via `{{<request_id>.<bind_name>}}` interpolation in any string field
//! (URL, headers, body). The runner builds a DAG from these references,
//! topologically sorts it, fires prerequisites first, captures bound
//! values, and substitutes them into dependents at fire time.
//!
//! This module is the resolver primitive. It does not fire HTTP requests â€”
//! that's the runner's job. Tests cover dependency extraction, topological
//! ordering, cycle detection, and missing-bind detection.

use std::collections::{HashMap, HashSet};

use bytes::Bytes;
use storage::types::{AuthConfig, BodyFormat, ExtractConfig, Request};

use crate::adapter::{self, AdapterResponse};
use crate::error::RunnerError;
use crate::session::SessionStrategy;
use crate::template::BindCache;

/// Reference to another Request's bound value: `{{<request_id>.<bind>}}`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct BindRef {
    pub request_id: String,
    pub bind: String,
}

/// Scan one template string for `{{<id>.<bind>}}` references. Tolerates
/// surrounding whitespace and minijinja filter/expression suffixes
/// (`{{ login.token | upper }}`). The bare `{{prompt}}` is intentionally
/// not matched here â€” it's not a dep.
pub fn extract_refs_from_template(text: &str) -> Vec<BindRef> {
    // Match a `{{ ... }}` block, then look for `<id>.<bind>` pairs inside
    // its body. Compiled once per process via OnceLock â€” no new dep.
    use std::sync::OnceLock;
    static BLOCK_RE: OnceLock<regex::Regex> = OnceLock::new();
    static PAIR_RE: OnceLock<regex::Regex> = OnceLock::new();
    let block_re = BLOCK_RE.get_or_init(|| regex::Regex::new(r"\{\{([^}]*)\}\}").unwrap());
    let pair_re = PAIR_RE.get_or_init(|| {
        regex::Regex::new(r"([a-zA-Z][a-zA-Z0-9_\-]*)\.([a-zA-Z][a-zA-Z0-9_\-]*)").unwrap()
    });

    let mut out = Vec::new();
    for caps in block_re.captures_iter(text) {
        let body = caps.get(1).map(|m| m.as_str()).unwrap_or("");
        for pair in pair_re.captures_iter(body) {
            let id = pair.get(1).unwrap().as_str().to_owned();
            let bind = pair.get(2).unwrap().as_str().to_owned();
            // Skip the literal `prompt.X` shape â€” `prompt` is reserved for
            // payload interpolation, not a Request id.
            if id == "prompt" {
                continue;
            }
            out.push(BindRef {
                request_id: id,
                bind,
            });
        }
    }
    out
}

/// Extract every `{{<id>.<bind>}}` reference from a Request's
/// substitutable surface: URL, headers (keys + values), and body content
/// (when it's a string or JSON value with strings inside).
pub fn extract_request_dependencies(req: &Request) -> Vec<BindRef> {
    let mut refs = Vec::new();

    refs.extend(extract_refs_from_template(&req.url));
    for (k, v) in &req.headers {
        refs.extend(extract_refs_from_template(k));
        refs.extend(extract_refs_from_template(v));
    }

    // Auth env-var fields are env names, not templates, so we don't scan
    // them. Keep that contract â€” secrets stay in env / keychain.
    let _ = matches!(req.auth, AuthConfig::None);

    match &req.body.format {
        BodyFormat::Json | BodyFormat::Form => {
            refs.extend(scan_json_value(&req.body.content));
        }
        BodyFormat::Text | BodyFormat::Raw => {
            if let Some(s) = req.body.content.as_str() {
                refs.extend(extract_refs_from_template(s));
            }
        }
    }

    // Dedupe while preserving order â€” same dep referenced N times
    // shouldn't fire N times.
    let mut seen = HashSet::new();
    refs.retain(|r| seen.insert(r.clone()));
    refs
}

fn scan_json_value(v: &serde_json::Value) -> Vec<BindRef> {
    let mut out = Vec::new();
    match v {
        serde_json::Value::String(s) => out.extend(extract_refs_from_template(s)),
        serde_json::Value::Array(arr) => {
            for item in arr {
                out.extend(scan_json_value(item));
            }
        }
        serde_json::Value::Object(map) => {
            for (k, child) in map {
                out.extend(extract_refs_from_template(k));
                out.extend(scan_json_value(child));
            }
        }
        _ => {}
    }
    out
}

/// Compute the firing order for a set of target Request ids against a
/// registry of all known Requests.
///
/// Returns the list of Request ids in topological order â€” every Request
/// fires after its dependencies. The targets themselves are at the end.
/// Errors:
/// - cycle in the dependency graph
/// - reference to an unknown Request id
/// - reference to a bind that the producing Request doesn't declare
pub fn topological_order(
    targets: &[String],
    registry: &HashMap<String, Request>,
) -> Result<Vec<String>, RunnerError> {
    // BFS-collect every transitively-referenced Request id, then run
    // Kahn's algorithm for the topological sort.

    let mut needed: HashSet<String> = HashSet::new();
    let mut frontier: Vec<String> = targets.to_vec();
    while let Some(id) = frontier.pop() {
        if !needed.insert(id.clone()) {
            continue;
        }
        let Some(req) = registry.get(&id) else {
            return Err(RunnerError::Extraction {
                reason: format!("dependency references unknown Request '{id}'"),
            });
        };
        for r in extract_request_dependencies(req) {
            // Validate the bind exists on the producing Request.
            let producer = registry
                .get(&r.request_id)
                .ok_or_else(|| RunnerError::Extraction {
                    reason: format!(
                        "Request '{}' references unknown Request '{}'",
                        id, r.request_id
                    ),
                })?;
            let declared = producer.response.bind.as_deref();
            if declared != Some(r.bind.as_str()) {
                return Err(RunnerError::Extraction {
                    reason: format!(
                        "Request '{}' references {{{{{}.{}}}}} but Request '{}' does not bind '{}' (declared: {:?})",
                        id, r.request_id, r.bind, r.request_id, r.bind, declared,
                    ),
                });
            }
            frontier.push(r.request_id);
        }
    }

    // Build adjacency: edge dep -> dependent. In-degree counts deps per node.
    let mut adj: HashMap<String, Vec<String>> = HashMap::new();
    let mut indeg: HashMap<String, usize> = HashMap::new();
    for id in &needed {
        indeg.entry(id.clone()).or_insert(0);
        let req = registry.get(id).expect("collected above");
        for r in extract_request_dependencies(req) {
            adj.entry(r.request_id.clone())
                .or_default()
                .push(id.clone());
            *indeg.entry(id.clone()).or_insert(0) += 1;
        }
    }

    // Kahn's algorithm.
    let mut queue: Vec<String> = indeg
        .iter()
        .filter_map(|(k, v)| if *v == 0 { Some(k.clone()) } else { None })
        .collect();
    queue.sort(); // deterministic order between independent nodes
    let mut order = Vec::with_capacity(needed.len());
    while let Some(node) = queue.pop() {
        order.push(node.clone());
        if let Some(succs) = adj.remove(&node) {
            for s in succs {
                let count = indeg.get_mut(&s).expect("counted above");
                *count -= 1;
                if *count == 0 {
                    queue.push(s);
                }
            }
        }
    }
    if order.len() != needed.len() {
        let cycle: Vec<&String> = indeg
            .iter()
            .filter_map(|(k, v)| if *v > 0 { Some(k) } else { None })
            .collect();
        return Err(RunnerError::Extraction {
            reason: format!(
                "cyclic Request dependencies among: {}",
                cycle.into_iter().cloned().collect::<Vec<_>>().join(", ")
            ),
        });
    }
    Ok(order)
}

/// Outcome of firing one Request through the chain â€” the response plus
/// any prerequisite firings (each as a separate `AdapterResponse`) so the
/// caller can persist them into the run JSONL with the appropriate
/// `kind: "prerequisite"` marker.
#[derive(Debug)]
pub struct ChainOutcome {
    /// Per-prerequisite firing, in topological order.
    pub prerequisites: Vec<(String, AdapterResponse)>,
    /// The target Request's response.
    pub target: AdapterResponse,
}

/// Fire `target_id` against `http`, firing every prerequisite Request first.
///
/// The starting `bind_cache` is consumed and updated. When `shared_session`
/// is enabled at the caller level, the same cache should be passed across
/// attempts so prerequisites only fire once per run; when disabled, callers
/// should pass a fresh empty cache for each attempt.
pub async fn fire_chain(
    http: &reqwest::Client,
    registry: &HashMap<String, Request>,
    target_id: &str,
    payload: &str,
    session_strategy: &SessionStrategy,
    session_value: &str,
    bind_cache: &mut BindCache,
) -> Result<ChainOutcome, RunnerError> {
    let order = topological_order(&[target_id.to_owned()], registry)?;
    let mut prerequisites = Vec::new();

    for id in &order {
        let req = registry.get(id).expect("topo includes only known ids");
        let already_bound = req
            .response
            .bind
            .as_deref()
            .map(|name| {
                bind_cache
                    .get(id)
                    .map(|m| m.contains_key(name))
                    .unwrap_or(false)
            })
            .unwrap_or(false);

        // Target itself fires below regardless.
        if id == target_id {
            continue;
        }

        // If we already have this Request's bind cached (from an earlier
        // attempt in shared_session mode), skip the call.
        if already_bound {
            continue;
        }

        // Prerequisites pass an empty payload â€” they typically don't use
        // `{{prompt}}`, and giving them the attack payload would conflate
        // attack-side input with auth-side state.
        let resp = adapter::execute_with_session_and_binds(
            http,
            req,
            "",
            session_strategy,
            session_value,
            bind_cache,
        )
        .await?;
        capture_bind(req, &resp, bind_cache)?;
        prerequisites.push((id.clone(), resp));
    }

    let target_req = registry
        .get(target_id)
        .ok_or_else(|| RunnerError::Extraction {
            reason: format!("unknown target Request '{target_id}'"),
        })?;
    let target = adapter::execute_with_session_and_binds(
        http,
        target_req,
        payload,
        session_strategy,
        session_value,
        bind_cache,
    )
    .await?;
    // The target may also declare a bind (e.g. when the target itself is a
    // prereq for a later step). Capture it too for completeness.
    capture_bind(target_req, &target, bind_cache)?;

    Ok(ChainOutcome {
        prerequisites,
        target,
    })
}

fn capture_bind(
    req: &Request,
    resp: &AdapterResponse,
    bind_cache: &mut BindCache,
) -> Result<(), RunnerError> {
    let Some(name) = &req.response.bind else {
        return Ok(());
    };

    // Re-extract the value matching the Request's `extract` config. The
    // adapter already extracted once into `resp.extracted`, but to be
    // explicit about which value is being bound we extract again from
    // the raw bytes here. Cheap, and keeps the contract self-contained.
    let value = extract_bound_value(&resp.body_bytes, &req.response.extract)?;
    let value = value.ok_or_else(|| RunnerError::Extraction {
        reason: format!(
            "Request '{}' bound '{}' but extract returned no value",
            req.id, name
        ),
    })?;

    bind_cache
        .entry(req.id.clone())
        .or_default()
        .insert(name.clone(), value);
    Ok(())
}

fn extract_bound_value(
    body: &Bytes,
    extract: &ExtractConfig,
) -> Result<Option<String>, RunnerError> {
    match extract {
        ExtractConfig::Raw => Ok(Some(String::from_utf8_lossy(body).into_owned())),
        ExtractConfig::Jsonpath { path } => {
            let json: serde_json::Value =
                serde_json::from_slice(body).map_err(|e| RunnerError::Extraction {
                    reason: format!("body is not JSON: {e}"),
                })?;
            let jp =
                serde_json_path::JsonPath::parse(path).map_err(|e| RunnerError::Extraction {
                    reason: format!("invalid JSONPath '{path}': {e}"),
                })?;
            let hit = jp.query(&json).first().cloned();
            Ok(hit.map(|v| match v {
                serde_json::Value::String(s) => s,
                other => other.to_string(),
            }))
        }
        ExtractConfig::Regex { pattern } => {
            let re = regex::Regex::new(pattern).map_err(|e| RunnerError::Extraction {
                reason: format!("invalid regex '{pattern}': {e}"),
            })?;
            let text = String::from_utf8_lossy(body);
            Ok(re
                .captures(&text)
                .and_then(|c| c.get(1).or_else(|| c.get(0)))
                .map(|m| m.as_str().to_owned()))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use storage::types::{AdapterType, BodyConfig, ExtractConfig, ResponseConfig};

    fn req(
        id: &str,
        url: &str,
        headers: &[(&str, &str)],
        body_text: &str,
        bind: Option<&str>,
    ) -> Request {
        Request {
            version: 1,
            id: id.into(),
            name: id.into(),
            method: "POST".into(),
            url: url.into(),
            auth: AuthConfig::None,
            headers: headers
                .iter()
                .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
                .collect(),
            body: BodyConfig {
                format: BodyFormat::Json,
                content: serde_json::json!({ "msg": body_text }),
            },
            response: ResponseConfig {
                extract: ExtractConfig::Raw,
                result_columns: Vec::new(),
                bind: bind.map(str::to_owned),
            },
            timeout_seconds: 30,
            adapter: AdapterType::CustomRest,
            tag: None,
            test_payload: None,
        }
    }

    #[test]
    fn extracts_simple_bind_ref_from_header() {
        let r = req(
            "chat",
            "https://x.test/y",
            &[("Authorization", "Bearer {{login.bearer_token}}")],
            "{{prompt}}",
            None,
        );
        let deps = extract_request_dependencies(&r);
        assert_eq!(
            deps,
            vec![BindRef {
                request_id: "login".into(),
                bind: "bearer_token".into(),
            }]
        );
    }

    #[test]
    fn ignores_bare_prompt_placeholder() {
        let r = req("chat", "https://x.test/y", &[], "say {{prompt}} now", None);
        assert!(extract_request_dependencies(&r).is_empty());
    }

    #[test]
    fn tolerates_jinja_whitespace_and_filters() {
        let s = "{{ login.bearer_token | upper }} and {{login.session}}";
        let refs = extract_refs_from_template(s);
        assert_eq!(
            refs,
            vec![
                BindRef {
                    request_id: "login".into(),
                    bind: "bearer_token".into(),
                },
                BindRef {
                    request_id: "login".into(),
                    bind: "session".into(),
                },
            ]
        );
    }

    #[test]
    fn topological_order_simple_chain() {
        let mut reg = HashMap::new();
        reg.insert(
            "login".into(),
            req(
                "login",
                "https://x.test/login",
                &[],
                "creds",
                Some("bearer_token"),
            ),
        );
        reg.insert(
            "chat".into(),
            req(
                "chat",
                "https://x.test/chat",
                &[("Authorization", "Bearer {{login.bearer_token}}")],
                "{{prompt}}",
                None,
            ),
        );
        let order = topological_order(&["chat".into()], &reg).unwrap();
        assert_eq!(order, vec!["login".to_string(), "chat".to_string()]);
    }

    #[test]
    fn topological_order_two_deep_chain() {
        let mut reg = HashMap::new();
        reg.insert(
            "login".into(),
            req("login", "https://x.test/login", &[], "creds", Some("token")),
        );
        reg.insert(
            "bootstrap".into(),
            req(
                "bootstrap",
                "https://x.test/boot",
                &[("Authorization", "Bearer {{login.token}}")],
                "go",
                Some("session"),
            ),
        );
        reg.insert(
            "chat".into(),
            req(
                "chat",
                "https://x.test/chat",
                &[
                    ("Authorization", "Bearer {{login.token}}"),
                    ("X-Session", "{{bootstrap.session}}"),
                ],
                "{{prompt}}",
                None,
            ),
        );
        let order = topological_order(&["chat".into()], &reg).unwrap();
        // login must come before bootstrap, both before chat.
        let pos = |id: &str| order.iter().position(|s| s == id).unwrap();
        assert!(pos("login") < pos("bootstrap"));
        assert!(pos("bootstrap") < pos("chat"));
    }

    #[test]
    fn cycle_is_detected() {
        let mut reg = HashMap::new();
        // a -> b -> a
        reg.insert(
            "a".into(),
            req(
                "a",
                "https://x.test/a",
                &[("X", "{{b.token}}")],
                "go",
                Some("token"),
            ),
        );
        reg.insert(
            "b".into(),
            req(
                "b",
                "https://x.test/b",
                &[("X", "{{a.token}}")],
                "go",
                Some("token"),
            ),
        );
        let err = topological_order(&["a".into()], &reg).expect_err("cycle must be rejected");
        assert!(format!("{err}").to_lowercase().contains("cycl"));
    }

    #[test]
    fn unknown_request_id_in_reference_errors() {
        let mut reg = HashMap::new();
        reg.insert(
            "chat".into(),
            req(
                "chat",
                "https://x.test/chat",
                &[("Authorization", "Bearer {{ghost.token}}")],
                "{{prompt}}",
                None,
            ),
        );
        let err =
            topological_order(&["chat".into()], &reg).expect_err("unknown id must be rejected");
        let msg = format!("{err}").to_lowercase();
        assert!(msg.contains("unknown") && msg.contains("ghost"));
    }

    #[test]
    fn reference_to_undeclared_bind_errors() {
        let mut reg = HashMap::new();
        // login does NOT declare a bind, but chat references one.
        reg.insert(
            "login".into(),
            req("login", "https://x.test/login", &[], "creds", None),
        );
        reg.insert(
            "chat".into(),
            req(
                "chat",
                "https://x.test/chat",
                &[("Authorization", "Bearer {{login.token}}")],
                "{{prompt}}",
                None,
            ),
        );
        let err = topological_order(&["chat".into()], &reg)
            .expect_err("unbound reference must be rejected");
        let msg = format!("{err}");
        assert!(msg.contains("does not bind"));
    }
}
