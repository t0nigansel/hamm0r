use serde::{Deserialize, Serialize};

/// Remote bundle manifest cached as `~/hamm0r/analyzer/manifest.json`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AnalyzerManifest {
    pub version: u32,
    pub generated_at: String,
    pub minimum_hamm0r_version: String,
    pub variants: Vec<AnalyzerVariant>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AnalyzerVariant {
    pub id: String,
    pub label: String,
    pub hardware: HardwareClass,
    pub recommended: bool,
    pub model: Artifact,
    /// Shared runtime library. `None` when llama-cpp is statically linked
    /// into the hamm0r binary (the default for released builds).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime: Option<Artifact>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HardwareClass {
    AppleSilicon,
    X86_64Avx2,
    Generic,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Artifact {
    pub url: String,
    pub sha256: String,
    pub size_bytes: u64,
}

// ── Manifest URL ──────────────────────────────────────────────────────────────

/// The URL from which hamm0r fetches the analyzer manifest.
/// TBD — will be set once hamm0r.io is live.
pub const MANIFEST_URL: &str = "https://hamm0r.io/analyzer/manifest.json";

/// Build the default manifest used for testing and the activation UI preview.
/// The model URLs point at the official Qwen2.5-3B-Instruct-GGUF repository
/// on Hugging Face. SHA256 values must be verified against the actual files
/// before a production release.
pub fn default_manifest() -> AnalyzerManifest {
    const HF_BASE: &str =
        "https://huggingface.co/Qwen/Qwen2.5-3B-Instruct-GGUF/resolve/main";
    const MODEL_FILE: &str = "qwen2.5-3b-instruct-q4_k_m.gguf";
    // ~1.93 GB — verified from the HF repo page at time of writing.
    const MODEL_SIZE: u64 = 1_930_000_000;
    // TODO: replace with real SHA256 once a download pipeline is wired up.
    const MODEL_SHA256: &str = "TODO-verify-sha256-from-huggingface";

    let model = Artifact {
        url: format!("{HF_BASE}/{MODEL_FILE}"),
        sha256: MODEL_SHA256.to_owned(),
        size_bytes: MODEL_SIZE,
    };

    AnalyzerManifest {
        version: 1,
        generated_at: "2026-04-27T00:00:00Z".to_owned(),
        minimum_hamm0r_version: "0.1.0".to_owned(),
        variants: vec![
            AnalyzerVariant {
                id: "qwen2.5-3b-q4-apple".to_owned(),
                label: "Qwen2.5 3B Q4_K_M (Apple Silicon — Metal)".to_owned(),
                hardware: HardwareClass::AppleSilicon,
                recommended: true,
                model: model.clone(),
                runtime: None,
            },
            AnalyzerVariant {
                id: "qwen2.5-3b-q4-x86".to_owned(),
                label: "Qwen2.5 3B Q4_K_M (x86-64 AVX2)".to_owned(),
                hardware: HardwareClass::X86_64Avx2,
                recommended: true,
                model: model.clone(),
                runtime: None,
            },
            AnalyzerVariant {
                id: "qwen2.5-3b-q4-generic".to_owned(),
                label: "Qwen2.5 3B Q4_K_M (CPU generic — slow)".to_owned(),
                hardware: HardwareClass::Generic,
                recommended: false,
                model,
                runtime: None,
            },
        ],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_roundtrip_json() {
        let manifest = default_manifest();
        let json = serde_json::to_string(&manifest).unwrap();
        let parsed: AnalyzerManifest = serde_json::from_str(&json).unwrap();
        assert_eq!(manifest, parsed);
    }

    #[test]
    fn runtime_omitted_when_none() {
        let manifest = default_manifest();
        let json = serde_json::to_string(&manifest).unwrap();
        assert!(!json.contains("\"runtime\""));
    }

    #[test]
    fn runtime_present_when_some() {
        let mut manifest = default_manifest();
        manifest.variants[0].runtime = Some(Artifact {
            url: "https://example.com/lib.dylib".into(),
            sha256: "abc".into(),
            size_bytes: 1_000_000,
        });
        let json = serde_json::to_string(&manifest).unwrap();
        assert!(json.contains("\"runtime\""));
        let parsed: AnalyzerManifest = serde_json::from_str(&json).unwrap();
        assert_eq!(manifest, parsed);
    }
}
