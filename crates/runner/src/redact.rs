//! Header redaction for logs and previews. Single source of truth for the
//! list of header names we treat as secret (see CLAUDE.md tech invariant 10).
//! Both the runner (run.rs) and the Tauri command layer (test_request,
//! diagnostics) call into this to make sure the redaction set cannot drift.

use std::collections::HashMap;

use storage::types::{AuthConfig, Request};

const SECRET_HEADERS: &[&str] = &[
    "authorization",
    "proxy-authorization",
    "x-api-key",
    "api-key",
    "x-auth-token",
];

/// Clone the request's headers and replace any secret values with
/// `<redacted>`. Auth-derived headers (Bearer/Basic/CustomHeader) get a
/// masked synthetic entry so the log shows that auth was applied without
/// revealing the value.
pub fn request_headers_for_log(request: &Request) -> HashMap<String, String> {
    let mut headers = request.headers.clone();
    redact_known_secret_headers(&mut headers);

    match &request.auth {
        AuthConfig::Bearer { .. } => {
            upsert_masked_header(&mut headers, "Authorization", "Bearer <redacted>");
        }
        AuthConfig::Basic { .. } => {
            upsert_masked_header(&mut headers, "Authorization", "Basic <redacted>");
        }
        AuthConfig::CustomHeader { header_name, .. } => {
            upsert_masked_header(&mut headers, header_name, "<redacted>");
        }
        AuthConfig::None => {}
    }

    headers
}

/// Mutate a header map in place, replacing values whose name matches the
/// secret list with `<redacted>`. Comparison is ASCII-case-insensitive.
pub fn redact_known_secret_headers(headers: &mut HashMap<String, String>) {
    for (name, value) in headers.iter_mut() {
        if SECRET_HEADERS
            .iter()
            .any(|secret| name.eq_ignore_ascii_case(secret))
        {
            *value = "<redacted>".to_owned();
        }
    }
}

/// Insert or overwrite a header in `headers` with `masked`. Matches the
/// existing key case-insensitively so we don't end up with two entries that
/// differ only by case.
pub fn upsert_masked_header(
    headers: &mut HashMap<String, String>,
    header_name: &str,
    masked: &str,
) {
    if let Some((_, value)) = headers
        .iter_mut()
        .find(|(name, _)| name.eq_ignore_ascii_case(header_name))
    {
        *value = masked.to_owned();
        return;
    }

    headers.insert(header_name.to_owned(), masked.to_owned());
}
