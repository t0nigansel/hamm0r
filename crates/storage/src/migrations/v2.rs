//! Phase 2 migrations from `docs/RefactorPlan.md`.
//!
//! Two migrations live here:
//!
//! 1. `tag_requests_from_targets` — copy each Target's `name` into the
//!    `tag` field of every Request the Target references. The `tag` is
//!    the new grouping primitive that replaces Target as a UI organization
//!    concept.
//! 2. `synthesize_auth_chain_requests` — translate each Target's
//!    `auth_acquisition.http_login` configuration into a real Request
//!    that other Requests reference via `{{<id>.bearer_token}}`. Q-H
//!    resolved that the new flow fires login per run (no keychain
//!    caching), which lets us migrate without preserving the historical
//!    once-per-install semantics. The runner's template engine learned
//!    `{{ env.VAR }}` substitution so the synthesized Request can read
//!    `login_env` / `password_env` credentials at fire time.
//!
//! Both migrations are idempotent: re-running on already-migrated data
//! is a no-op.

use std::collections::HashMap;
use std::path::Path;

use anyhow::Context as _;

use crate::types::{
    AuthConfig, BodyConfig, BodyFormat, ExtractConfig, HttpLoginConfig, Request, ResponseConfig,
};
use crate::{requests, targets};

/// Result of running `tag_requests_from_targets`.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct TagMigrationReport {
    /// Number of Request files that received a new `tag` value.
    pub tagged: usize,
    /// Number of Requests already tagged that we left alone.
    pub already_tagged: usize,
    /// Targets we encountered that pointed at unknown Request ids.
    pub orphan_target_refs: usize,
}

/// Set `Request.tag = Target.name` for every Request a Target references,
/// **only when** the Request currently has no tag. Existing tags are
/// preserved (the user may have set them manually, or a previous run of
/// this migration already did its work).
///
/// Idempotent: safe to run on every startup. The first run does the work;
/// subsequent runs report `tagged: 0` and `already_tagged: <count>`.
pub fn tag_requests_from_targets(
    targets_dir: &Path,
    requests_dir: &Path,
) -> anyhow::Result<TagMigrationReport> {
    let mut report = TagMigrationReport::default();

    if !targets_dir.exists() || !requests_dir.exists() {
        return Ok(report);
    }

    let all_targets = targets::load_all(targets_dir)
        .with_context(|| format!("loading targets from {}", targets_dir.display()))?;
    let mut all_requests = requests::load_all(requests_dir)
        .with_context(|| format!("loading requests from {}", requests_dir.display()))?;

    for target in all_targets.values() {
        let tag = target.name.trim();
        if tag.is_empty() {
            continue;
        }

        // Each Target references its primary plus any additional request_ids.
        let referenced_ids: Vec<String> = std::iter::once(target.request_id.clone())
            .chain(target.request_ids.iter().cloned())
            .filter(|id| !id.trim().is_empty())
            .collect();

        for req_id in referenced_ids {
            let Some(req) = all_requests.get_mut(&req_id) else {
                report.orphan_target_refs += 1;
                continue;
            };

            if req.tag.is_some() {
                report.already_tagged += 1;
                continue;
            }

            req.tag = Some(tag.to_owned());
            requests::save(requests_dir, req).with_context(|| {
                format!(
                    "saving tagged request '{}' to {}",
                    req.id,
                    requests_dir.display()
                )
            })?;
            report.tagged += 1;
        }
    }

    Ok(report)
}

/// Result of running `synthesize_auth_chain_requests`.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct AuthChainMigrationReport {
    /// Number of new login Requests written.
    pub login_requests_synthesized: usize,
    /// Number of Targets we skipped because their login Request already
    /// existed (idempotency path) or the Target had no `http_login`.
    pub targets_skipped: usize,
    /// Number of chat Requests we updated to reference the new login
    /// Request via `{{<id>.bearer_token}}`.
    pub chat_requests_wired: usize,
    /// Number of chat Requests left untouched because they already had
    /// an `Authorization` header (we don't overwrite manual config).
    pub chat_requests_already_wired: usize,
}

/// Translate every Target's `auth_acquisition.http_login` configuration
/// into a standalone login Request that the chat Requests then reference
/// via `{{<login_id>.bearer_token}}`.
///
/// Per Q-H: per-run login is acceptable, so each engagement run will
/// re-fire the login chain. No keychain caching is preserved.
///
/// The synthesized login Request:
/// - id: `<target.id>__login` (deterministic; idempotent re-run skips it)
/// - body: `Raw` with the original `body_template`, with `${VAR}` blocks
///   rewritten to `{{ env.VAR }}` so the runner's renderer can resolve
///   credentials from the process environment at fire time
/// - response: `Jsonpath` extract on `token_json_path` (defaulting to
///   `$.access_token`) and a `bearer_token` bind
///
/// Each chat Request the Target references gets an `Authorization:
/// Bearer {{<login_id>.bearer_token}}` header iff it doesn't already
/// have an `Authorization` header. We never overwrite manual config.
pub fn synthesize_auth_chain_requests(
    targets_dir: &Path,
    requests_dir: &Path,
) -> anyhow::Result<AuthChainMigrationReport> {
    let mut report = AuthChainMigrationReport::default();

    if !targets_dir.exists() || !requests_dir.exists() {
        return Ok(report);
    }

    let all_targets = targets::load_all(targets_dir)
        .with_context(|| format!("loading targets from {}", targets_dir.display()))?;
    let mut all_requests = requests::load_all(requests_dir)
        .with_context(|| format!("loading requests from {}", requests_dir.display()))?;

    for target in all_targets.values() {
        let Some(http_login) = target.auth_acquisition.http_login.as_ref() else {
            report.targets_skipped += 1;
            continue;
        };

        let login_id = format!("{}__login", target.id);

        // Idempotency: if a synthesized login Request already exists,
        // skip the Target entirely. We still examine its chat Requests
        // below so a partial earlier run can finish wiring.
        let synthesized_now = if all_requests.contains_key(&login_id) {
            report.targets_skipped += 1;
            false
        } else {
            let login_req = build_login_request(&login_id, target.name.as_str(), http_login);
            requests::save(requests_dir, &login_req).with_context(|| {
                format!(
                    "saving synthesized login Request '{login_id}' to {}",
                    requests_dir.display()
                )
            })?;
            all_requests.insert(login_id.clone(), login_req);
            report.login_requests_synthesized += 1;
            true
        };

        // Wire chat Requests. We always do this (whether we just synthesized
        // the login or it was already there) so a half-finished previous
        // migration completes on re-run.
        let _ = synthesized_now;
        let referenced_ids: Vec<String> = std::iter::once(target.request_id.clone())
            .chain(target.request_ids.iter().cloned())
            .filter(|id| !id.trim().is_empty() && id != &login_id)
            .collect();

        for chat_id in referenced_ids {
            let Some(chat_req) = all_requests.get_mut(&chat_id) else {
                continue;
            };

            if has_authorization_header(&chat_req.headers) {
                report.chat_requests_already_wired += 1;
                continue;
            }

            chat_req.headers.insert(
                "Authorization".to_owned(),
                format!("Bearer {{{{ {login_id}.bearer_token }}}}"),
            );
            requests::save(requests_dir, chat_req).with_context(|| {
                format!(
                    "saving chat Request '{}' after wiring auth chain to {}",
                    chat_req.id,
                    requests_dir.display()
                )
            })?;
            report.chat_requests_wired += 1;
        }
    }

    Ok(report)
}

fn build_login_request(login_id: &str, target_name: &str, http_login: &HttpLoginConfig) -> Request {
    let url = translate_env_placeholders(http_login.url.as_deref().unwrap_or(""));
    let method = http_login
        .method
        .as_deref()
        .unwrap_or("POST")
        .to_uppercase();

    let headers: HashMap<String, String> = http_login
        .headers
        .iter()
        .map(|(k, v)| (k.clone(), translate_env_placeholders(v)))
        .collect();

    let body_str = translate_env_placeholders(http_login.body_template.as_deref().unwrap_or(""));
    let body = BodyConfig {
        format: BodyFormat::Raw,
        content: serde_json::Value::String(body_str),
    };

    let token_path = http_login
        .token_json_path
        .clone()
        .unwrap_or_else(|| "$.access_token".to_owned());

    let response = ResponseConfig {
        extract: ExtractConfig::Jsonpath { path: token_path },
        bind: Some("bearer_token".to_owned()),
    };

    Request {
        version: 1,
        id: login_id.to_owned(),
        name: format!("{} login", target_name.trim()),
        method,
        url,
        auth: AuthConfig::None,
        headers,
        body,
        response,
        timeout_seconds: http_login.timeout_seconds.unwrap_or(30),
        adapter: Default::default(),
        tag: Some(target_name.to_owned()),
    }
}

/// Rewrite `${VAR}` to `{{ env.VAR }}` so the runner's minijinja env can
/// resolve it. Only matches the conservative `[A-Za-z_][A-Za-z0-9_]*`
/// shell-style identifier shape; other `$`-led sequences pass through
/// untouched.
fn translate_env_placeholders(input: &str) -> String {
    use std::sync::OnceLock;
    static RE: OnceLock<regex::Regex> = OnceLock::new();
    let re = RE
        .get_or_init(|| regex::Regex::new(r"\$\{([A-Za-z_][A-Za-z0-9_]*)\}").expect("valid regex"));
    re.replace_all(input, "{{ env.$1 }}").into_owned()
}

fn has_authorization_header(headers: &HashMap<String, String>) -> bool {
    headers
        .keys()
        .any(|k| k.eq_ignore_ascii_case("authorization"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{
        AuthConfig, BodyConfig, BodyFormat, ExtractConfig, Request, ResponseConfig, Target,
    };
    use std::collections::HashMap;
    use tempfile::TempDir;

    fn sample_request(id: &str) -> Request {
        Request {
            version: 1,
            id: id.into(),
            name: id.into(),
            method: "POST".into(),
            url: "https://example.test/x".into(),
            auth: AuthConfig::None,
            headers: HashMap::new(),
            body: BodyConfig {
                format: BodyFormat::Json,
                content: serde_json::json!({}),
            },
            response: ResponseConfig {
                extract: ExtractConfig::Raw,
                bind: None,
            },
            timeout_seconds: 30,
            adapter: Default::default(),
            tag: None,
        }
    }

    fn sample_target(id: &str, name: &str, primary: &str, others: &[&str]) -> Target {
        Target {
            version: 1,
            id: id.into(),
            name: name.into(),
            request_ids: others.iter().map(|s| (*s).to_owned()).collect(),
            request_id: primary.into(),
            session_config: Default::default(),
            auth_acquisition: Default::default(),
            notes: None,
        }
    }

    fn write_target(dir: &Path, t: &Target) {
        targets::save(dir, t).unwrap();
    }
    fn write_request(dir: &Path, r: &Request) {
        requests::save(dir, r).unwrap();
    }

    #[test]
    fn tags_primary_and_secondary_requests() {
        let root = TempDir::new().unwrap();
        let targets_dir = root.path().join("targets");
        let requests_dir = root.path().join("requests");
        std::fs::create_dir_all(&targets_dir).unwrap();
        std::fs::create_dir_all(&requests_dir).unwrap();

        write_request(&requests_dir, &sample_request("primary"));
        write_request(&requests_dir, &sample_request("secondary"));
        write_target(
            &targets_dir,
            &sample_target("acme", "Acme staging", "primary", &["secondary"]),
        );

        let report = tag_requests_from_targets(&targets_dir, &requests_dir).unwrap();
        assert_eq!(report.tagged, 2);
        assert_eq!(report.already_tagged, 0);
        assert_eq!(report.orphan_target_refs, 0);

        let reqs = requests::load_all(&requests_dir).unwrap();
        assert_eq!(reqs["primary"].tag.as_deref(), Some("Acme staging"));
        assert_eq!(reqs["secondary"].tag.as_deref(), Some("Acme staging"));
    }

    #[test]
    fn idempotent_second_run_is_a_noop() {
        let root = TempDir::new().unwrap();
        let targets_dir = root.path().join("targets");
        let requests_dir = root.path().join("requests");
        std::fs::create_dir_all(&targets_dir).unwrap();
        std::fs::create_dir_all(&requests_dir).unwrap();

        write_request(&requests_dir, &sample_request("primary"));
        write_target(
            &targets_dir,
            &sample_target("acme", "Acme staging", "primary", &[]),
        );

        let first = tag_requests_from_targets(&targets_dir, &requests_dir).unwrap();
        assert_eq!(first.tagged, 1);

        let second = tag_requests_from_targets(&targets_dir, &requests_dir).unwrap();
        assert_eq!(second.tagged, 0);
        assert_eq!(second.already_tagged, 1);
    }

    #[test]
    fn does_not_overwrite_user_set_tag() {
        let root = TempDir::new().unwrap();
        let targets_dir = root.path().join("targets");
        let requests_dir = root.path().join("requests");
        std::fs::create_dir_all(&targets_dir).unwrap();
        std::fs::create_dir_all(&requests_dir).unwrap();

        let mut already_tagged = sample_request("manual");
        already_tagged.tag = Some("user-chosen".into());
        write_request(&requests_dir, &already_tagged);
        write_target(
            &targets_dir,
            &sample_target("acme", "Acme staging", "manual", &[]),
        );

        let report = tag_requests_from_targets(&targets_dir, &requests_dir).unwrap();
        assert_eq!(report.tagged, 0);
        assert_eq!(report.already_tagged, 1);

        let reqs = requests::load_all(&requests_dir).unwrap();
        assert_eq!(reqs["manual"].tag.as_deref(), Some("user-chosen"));
    }

    #[test]
    fn multi_target_uses_first_winning_target_name() {
        // Two Targets, both pointing at the same Request. The first one
        // tags it; the second hits the already_tagged path. We don't
        // guarantee which Target wins (HashMap iteration order is
        // unspecified) — just that the field is set and consistent.
        let root = TempDir::new().unwrap();
        let targets_dir = root.path().join("targets");
        let requests_dir = root.path().join("requests");
        std::fs::create_dir_all(&targets_dir).unwrap();
        std::fs::create_dir_all(&requests_dir).unwrap();

        write_request(&requests_dir, &sample_request("shared"));
        write_target(&targets_dir, &sample_target("a", "Alpha", "shared", &[]));
        write_target(&targets_dir, &sample_target("b", "Beta", "shared", &[]));

        let report = tag_requests_from_targets(&targets_dir, &requests_dir).unwrap();
        assert_eq!(report.tagged, 1);
        assert_eq!(report.already_tagged, 1);

        let reqs = requests::load_all(&requests_dir).unwrap();
        let tag = reqs["shared"].tag.as_deref().unwrap();
        assert!(tag == "Alpha" || tag == "Beta", "got: {tag}");
    }

    #[test]
    fn orphan_request_refs_are_counted_not_fatal() {
        let root = TempDir::new().unwrap();
        let targets_dir = root.path().join("targets");
        let requests_dir = root.path().join("requests");
        std::fs::create_dir_all(&targets_dir).unwrap();
        std::fs::create_dir_all(&requests_dir).unwrap();

        // Target points at a Request id that has no file.
        write_target(
            &targets_dir,
            &sample_target("a", "Alpha", "ghost", &["also-ghost"]),
        );

        let report = tag_requests_from_targets(&targets_dir, &requests_dir).unwrap();
        assert_eq!(report.tagged, 0);
        assert_eq!(report.orphan_target_refs, 2);
    }

    #[test]
    fn empty_target_name_is_skipped() {
        let root = TempDir::new().unwrap();
        let targets_dir = root.path().join("targets");
        let requests_dir = root.path().join("requests");
        std::fs::create_dir_all(&targets_dir).unwrap();
        std::fs::create_dir_all(&requests_dir).unwrap();

        write_request(&requests_dir, &sample_request("r1"));
        write_target(&targets_dir, &sample_target("nameless", "   ", "r1", &[]));

        let report = tag_requests_from_targets(&targets_dir, &requests_dir).unwrap();
        assert_eq!(report.tagged, 0);

        let reqs = requests::load_all(&requests_dir).unwrap();
        assert_eq!(reqs["r1"].tag, None);
    }

    #[test]
    fn missing_dirs_are_tolerated() {
        let root = TempDir::new().unwrap();
        let targets_dir = root.path().join("targets");
        let requests_dir = root.path().join("requests");
        // Neither dir exists.
        let report = tag_requests_from_targets(&targets_dir, &requests_dir).unwrap();
        assert_eq!(report, TagMigrationReport::default());
    }

    // ── Auth-chain synthesis tests ───────────────────────────────────

    use crate::types::{AuthAcquisitionConfig, AuthAcquisitionMode, HttpLoginConfig};

    fn target_with_http_login(
        id: &str,
        name: &str,
        primary: &str,
        others: &[&str],
        http_login: HttpLoginConfig,
    ) -> Target {
        Target {
            version: 1,
            id: id.into(),
            name: name.into(),
            request_ids: others.iter().map(|s| (*s).to_owned()).collect(),
            request_id: primary.into(),
            session_config: Default::default(),
            auth_acquisition: AuthAcquisitionConfig {
                mode: AuthAcquisitionMode::HttpLogin,
                http_login: Some(http_login),
            },
            notes: None,
        }
    }

    fn standard_http_login() -> HttpLoginConfig {
        HttpLoginConfig {
            login_env: Some("LOGIN_USERNAME".into()),
            password_env: Some("LOGIN_PASSWORD".into()),
            url: Some("https://example.test/auth/login".into()),
            method: Some("post".into()),
            headers: [("Content-Type".into(), "application/json".into())].into(),
            body_template: Some(
                r#"{"username":"${LOGIN_USERNAME}","password":"${LOGIN_PASSWORD}"}"#.into(),
            ),
            token_json_path: Some("$.access_token".into()),
            timeout_seconds: Some(15),
        }
    }

    #[test]
    fn synthesizes_login_request_and_wires_chat_request() {
        let root = TempDir::new().unwrap();
        let targets_dir = root.path().join("targets");
        let requests_dir = root.path().join("requests");
        std::fs::create_dir_all(&targets_dir).unwrap();
        std::fs::create_dir_all(&requests_dir).unwrap();

        write_request(&requests_dir, &sample_request("chat"));
        write_target(
            &targets_dir,
            &target_with_http_login("acme", "Acme", "chat", &[], standard_http_login()),
        );

        let report = synthesize_auth_chain_requests(&targets_dir, &requests_dir).unwrap();
        assert_eq!(report.login_requests_synthesized, 1);
        assert_eq!(report.chat_requests_wired, 1);
        assert_eq!(report.chat_requests_already_wired, 0);

        let reqs = requests::load_all(&requests_dir).unwrap();
        let login = reqs.get("acme__login").expect("login request synthesized");
        assert_eq!(login.method, "POST");
        assert_eq!(login.url, "https://example.test/auth/login");
        assert_eq!(login.timeout_seconds, 15);
        assert_eq!(login.tag.as_deref(), Some("Acme"));
        assert_eq!(login.response.bind.as_deref(), Some("bearer_token"));
        match &login.response.extract {
            ExtractConfig::Jsonpath { path } => assert_eq!(path, "$.access_token"),
            other => panic!("expected jsonpath extract, got {other:?}"),
        }
        assert_eq!(login.body.format, BodyFormat::Raw);
        let body_str = login.body.content.as_str().expect("raw body is a string");
        assert!(
            body_str.contains("{{ env.LOGIN_USERNAME }}"),
            "expected `${{...}}` to be rewritten to `{{{{ env.X }}}}`, got: {body_str}"
        );
        assert!(body_str.contains("{{ env.LOGIN_PASSWORD }}"));
        assert!(!body_str.contains("${LOGIN"));

        let chat = reqs.get("chat").expect("chat request still present");
        assert_eq!(
            chat.headers.get("Authorization").map(|s| s.as_str()),
            Some("Bearer {{ acme__login.bearer_token }}")
        );
    }

    #[test]
    fn synthesis_is_idempotent() {
        let root = TempDir::new().unwrap();
        let targets_dir = root.path().join("targets");
        let requests_dir = root.path().join("requests");
        std::fs::create_dir_all(&targets_dir).unwrap();
        std::fs::create_dir_all(&requests_dir).unwrap();

        write_request(&requests_dir, &sample_request("chat"));
        write_target(
            &targets_dir,
            &target_with_http_login("acme", "Acme", "chat", &[], standard_http_login()),
        );

        let first = synthesize_auth_chain_requests(&targets_dir, &requests_dir).unwrap();
        assert_eq!(first.login_requests_synthesized, 1);
        assert_eq!(first.chat_requests_wired, 1);

        let second = synthesize_auth_chain_requests(&targets_dir, &requests_dir).unwrap();
        assert_eq!(second.login_requests_synthesized, 0);
        assert_eq!(second.chat_requests_wired, 0);
        assert_eq!(second.chat_requests_already_wired, 1);
        assert_eq!(second.targets_skipped, 1);
    }

    #[test]
    fn does_not_overwrite_existing_authorization_header() {
        let root = TempDir::new().unwrap();
        let targets_dir = root.path().join("targets");
        let requests_dir = root.path().join("requests");
        std::fs::create_dir_all(&targets_dir).unwrap();
        std::fs::create_dir_all(&requests_dir).unwrap();

        let mut chat = sample_request("chat");
        chat.headers
            .insert("Authorization".into(), "Bearer ${EXISTING_TOKEN}".into());
        write_request(&requests_dir, &chat);
        write_target(
            &targets_dir,
            &target_with_http_login("acme", "Acme", "chat", &[], standard_http_login()),
        );

        let report = synthesize_auth_chain_requests(&targets_dir, &requests_dir).unwrap();
        assert_eq!(report.login_requests_synthesized, 1);
        assert_eq!(report.chat_requests_wired, 0);
        assert_eq!(report.chat_requests_already_wired, 1);

        let reqs = requests::load_all(&requests_dir).unwrap();
        assert_eq!(
            reqs["chat"].headers.get("Authorization").unwrap(),
            "Bearer ${EXISTING_TOKEN}"
        );
    }

    #[test]
    fn target_without_http_login_is_skipped() {
        let root = TempDir::new().unwrap();
        let targets_dir = root.path().join("targets");
        let requests_dir = root.path().join("requests");
        std::fs::create_dir_all(&targets_dir).unwrap();
        std::fs::create_dir_all(&requests_dir).unwrap();

        write_request(&requests_dir, &sample_request("chat"));
        write_target(
            &targets_dir,
            &sample_target("manual", "Manual", "chat", &[]),
        );

        let report = synthesize_auth_chain_requests(&targets_dir, &requests_dir).unwrap();
        assert_eq!(report.login_requests_synthesized, 0);
        assert_eq!(report.chat_requests_wired, 0);
        assert_eq!(report.targets_skipped, 1);

        let reqs = requests::load_all(&requests_dir).unwrap();
        assert!(!reqs.contains_key("manual__login"));
    }

    #[test]
    fn missing_token_path_defaults_to_access_token() {
        let root = TempDir::new().unwrap();
        let targets_dir = root.path().join("targets");
        let requests_dir = root.path().join("requests");
        std::fs::create_dir_all(&targets_dir).unwrap();
        std::fs::create_dir_all(&requests_dir).unwrap();

        let mut http_login = standard_http_login();
        http_login.token_json_path = None;

        write_request(&requests_dir, &sample_request("chat"));
        write_target(
            &targets_dir,
            &target_with_http_login("acme", "Acme", "chat", &[], http_login),
        );

        let _ = synthesize_auth_chain_requests(&targets_dir, &requests_dir).unwrap();
        let reqs = requests::load_all(&requests_dir).unwrap();
        match &reqs["acme__login"].response.extract {
            ExtractConfig::Jsonpath { path } => assert_eq!(path, "$.access_token"),
            other => panic!("expected jsonpath default, got {other:?}"),
        }
    }

    #[test]
    fn translate_env_placeholders_handles_mixed_input() {
        assert_eq!(
            translate_env_placeholders("Bearer ${TOKEN}"),
            "Bearer {{ env.TOKEN }}"
        );
        assert_eq!(
            translate_env_placeholders(r#"{"u":"${A}","p":"${B_C}"}"#),
            r#"{"u":"{{ env.A }}","p":"{{ env.B_C }}"}"#
        );
        // Bare $ without braces or with non-identifier chars is preserved.
        assert_eq!(translate_env_placeholders("price $5"), "price $5");
        assert_eq!(translate_env_placeholders("${1BAD}"), "${1BAD}");
    }

    #[test]
    fn missing_dirs_are_tolerated_for_auth_chain() {
        let root = TempDir::new().unwrap();
        let targets_dir = root.path().join("targets");
        let requests_dir = root.path().join("requests");
        let report = synthesize_auth_chain_requests(&targets_dir, &requests_dir).unwrap();
        assert_eq!(report, AuthChainMigrationReport::default());
    }
}
