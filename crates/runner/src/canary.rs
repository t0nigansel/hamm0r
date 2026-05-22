// Multi-session canary tokens (Section 1.3 of docs/ToDo.md).
//
// A canary is a short, prefixed string a plant prompt can embed so the
// post-run leak scanner can find it in a probe response from a different
// session. The token is deterministic — `(run_id, session_idx,
// scenario_id)` always hashes to the same value — so reruns are
// reproducible and the same canary can be looked up by the scanner after
// the run completes.
//
// Format: `HAMM0R-<11-hex-chars>` = 18 chars total. The `HAMM0R-` prefix
// makes scan results unambiguous when they show up in transcripts; 11
// hex chars expose 44 bits of `SHA-256(run_id || ':' || session_idx ||
// ':' || scenario_id)`, which is collision-free across realistic
// engagement volumes.

use sha2::{Digest as _, Sha256};

/// Length of the hex suffix after the `HAMM0R-` prefix.
const HEX_LEN: usize = 11;

/// Prefix written into every canary. Static so the leak scanner can
/// fast-filter response bodies before doing the substring scan.
pub const CANARY_PREFIX: &str = "HAMM0R-";

/// Deterministic per-(run, session) canary string.
///
/// Inputs are folded together with a `:` separator that never appears in
/// run_id / scenario_id (kebab-case) and is not in `session_idx` (an
/// integer), so the joined string maps unambiguously back to a single
/// (run, session, scenario) tuple.
pub fn generate(run_id: &str, session_idx: u32, scenario_id: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(run_id.as_bytes());
    hasher.update(b":");
    hasher.update(session_idx.to_string().as_bytes());
    hasher.update(b":");
    hasher.update(scenario_id.as_bytes());
    let digest = hasher.finalize();

    let mut out = String::with_capacity(CANARY_PREFIX.len() + HEX_LEN);
    out.push_str(CANARY_PREFIX);
    for byte in digest.iter().take(HEX_LEN.div_ceil(2)) {
        out.push_str(&format!("{byte:02x}"));
    }
    out.truncate(CANARY_PREFIX.len() + HEX_LEN);
    out
}

/// Substitute `{{canary}}` (with any whitespace inside the braces) for
/// `canary` in `text`. Used by the multi-session orchestrator to plant
/// the canary into prompt text before it reaches the template engine.
/// Done as a literal string pass (not a minijinja render) so the
/// payload's other braces stay opaque — attack prompts often contain
/// other `{{ }}` markers we don't want to evaluate.
pub fn inject(text: &str, canary: &str) -> String {
    // Hand-rolled rather than regex: avoids needing to escape the canary
    // in the replacement and keeps the substitution semantics obvious.
    let mut out = String::with_capacity(text.len());
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'{' && i + 1 < bytes.len() && bytes[i + 1] == b'{' {
            // Skip leading whitespace inside `{{ ... }}`.
            let mut j = i + 2;
            while j < bytes.len() && (bytes[j] == b' ' || bytes[j] == b'\t') {
                j += 1;
            }
            if bytes[j..].starts_with(b"canary") {
                let mut k = j + b"canary".len();
                while k < bytes.len() && (bytes[k] == b' ' || bytes[k] == b'\t') {
                    k += 1;
                }
                if k + 1 < bytes.len() && bytes[k] == b'}' && bytes[k + 1] == b'}' {
                    out.push_str(canary);
                    i = k + 2;
                    continue;
                }
            }
        }
        // Use char boundaries; bytes[i] is ASCII for the markers above,
        // so we can advance by the UTF-8 width of whatever's here.
        let ch = text[i..].chars().next().expect("char at byte index");
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canary_format_is_prefix_plus_11_hex() {
        let c = generate("run-001", 0, "scn-a");
        assert!(c.starts_with(CANARY_PREFIX));
        assert_eq!(c.len(), CANARY_PREFIX.len() + HEX_LEN);
        for ch in c[CANARY_PREFIX.len()..].chars() {
            assert!(ch.is_ascii_hexdigit(), "non-hex char: {ch}");
        }
    }

    #[test]
    fn canary_is_deterministic_for_same_inputs() {
        let a = generate("run-001", 1, "scn-x");
        let b = generate("run-001", 1, "scn-x");
        assert_eq!(a, b);
    }

    #[test]
    fn canary_differs_across_sessions() {
        let a = generate("run-001", 0, "scn-x");
        let b = generate("run-001", 1, "scn-x");
        assert_ne!(a, b);
    }

    #[test]
    fn canary_differs_across_runs() {
        let a = generate("run-001", 0, "scn-x");
        let b = generate("run-002", 0, "scn-x");
        assert_ne!(a, b);
    }

    #[test]
    fn canary_differs_across_scenarios() {
        let a = generate("run-001", 0, "scn-a");
        let b = generate("run-001", 0, "scn-b");
        assert_ne!(a, b);
    }

    #[test]
    fn inject_replaces_canary_marker() {
        let out = inject("hello {{canary}}!", "HAMM0R-abc");
        assert_eq!(out, "hello HAMM0R-abc!");
    }

    #[test]
    fn inject_tolerates_whitespace_inside_braces() {
        let out = inject("x {{ canary }} y", "C");
        assert_eq!(out, "x C y");
    }

    #[test]
    fn inject_leaves_other_template_markers_alone() {
        let out = inject("prefix {{prompt}} {{canary}} {{ env.X }}", "K");
        assert_eq!(out, "prefix {{prompt}} K {{ env.X }}");
    }

    #[test]
    fn inject_with_no_marker_returns_text_unchanged() {
        let text = "no canary here";
        assert_eq!(inject(text, "K"), text);
    }
}
