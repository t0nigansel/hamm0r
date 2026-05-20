use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use runner::run::{execute_run, AttemptLog, RunCancellation};
use runner::session::SessionStrategy;
use runner::{Payload, RunConfig};
use storage::runs::{read_all, RunRecord, RunStatus};
use storage::types::{
    AdapterType, AuthConfig, BodyConfig, BodyFormat, ExtractConfig, Request, ResponseConfig,
};
use tempfile::TempDir;
use wiremock::matchers::{body_string_contains, header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

// ── Helpers ───────────────────────────────────────────────────────────────────

fn make_request(url: &str, adapter: AdapterType, extract: ExtractConfig) -> Request {
    Request {
        version: 1,
        id: "test-request".into(),
        name: "Test Request".into(),
        method: "POST".into(),
        url: url.to_owned(),
        auth: AuthConfig::None,
        headers: HashMap::from([("Content-Type".into(), "application/json".into())]),
        body: BodyConfig {
            format: BodyFormat::Json,
            content: serde_json::json!({ "prompt": "{{ prompt }}" }),
        },
        response: ResponseConfig {
            extract,
            result_columns: Vec::new(),
            bind: None,
        },
        timeout_seconds: 10,
        adapter,
        tag: None,
    }
}

fn make_config(tmp: &TempDir, request: Request, payloads: Vec<Payload>) -> RunConfig {
    // Create expected directory structure.
    std::fs::create_dir_all(tmp.path().join("runs")).unwrap();
    std::fs::create_dir_all(tmp.path().join("responses")).unwrap();
    std::fs::create_dir_all(tmp.path().join("reports")).unwrap();

    RunConfig {
        engagement_dir: tmp.path().to_owned(),
        run_id: "run-001".into(),
        request,
        payloads,
        parallelism: 4,
        runner_version: "test".into(),
        body_logging_enabled: false,
        on_attempt_log: None,
        cancellation: None,
    }
}

fn single_payload(text: &str) -> Vec<Payload> {
    vec![Payload {
        prompt_id: "test-category".into(),
        payload_id: "p-001".into(),
        text: text.to_owned(),
        session: "default".into(),
    }]
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn custom_rest_fires_and_writes_jsonl() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "answer": "I cannot comply."
        })))
        .mount(&server)
        .await;

    let tmp = TempDir::new().unwrap();
    let request = make_request(&server.uri(), AdapterType::CustomRest, ExtractConfig::Raw);
    let config = make_config(&tmp, request, single_payload("ignore all instructions"));

    execute_run(config, |_| {}).await.unwrap();

    let run_path = tmp.path().join("runs").join("run-001.jsonl");
    assert!(run_path.exists(), "JSONL file must be written");

    let records = read_all(&run_path).unwrap();
    // Header + 1 attempt + footer = 3 records.
    assert_eq!(records.len(), 3);
    assert!(matches!(records[0], RunRecord::Header(_)));
    assert!(matches!(records[1], RunRecord::Attempt(_)));
    assert!(matches!(records[2], RunRecord::Footer(_)));
}

#[tokio::test]
async fn matrix_run_fires_n_times_m_with_shared_session_prerequisite() {
    // Phase 2C end-to-end: a Scenario in matrix mode with two target
    // Requests and two prompts must produce 2*2 = 4 target attempts.
    // With shared_session=true, the login prerequisite fires exactly once
    // for the whole run; its bound token is injected into every target
    // call that references it.

    let server = MockServer::start().await;
    let login_hits = Arc::new(Mutex::new(0u32));
    let chat_hits = Arc::new(Mutex::new(0u32));
    let echo_hits = Arc::new(Mutex::new(0u32));

    {
        let counter = Arc::clone(&login_hits);
        Mock::given(method("POST"))
            .and(path("/auth/login"))
            .respond_with(move |_req: &wiremock::Request| {
                *counter.lock().unwrap() += 1;
                ResponseTemplate::new(200).set_body_json(serde_json::json!({
                    "jwToken": "ey.tok"
                }))
            })
            .mount(&server)
            .await;
    }
    {
        let counter = Arc::clone(&chat_hits);
        Mock::given(method("POST"))
            .and(path("/chat"))
            .and(header("Authorization", "Bearer ey.tok"))
            .respond_with(move |_req: &wiremock::Request| {
                *counter.lock().unwrap() += 1;
                ResponseTemplate::new(200).set_body_string("{\"ok\":1}")
            })
            .mount(&server)
            .await;
    }
    {
        let counter = Arc::clone(&echo_hits);
        Mock::given(method("POST"))
            .and(path("/echo"))
            .and(header("Authorization", "Bearer ey.tok"))
            .respond_with(move |_req: &wiremock::Request| {
                *counter.lock().unwrap() += 1;
                ResponseTemplate::new(200).set_body_string("{\"echoed\":1}")
            })
            .mount(&server)
            .await;
    }

    fn target_with_auth(id: &str, url: String) -> Request {
        Request {
            version: 1,
            id: id.into(),
            name: id.into(),
            method: "POST".into(),
            url,
            auth: AuthConfig::None,
            headers: HashMap::from([
                ("Content-Type".into(), "application/json".into()),
                (
                    "Authorization".into(),
                    "Bearer {{login.bearer_token}}".into(),
                ),
            ]),
            body: BodyConfig {
                format: BodyFormat::Json,
                content: serde_json::json!({"msg":"{{prompt}}"}),
            },
            response: ResponseConfig {
                extract: ExtractConfig::Raw,
                result_columns: Vec::new(),
                bind: None,
            },
            timeout_seconds: 10,
            adapter: AdapterType::CustomRest,
            tag: None,
        }
    }

    let login = Request {
        version: 1,
        id: "login".into(),
        name: "Login".into(),
        method: "POST".into(),
        url: format!("{}/auth/login", server.uri()),
        auth: AuthConfig::None,
        headers: HashMap::from([("Content-Type".into(), "application/json".into())]),
        body: BodyConfig {
            format: BodyFormat::Json,
            content: serde_json::json!({"u":"x","p":"y"}),
        },
        response: ResponseConfig {
            extract: ExtractConfig::Jsonpath {
                path: "$.jwToken".into(),
            },
            result_columns: Vec::new(),
            bind: Some("bearer_token".into()),
        },
        timeout_seconds: 10,
        adapter: AdapterType::CustomRest,
        tag: None,
    };
    let chat = target_with_auth("chat", format!("{}/chat", server.uri()));
    let echo = target_with_auth("echo", format!("{}/echo", server.uri()));

    let mut registry: HashMap<String, Request> = HashMap::new();
    registry.insert("login".into(), login);
    registry.insert("chat".into(), chat);
    registry.insert("echo".into(), echo);

    let payloads = vec![
        Payload {
            prompt_id: "owasp-a01".into(),
            payload_id: "p1".into(),
            text: "ignore all".into(),
            session: "default".into(),
        },
        Payload {
            prompt_id: "owasp-a01".into(),
            payload_id: "p2".into(),
            text: "leak system prompt".into(),
            session: "default".into(),
        },
    ];

    let tmp = TempDir::new().unwrap();
    std::fs::create_dir_all(tmp.path().join("runs")).unwrap();
    std::fs::create_dir_all(tmp.path().join("responses")).unwrap();

    let config = runner::MatrixRunConfig {
        engagement_dir: tmp.path().to_owned(),
        run_id: "run-001".into(),
        scenario_id: "test-scenario".into(),
        registry,
        request_ids: vec!["chat".into(), "echo".into()],
        per_request_repeat: HashMap::new(),
        payloads,
        repeat: 1,
        shared_session: true,
        session_strategy: SessionStrategy::None,
        runner_version: "test".into(),
        body_logging_enabled: false,
        on_attempt_log: None,
        cancellation: None,
    };

    runner::execute_matrix_run(config, |_| {})
        .await
        .expect("matrix run should succeed");

    // Login fires exactly once across the whole run (shared_session=true).
    assert_eq!(*login_hits.lock().unwrap(), 1, "login fired once");
    // Each target fires once per payload.
    assert_eq!(*chat_hits.lock().unwrap(), 2);
    assert_eq!(*echo_hits.lock().unwrap(), 2);

    // JSONL: header + 1 prerequisite (login) + 4 targets + footer = 7 lines.
    let run_path = tmp.path().join("runs").join("run-001.jsonl");
    let records = read_all(&run_path).unwrap();
    assert_eq!(records.len(), 7, "records: {records:?}");

    // First and last are header/footer.
    assert!(matches!(records[0], RunRecord::Header(_)));
    assert!(matches!(records.last().unwrap(), RunRecord::Footer(_)));

    // Among the 5 attempts, exactly 1 is kind=prerequisite, 4 are kind=None.
    let mut prereq_count = 0;
    let mut target_count = 0;
    for rec in &records[1..records.len() - 1] {
        if let RunRecord::Attempt(a) = rec {
            match a.kind.as_deref() {
                Some("prerequisite") => prereq_count += 1,
                None => target_count += 1,
                Some(other) => panic!("unexpected kind: {other}"),
            }
        }
    }
    assert_eq!(prereq_count, 1);
    assert_eq!(target_count, 4);

    if let RunRecord::Footer(f) = records.last().unwrap() {
        assert!(matches!(f.status, RunStatus::Completed));
        assert_eq!(f.attempts_failed, 0);
        // attempts_total counts every attempt including prereqs.
        assert_eq!(f.attempts_total, 5);
    }
}

#[tokio::test]
async fn matrix_run_with_shared_session_false_fires_prereq_per_cell() {
    // Same setup as above but shared_session=false. The login prerequisite
    // should fire FOUR times — once per (request, prompt) cell — because
    // each cell starts with a fresh BindCache.

    let server = MockServer::start().await;
    let login_hits = Arc::new(Mutex::new(0u32));

    {
        let counter = Arc::clone(&login_hits);
        Mock::given(method("POST"))
            .and(path("/auth/login"))
            .respond_with(move |_req: &wiremock::Request| {
                *counter.lock().unwrap() += 1;
                ResponseTemplate::new(200).set_body_json(serde_json::json!({"jwToken": "t"}))
            })
            .mount(&server)
            .await;
    }
    Mock::given(method("POST"))
        .and(path("/x"))
        .and(header("Authorization", "Bearer t"))
        .respond_with(ResponseTemplate::new(200).set_body_string("{}"))
        .mount(&server)
        .await;

    let login = Request {
        version: 1,
        id: "login".into(),
        name: "Login".into(),
        method: "POST".into(),
        url: format!("{}/auth/login", server.uri()),
        auth: AuthConfig::None,
        headers: HashMap::from([("Content-Type".into(), "application/json".into())]),
        body: BodyConfig {
            format: BodyFormat::Json,
            content: serde_json::json!({}),
        },
        response: ResponseConfig {
            extract: ExtractConfig::Jsonpath {
                path: "$.jwToken".into(),
            },
            result_columns: Vec::new(),
            bind: Some("bearer_token".into()),
        },
        timeout_seconds: 10,
        adapter: AdapterType::CustomRest,
        tag: None,
    };
    let target = Request {
        version: 1,
        id: "x".into(),
        name: "X".into(),
        method: "POST".into(),
        url: format!("{}/x", server.uri()),
        auth: AuthConfig::None,
        headers: HashMap::from([
            ("Content-Type".into(), "application/json".into()),
            (
                "Authorization".into(),
                "Bearer {{login.bearer_token}}".into(),
            ),
        ]),
        body: BodyConfig {
            format: BodyFormat::Json,
            content: serde_json::json!({"m": "{{prompt}}"}),
        },
        response: ResponseConfig {
            extract: ExtractConfig::Raw,
            result_columns: Vec::new(),
            bind: None,
        },
        timeout_seconds: 10,
        adapter: AdapterType::CustomRest,
        tag: None,
    };

    let mut registry: HashMap<String, Request> = HashMap::new();
    registry.insert("login".into(), login);
    registry.insert("x".into(), target);

    let payloads = vec![
        Payload {
            prompt_id: "p".into(),
            payload_id: "p1".into(),
            text: "a".into(),
            session: "default".into(),
        },
        Payload {
            prompt_id: "p".into(),
            payload_id: "p2".into(),
            text: "b".into(),
            session: "default".into(),
        },
    ];

    let tmp = TempDir::new().unwrap();
    std::fs::create_dir_all(tmp.path().join("runs")).unwrap();

    let config = runner::MatrixRunConfig {
        engagement_dir: tmp.path().to_owned(),
        run_id: "run-001".into(),
        scenario_id: "test-scenario".into(),
        registry,
        request_ids: vec!["x".into()],
        per_request_repeat: HashMap::new(),
        payloads,
        repeat: 1,
        shared_session: false,
        session_strategy: SessionStrategy::None,
        runner_version: "test".into(),
        body_logging_enabled: false,
        on_attempt_log: None,
        cancellation: None,
    };
    runner::execute_matrix_run(config, |_| {}).await.unwrap();

    // shared_session=false: 1 request × 2 payloads = 2 cells, prereq per cell.
    assert_eq!(*login_hits.lock().unwrap(), 2);
}

#[tokio::test]
async fn auth_chain_fires_login_then_injects_bearer_token() {
    // Phase 2B end-to-end: a `login` Request extracts a token via JSONPath
    // and binds it as `bearer_token`. A `chat` Request references that bind
    // in its Authorization header. Calling fire_chain on `chat` must:
    // 1. fire `login` first (no Authorization header expected by the mock),
    // 2. extract `eyJ.fake.token` from the JSON response,
    // 3. fire `chat` with `Authorization: Bearer eyJ.fake.token`.

    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/auth/login"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "jwToken": "eyJ.fake.token"
        })))
        .mount(&server)
        .await;

    Mock::given(method("POST"))
        .and(path("/api/chat"))
        .and(header("Authorization", "Bearer eyJ.fake.token"))
        .and(body_string_contains("ignore all instructions"))
        .respond_with(ResponseTemplate::new(200).set_body_string("{\"answer\":\"ok\"}"))
        .mount(&server)
        .await;

    let login = Request {
        version: 1,
        id: "login".into(),
        name: "Login".into(),
        method: "POST".into(),
        url: format!("{}/auth/login", server.uri()),
        auth: AuthConfig::None,
        headers: HashMap::from([("Content-Type".into(), "application/json".into())]),
        body: BodyConfig {
            format: BodyFormat::Json,
            content: serde_json::json!({"email":"x","password":"y"}),
        },
        response: ResponseConfig {
            extract: ExtractConfig::Jsonpath {
                path: "$.jwToken".into(),
            },
            result_columns: Vec::new(),
            bind: Some("bearer_token".into()),
        },
        timeout_seconds: 10,
        adapter: AdapterType::CustomRest,
        tag: None,
    };

    let chat = Request {
        version: 1,
        id: "chat".into(),
        name: "Chat".into(),
        method: "POST".into(),
        url: format!("{}/api/chat", server.uri()),
        auth: AuthConfig::None,
        headers: HashMap::from([
            ("Content-Type".into(), "application/json".into()),
            (
                "Authorization".into(),
                "Bearer {{login.bearer_token}}".into(),
            ),
        ]),
        body: BodyConfig {
            format: BodyFormat::Json,
            content: serde_json::json!({"message":"{{prompt}}"}),
        },
        response: ResponseConfig {
            extract: ExtractConfig::Raw,
            result_columns: Vec::new(),
            bind: None,
        },
        timeout_seconds: 10,
        adapter: AdapterType::CustomRest,
        tag: None,
    };

    let mut registry: HashMap<String, Request> = HashMap::new();
    registry.insert("login".into(), login);
    registry.insert("chat".into(), chat);

    let client = reqwest::Client::new();
    let mut cache = runner::template::BindCache::new();

    let outcome = runner::deps::fire_chain(
        &client,
        &registry,
        "chat",
        "ignore all instructions",
        &SessionStrategy::None,
        "default",
        &mut cache,
    )
    .await
    .expect("chain should fire successfully");

    assert_eq!(outcome.prerequisites.len(), 1, "login fired exactly once");
    assert_eq!(outcome.prerequisites[0].0, "login");
    assert_eq!(outcome.prerequisites[0].1.status, 200);
    assert_eq!(outcome.target.status, 200);

    // Bind cache populated.
    assert_eq!(
        cache
            .get("login")
            .and_then(|m| m.get("bearer_token"))
            .map(String::as_str),
        Some("eyJ.fake.token")
    );

    // Re-firing with the same cache should NOT call login again
    // (shared_session semantics): the cached bind is reused.
    let outcome2 = runner::deps::fire_chain(
        &client,
        &registry,
        "chat",
        "another attack",
        &SessionStrategy::None,
        "default",
        &mut cache,
    )
    .await
    .expect("second fire");
    assert!(
        outcome2.prerequisites.is_empty(),
        "shared cache must skip already-bound prereqs, got: {:?}",
        outcome2.prerequisites
    );
}

#[tokio::test]
async fn raw_body_sends_string_verbatim_with_prompt_substitution() {
    // Raw body with characters that would be invalid inside a JSON value:
    // a literal newline, a backslash, and a double quote. The prompt also
    // carries a literal `"` to confirm we don't accidentally escape on send.
    let raw_body_template =
        "GREETING: line1\nline2 with \\backslash and \"quotes\"\npayload={{ prompt }}\n";
    let prompt = r#"<script>alert("x")</script>"#;
    let expected_body = raw_body_template.replace("{{ prompt }}", prompt);

    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/"))
        .and(body_string_contains(
            "payload=<script>alert(\"x\")</script>",
        ))
        .and(body_string_contains(
            "line2 with \\backslash and \"quotes\"",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_string("ok"))
        .mount(&server)
        .await;

    let request = Request {
        version: 1,
        id: "raw-test".into(),
        name: "Raw Test".into(),
        method: "POST".into(),
        url: server.uri(),
        auth: AuthConfig::None,
        headers: HashMap::from([("Content-Type".into(), "text/plain".into())]),
        body: BodyConfig {
            format: BodyFormat::Raw,
            content: serde_json::Value::String(raw_body_template.to_owned()),
        },
        response: ResponseConfig {
            extract: ExtractConfig::Raw,
            result_columns: Vec::new(),
            bind: None,
        },
        timeout_seconds: 10,
        adapter: AdapterType::CustomRest,
        tag: None,
    };

    let tmp = TempDir::new().unwrap();
    let config = make_config(&tmp, request, single_payload(prompt));
    execute_run(config, |_| {}).await.unwrap();

    let run_path = tmp.path().join("runs").join("run-001.jsonl");
    let records = read_all(&run_path).unwrap();
    // Header + 1 attempt + footer.
    assert_eq!(records.len(), 3);
    let attempt = match &records[1] {
        RunRecord::Attempt(a) => a,
        _ => panic!("expected attempt record"),
    };
    assert_eq!(attempt.response.status, 200);
    // body_size must reflect the rendered (post-substitution) body length.
    assert_eq!(attempt.request.body_size, expected_body.len() as u64);
}

#[tokio::test]
async fn response_body_written_to_file() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_string("secret content"))
        .mount(&server)
        .await;

    let tmp = TempDir::new().unwrap();
    let config = make_config(
        &tmp,
        make_request(&server.uri(), AdapterType::CustomRest, ExtractConfig::Raw),
        single_payload("test"),
    );

    execute_run(config, |_| {}).await.unwrap();

    let body_file = tmp
        .path()
        .join("responses")
        .join("run-001")
        .join("0001.txt");
    assert!(body_file.exists(), "response body file must be written");
    let body = std::fs::read_to_string(&body_file).unwrap();
    assert_eq!(body, "secret content");
}

#[tokio::test]
async fn jsonl_attempt_references_body_file() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_string("ok"))
        .mount(&server)
        .await;

    let tmp = TempDir::new().unwrap();
    let config = make_config(
        &tmp,
        make_request(&server.uri(), AdapterType::CustomRest, ExtractConfig::Raw),
        single_payload("test"),
    );

    execute_run(config, |_| {}).await.unwrap();

    let records = read_all(&tmp.path().join("runs").join("run-001.jsonl")).unwrap();
    if let RunRecord::Attempt(a) = &records[1] {
        assert_eq!(
            a.response.body_file.as_deref(),
            Some("responses/run-001/0001.txt")
        );
    } else {
        panic!("expected attempt record");
    }
}

#[tokio::test]
async fn failed_request_records_error_in_jsonl() {
    // Point at a port with nothing listening.
    let request = make_request(
        "http://127.0.0.1:1", // nothing listening
        AdapterType::CustomRest,
        ExtractConfig::Raw,
    );
    let tmp = TempDir::new().unwrap();
    let mut req = request;
    req.timeout_seconds = 2;
    let config = make_config(&tmp, req, single_payload("test"));

    // execute_run must succeed even when the HTTP call fails.
    execute_run(config, |_| {}).await.unwrap();

    let records = read_all(&tmp.path().join("runs").join("run-001.jsonl")).unwrap();
    if let RunRecord::Attempt(a) = &records[1] {
        assert_eq!(
            a.response.status, 0,
            "failed request should record status 0"
        );
        assert!(a.response.error.is_some(), "error field must be set");
    } else {
        panic!("expected attempt record");
    }
}

#[tokio::test]
async fn jsonpath_extraction_works() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "choices": [{"message": {"content": "extracted!"}}]
        })))
        .mount(&server)
        .await;

    let tmp = TempDir::new().unwrap();
    let request = make_request(
        &server.uri(),
        AdapterType::CustomRest,
        ExtractConfig::Jsonpath {
            path: "$.choices[0].message.content".into(),
        },
    );
    let config = make_config(&tmp, request, single_payload("test"));

    execute_run(config, |_| {}).await.unwrap();

    // The extracted value should be in the body file (for now body file = raw body).
    // Extraction is stored separately in M6; here we just verify no panic.
    let records = read_all(&tmp.path().join("runs").join("run-001.jsonl")).unwrap();
    assert!(matches!(records[1], RunRecord::Attempt(_)));
}

#[tokio::test]
async fn bounded_parallelism_fires_all_payloads() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_string("ok"))
        .expect(5) // must receive exactly 5 calls
        .mount(&server)
        .await;

    let payloads: Vec<Payload> = (1..=5)
        .map(|i| Payload {
            prompt_id: "cat".into(),
            payload_id: format!("p-{i:03}"),
            text: format!("payload {i}"),
            session: "default".into(),
        })
        .collect();

    let tmp = TempDir::new().unwrap();
    let config = RunConfig {
        engagement_dir: {
            std::fs::create_dir_all(tmp.path().join("runs")).unwrap();
            std::fs::create_dir_all(tmp.path().join("responses")).unwrap();
            std::fs::create_dir_all(tmp.path().join("reports")).unwrap();
            tmp.path().to_owned()
        },
        run_id: "run-001".into(),
        request: make_request(&server.uri(), AdapterType::CustomRest, ExtractConfig::Raw),
        payloads,
        parallelism: 2, // only 2 in flight at a time
        runner_version: "test".into(),
        body_logging_enabled: false,
        on_attempt_log: None,
        cancellation: None,
    };

    execute_run(config, |_| {}).await.unwrap();

    let records = read_all(&tmp.path().join("runs").join("run-001.jsonl")).unwrap();
    // header + 5 attempts + footer = 7
    assert_eq!(records.len(), 7);

    // wiremock verifies the `expect(5)` expectation on drop.
}

#[tokio::test]
async fn session_cookie_strategy_reuses_client() {
    // Two payloads in the same session: both should succeed.
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_string("ok"))
        .expect(2)
        .mount(&server)
        .await;

    let payloads = vec![
        Payload {
            prompt_id: "cat".into(),
            payload_id: "p1".into(),
            text: "first".into(),
            session: "session-A".into(),
        },
        Payload {
            prompt_id: "cat".into(),
            payload_id: "p2".into(),
            text: "second".into(),
            session: "session-A".into(),
        },
    ];

    let tmp = TempDir::new().unwrap();
    std::fs::create_dir_all(tmp.path().join("runs")).unwrap();
    std::fs::create_dir_all(tmp.path().join("responses")).unwrap();
    std::fs::create_dir_all(tmp.path().join("reports")).unwrap();

    let config = RunConfig {
        engagement_dir: tmp.path().to_owned(),
        run_id: "run-001".into(),
        request: make_request(&server.uri(), AdapterType::CustomRest, ExtractConfig::Raw),
        payloads,
        parallelism: 1,
        runner_version: "test".into(),
        body_logging_enabled: false,
        on_attempt_log: None,
        cancellation: None,
    };

    execute_run(config, |_| {}).await.unwrap();

    let records = read_all(&tmp.path().join("runs").join("run-001.jsonl")).unwrap();
    assert_eq!(records.len(), 4); // header + 2 attempts + footer
}

#[tokio::test]
async fn attempt_log_omits_bodies_when_body_logging_is_disabled() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_string("ok-body"))
        .mount(&server)
        .await;

    let tmp = TempDir::new().unwrap();
    let captured: Arc<Mutex<Vec<AttemptLog>>> = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&captured);
    let mut config = make_config(
        &tmp,
        make_request(&server.uri(), AdapterType::CustomRest, ExtractConfig::Raw),
        single_payload("hello body"),
    );
    config.body_logging_enabled = false;
    config.on_attempt_log = Some(Arc::new(move |attempt| {
        sink.lock().unwrap().push(attempt);
    }));

    execute_run(config, |_| {}).await.unwrap();

    let items = captured.lock().unwrap();
    assert_eq!(items.len(), 1);
    assert!(items[0].request_body.is_none());
    assert!(items[0].response_body.is_none());
}

#[tokio::test]
async fn attempt_log_includes_bodies_when_body_logging_is_enabled() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_string("ok-body"))
        .mount(&server)
        .await;

    let tmp = TempDir::new().unwrap();
    let captured: Arc<Mutex<Vec<AttemptLog>>> = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&captured);
    let mut config = make_config(
        &tmp,
        make_request(&server.uri(), AdapterType::CustomRest, ExtractConfig::Raw),
        single_payload("hello body"),
    );
    config.body_logging_enabled = true;
    config.on_attempt_log = Some(Arc::new(move |attempt| {
        sink.lock().unwrap().push(attempt);
    }));

    execute_run(config, |_| {}).await.unwrap();

    let items = captured.lock().unwrap();
    assert_eq!(items.len(), 1);
    assert!(items[0]
        .request_body
        .as_deref()
        .is_some_and(|body| body.contains("hello body")));
    assert_eq!(items[0].response_body.as_deref(), Some("ok-body"));
}

#[tokio::test]
async fn run_cancellation_writes_aborted_footer() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_delay(Duration::from_millis(250))
                .set_body_string("ok"),
        )
        .mount(&server)
        .await;

    let payloads: Vec<Payload> = (1..=3)
        .map(|i| Payload {
            prompt_id: "cat".into(),
            payload_id: format!("p-{i:03}"),
            text: format!("payload {i}"),
            session: "default".into(),
        })
        .collect();

    let tmp = TempDir::new().unwrap();
    let cancellation = RunCancellation::new();
    let mut config = make_config(
        &tmp,
        make_request(&server.uri(), AdapterType::CustomRest, ExtractConfig::Raw),
        payloads,
    );
    config.parallelism = 1;
    config.cancellation = Some(cancellation.clone());

    let task = tokio::spawn(async move { execute_run(config, |_| {}).await.unwrap() });
    tokio::time::sleep(Duration::from_millis(60)).await;
    cancellation.cancel();
    task.await.unwrap();

    let records = read_all(&tmp.path().join("runs").join("run-001.jsonl")).unwrap();
    let footer = records.iter().find_map(|record| match record {
        RunRecord::Footer(footer) => Some(footer),
        _ => None,
    });

    let footer = footer.expect("aborted run should still write a footer");
    assert_eq!(footer.status, RunStatus::AbortedByUser);
    assert!(footer.attempts_total < 3);
}

// ── Per-request repeat (item 5 of docs/ToDo.md) ───────────────────────────────

#[tokio::test]
async fn per_request_repeat_multiplies_attempt_count() {
    // login: no per-request repeat (defaults to 1)
    // chat: per-request repeat = 3
    // Global repeat = 2, 1 payload.
    //
    // Expected target attempts: (1×2 + 3×2) × 1 payload = 8
    // login fires: 2 (global_repeat × 1)
    // chat fires: 6 (global_repeat × 3)

    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/login"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"token":"t"})))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/chat"))
        .respond_with(ResponseTemplate::new(200).set_body_string("ok"))
        .mount(&server)
        .await;

    let mut registry = HashMap::new();
    registry.insert(
        "login".into(),
        Request {
            version: 1,
            id: "login".into(),
            name: "Login".into(),
            method: "POST".into(),
            url: format!("{}/login", server.uri()),
            auth: AuthConfig::None,
            headers: HashMap::new(),
            body: BodyConfig {
                format: BodyFormat::Json,
                content: serde_json::json!({}),
            },
            response: ResponseConfig {
                extract: ExtractConfig::Raw,
                result_columns: Vec::new(),
                bind: None,
            },
            timeout_seconds: 5,
            adapter: AdapterType::CustomRest,
            tag: None,
        },
    );
    registry.insert(
        "chat".into(),
        Request {
            version: 1,
            id: "chat".into(),
            name: "Chat".into(),
            method: "POST".into(),
            url: format!("{}/chat", server.uri()),
            auth: AuthConfig::None,
            headers: HashMap::new(),
            body: BodyConfig {
                format: BodyFormat::Json,
                content: serde_json::json!({"msg": "{{prompt}}"}),
            },
            response: ResponseConfig {
                extract: ExtractConfig::Raw,
                result_columns: Vec::new(),
                bind: None,
            },
            timeout_seconds: 5,
            adapter: AdapterType::CustomRest,
            tag: None,
        },
    );

    let payloads = vec![Payload {
        prompt_id: "cat".into(),
        payload_id: "p1".into(),
        text: "attack".into(),
        session: "default".into(),
    }];

    let tmp = TempDir::new().unwrap();
    std::fs::create_dir_all(tmp.path().join("runs")).unwrap();
    std::fs::create_dir_all(tmp.path().join("responses")).unwrap();

    let mut per_request_repeat = HashMap::new();
    per_request_repeat.insert("chat".into(), 3u32);

    let config = runner::MatrixRunConfig {
        engagement_dir: tmp.path().to_owned(),
        run_id: "run-001".into(),
        scenario_id: "repeat-test".into(),
        registry,
        request_ids: vec!["login".into(), "chat".into()],
        per_request_repeat,
        payloads,
        repeat: 2,
        shared_session: false,
        session_strategy: SessionStrategy::None,
        runner_version: "test".into(),
        body_logging_enabled: false,
        on_attempt_log: None,
        cancellation: None,
    };

    runner::execute_matrix_run(config, |_| {}).await.unwrap();

    let records = storage::runs::read_all(&tmp.path().join("runs").join("run-001.jsonl")).unwrap();

    // Count target attempts (kind != "prerequisite").
    let target_attempts: Vec<_> = records
        .iter()
        .filter_map(|r| match r {
            storage::runs::RunRecord::Attempt(a) if a.kind.as_deref() != Some("prerequisite") => {
                Some(a.as_ref())
            }
            _ => None,
        })
        .collect();

    // login: 1 (per_request) × 2 (global) = 2 target firings
    let login_count = target_attempts
        .iter()
        .filter(|a| a.request.url.contains("/login"))
        .count();
    // chat: 3 (per_request) × 2 (global) = 6 target firings
    let chat_count = target_attempts
        .iter()
        .filter(|a| a.request.url.contains("/chat"))
        .count();

    assert_eq!(login_count, 2, "login fires global_repeat × 1 = 2 times");
    assert_eq!(
        chat_count, 6,
        "chat fires global_repeat × per_repeat = 6 times"
    );
    assert_eq!(target_attempts.len(), 8, "total target attempts = 8");

    let footer = records.iter().find_map(|r| match r {
        storage::runs::RunRecord::Footer(f) => Some(f),
        _ => None,
    });
    assert_eq!(footer.unwrap().attempts_total, 8);
}

#[test]
fn per_request_repeat_expansion_math() {
    // Unit test: verify the total calculation without network I/O.
    // 2 requests (login×1, chat×3), 2 payloads, global repeat=2 → 16 target cells.
    // (login contributes 1×2 + chat contributes 3×2) × 2 payloads = 16.
    let request_repeats: HashMap<&str, u32> = [("chat", 3)].into();
    let request_ids = ["login", "chat"];
    let payload_count = 2u32;
    let global_repeat = 2u32;

    let request_repeat_sum: u32 = request_ids
        .iter()
        .map(|id| request_repeats.get(id).copied().unwrap_or(1).max(1))
        .sum();
    let total = payload_count
        .saturating_mul(request_repeat_sum)
        .saturating_mul(global_repeat);

    assert_eq!(request_repeat_sum, 4); // 1 + 3
    assert_eq!(total, 16); // 2 payloads × 4 × 2 global
}
