use std::collections::HashMap;

use minijinja::{AutoEscape, Environment};

use crate::error::RunnerError;

/// Phase 2 of docs/RefactorPlan.md: bind cache used to satisfy
/// `{{<request_id>.<bind_name>}}` references at render time.
///
/// Outer key is the producing Request id; inner key is the bind name
/// declared on `response.bind` (or any future binding the runner adds).
/// Empty cache produces today's behavior — only `{{ prompt }}` is in scope.
pub type BindCache = HashMap<String, HashMap<String, String>>;

/// Render a template string, substituting `{{ prompt }}` with `payload`.
///
/// Auto-escape is off — payloads are intentionally literal attack strings.
/// The environment is sandboxed: no filesystem loader, no includes.
/// Trailing newlines are preserved so raw / text bodies are sent verbatim.
pub fn render(template: &str, payload: &str) -> Result<String, RunnerError> {
    render_with(template, payload, &BindCache::new())
}

/// Render with `{{ prompt }}`, `{{<request_id>.<bind_name>}}`, and
/// `{{ env.VAR_NAME }}` references in scope. Each producing Request id
/// becomes a top-level minijinja variable whose attributes are its bind
/// names; `env` is a top-level object mirroring the process environment.
///
/// Env var access is read-only and read at render time. Attack payloads
/// are substituted as opaque strings (not re-rendered), so a `{{ env.X }}`
/// inside `payload` does NOT exfiltrate process env.
pub fn render_with(
    template: &str,
    payload: &str,
    binds: &BindCache,
) -> Result<String, RunnerError> {
    let mut env = Environment::new();
    env.set_auto_escape_callback(|_| AutoEscape::None);
    env.set_keep_trailing_newline(true);

    let tmpl = env.template_from_str(template)?;

    // Build context: { prompt: <payload>, env: {...}, <req_id>: { <bind>: <value>, ... }, ... }
    let mut ctx = serde_json::Map::new();
    ctx.insert(
        "prompt".to_owned(),
        serde_json::Value::String(payload.to_owned()),
    );
    let env_obj: serde_json::Map<String, serde_json::Value> = std::env::vars()
        .map(|(k, v)| (k, serde_json::Value::String(v)))
        .collect();
    ctx.insert("env".to_owned(), serde_json::Value::Object(env_obj));
    for (req_id, bindings) in binds {
        if req_id == "env" || req_id == "prompt" {
            continue;
        }
        let inner: serde_json::Map<String, serde_json::Value> = bindings
            .iter()
            .map(|(k, v)| (k.clone(), serde_json::Value::String(v.clone())))
            .collect();
        ctx.insert(req_id.clone(), serde_json::Value::Object(inner));
    }
    Ok(tmpl.render(serde_json::Value::Object(ctx))?)
}

/// Render template markers inside a JSON value while preserving JSON types.
pub fn render_json_value(
    value: serde_json::Value,
    payload: &str,
) -> Result<serde_json::Value, RunnerError> {
    render_json_value_with(value, payload, &BindCache::new())
}

/// Same as `render_json_value`, with bind-cache substitution support.
pub fn render_json_value_with(
    value: serde_json::Value,
    payload: &str,
    binds: &BindCache,
) -> Result<serde_json::Value, RunnerError> {
    match value {
        serde_json::Value::String(s) => {
            Ok(serde_json::Value::String(render_with(&s, payload, binds)?))
        }
        serde_json::Value::Array(items) => items
            .into_iter()
            .map(|item| render_json_value_with(item, payload, binds))
            .collect::<Result<Vec<_>, _>>()
            .map(serde_json::Value::Array),
        serde_json::Value::Object(map) => {
            let rendered = map
                .into_iter()
                .map(|(key, value)| {
                    Ok((
                        render_with(&key, payload, binds)?,
                        render_json_value_with(value, payload, binds)?,
                    ))
                })
                .collect::<Result<serde_json::Map<_, _>, RunnerError>>()?;
            Ok(serde_json::Value::Object(rendered))
        }
        other => Ok(other),
    }
}

/// Render each header value that contains `{{ prompt }}`.
pub fn render_headers(
    headers: &std::collections::HashMap<String, String>,
    payload: &str,
) -> Result<std::collections::HashMap<String, String>, RunnerError> {
    render_headers_with(headers, payload, &BindCache::new())
}

/// Same as `render_headers`, with bind-cache substitution support.
pub fn render_headers_with(
    headers: &std::collections::HashMap<String, String>,
    payload: &str,
    binds: &BindCache,
) -> Result<std::collections::HashMap<String, String>, RunnerError> {
    headers
        .iter()
        .map(|(k, v)| Ok((k.clone(), render_with(v, payload, binds)?)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn basic_substitution() {
        let out = render("Say: {{ prompt }}", "hello world").unwrap();
        assert_eq!(out, "Say: hello world");
    }

    #[test]
    fn payload_not_html_escaped() {
        let out = render("{{ prompt }}", "<script>alert(1)</script>").unwrap();
        assert_eq!(out, "<script>alert(1)</script>");
    }

    #[test]
    fn no_prompt_variable_passes_through() {
        let out = render("static body", "ignored").unwrap();
        assert_eq!(out, "static body");
    }

    #[test]
    fn trailing_newline_preserved() {
        let out = render("body\n", "x").unwrap();
        assert_eq!(out, "body\n", "trailing \\n must be kept verbatim");
    }

    #[test]
    fn bind_substitution_into_header_value() {
        let mut binds: BindCache = HashMap::new();
        let mut login_binds = HashMap::new();
        login_binds.insert("bearer_token".to_owned(), "abc123".to_owned());
        binds.insert("login".to_owned(), login_binds);

        let out = render_with(
            "Authorization: Bearer {{ login.bearer_token }}",
            "ignored",
            &binds,
        )
        .unwrap();
        assert_eq!(out, "Authorization: Bearer abc123");
    }

    #[test]
    fn bind_and_prompt_coexist_in_one_template() {
        let mut binds: BindCache = HashMap::new();
        let mut bs = HashMap::new();
        bs.insert("session".to_owned(), "sess-42".to_owned());
        binds.insert("bootstrap".to_owned(), bs);

        let out = render_with(
            "session={{ bootstrap.session }}, attack={{ prompt }}",
            "ignore all",
            &binds,
        )
        .unwrap();
        assert_eq!(out, "session=sess-42, attack=ignore all");
    }

    #[test]
    fn missing_bind_errors() {
        let binds = BindCache::new();
        let err = render_with("{{ login.bearer_token }}", "x", &binds)
            .expect_err("missing bind must error");
        let msg = format!("{err}");
        // minijinja surfaces "undefined value" on attribute access against missing var
        assert!(
            msg.to_lowercase().contains("undefined") || msg.to_lowercase().contains("not"),
            "expected an undefined-variable error, got: {msg}"
        );
    }

    #[test]
    fn env_var_substitution() {
        // Use a stable env var the test runtime is guaranteed to set.
        // PATH on Windows, PATH on Unix — both exist.
        let actual = std::env::var("PATH").unwrap_or_default();
        let out = render("PATH={{ env.PATH }}", "ignored").unwrap();
        assert_eq!(out, format!("PATH={actual}"));
    }

    #[test]
    fn env_with_explicit_test_var() {
        // SAFETY: tests in this module run sequentially within the binary;
        // we set + unset around the assertion.
        unsafe {
            std::env::set_var("HAMM0R_TEMPLATE_TEST_VAR", "hello");
        }
        let out = render("v={{ env.HAMM0R_TEMPLATE_TEST_VAR }}", "x").unwrap();
        assert_eq!(out, "v=hello");
        unsafe {
            std::env::remove_var("HAMM0R_TEMPLATE_TEST_VAR");
        }
    }

    #[test]
    fn missing_env_var_renders_empty() {
        // minijinja's default undefined behavior renders empty for a missing
        // attribute on a defined object. That matches what we want for
        // env vars: a typo yields a visibly-empty value, not a hard error.
        let out = render("v={{ env.HAMM0R_DEFINITELY_NOT_SET_XYZ }}", "x").unwrap();
        assert_eq!(out, "v=");
    }

    #[test]
    fn header_rendering() {
        use std::collections::HashMap;
        let headers: HashMap<String, String> = [
            ("X-Payload".into(), "value={{ prompt }}".into()),
            ("Content-Type".into(), "application/json".into()),
        ]
        .into();
        let rendered = render_headers(&headers, "attack").unwrap();
        assert_eq!(rendered["X-Payload"], "value=attack");
        assert_eq!(rendered["Content-Type"], "application/json");
    }

    #[test]
    fn json_value_rendering_preserves_types() {
        let input = serde_json::json!({
            "message": "{{ prompt }}",
            "enabled": true,
            "count": 3,
            "nested": ["prefix {{ prompt }}", null]
        });

        let rendered = render_json_value(input, "hello").unwrap();

        assert_eq!(
            rendered,
            serde_json::json!({
                "message": "hello",
                "enabled": true,
                "count": 3,
                "nested": ["prefix hello", null]
            })
        );
    }

    #[test]
    fn json_value_rendering_supports_binds_and_keys() {
        let mut binds: BindCache = HashMap::new();
        let mut bs = HashMap::new();
        bs.insert("field".to_owned(), "token".to_owned());
        bs.insert("value".to_owned(), "abc123".to_owned());
        binds.insert("login".to_owned(), bs);

        let input = serde_json::json!({
            "{{ login.field }}": "{{ login.value }}",
            "prompt": "{{ prompt }}"
        });

        let rendered = render_json_value_with(input, "attack", &binds).unwrap();

        assert_eq!(
            rendered,
            serde_json::json!({
                "token": "abc123",
                "prompt": "attack"
            })
        );
    }
}
