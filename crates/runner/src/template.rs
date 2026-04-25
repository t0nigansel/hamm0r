use minijinja::{AutoEscape, Environment};

use crate::error::RunnerError;

/// Render a template string, substituting `{{ prompt }}` with `payload`.
///
/// Auto-escape is off — payloads are intentionally literal attack strings.
/// The environment is sandboxed: no filesystem loader, no includes.
pub fn render(template: &str, payload: &str) -> Result<String, RunnerError> {
    let mut env = Environment::new();
    env.set_auto_escape_callback(|_| AutoEscape::None);

    let tmpl = env.template_from_str(template)?;
    Ok(tmpl.render(minijinja::context! { prompt => payload })?)
}

/// Render each header value that contains `{{ prompt }}`.
pub fn render_headers(
    headers: &std::collections::HashMap<String, String>,
    payload: &str,
) -> Result<std::collections::HashMap<String, String>, RunnerError> {
    headers
        .iter()
        .map(|(k, v)| Ok((k.clone(), render(v, payload)?)))
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
}
