//! Analyzer manifest types and fetch helpers.
//!
//! The manifest lives at a fixed URL and tells the installer which
//! per-OS bundles exist, what they hash to, and what app version they
//! require. There is no offline fallback: any "fallback" we shipped
//! would either carry placeholder hashes that fail verification
//! (confusing) or hard-coded real hashes that go stale (dangerous).

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnalyzerManifest {
    pub version: u32,
    pub generated_at: String,
    pub minimum_hamm0r_version: String,
    pub variants: Vec<AnalyzerVariant>,
}

/// One installable analyzer variant. The bundle artifact is a single
/// archive containing the analyz0r binary, any runtime assets, and the
/// model file — see `docs/analyzorPlan.md` (Phase 2 install layout).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AnalyzerVariant {
    pub id: String,
    pub label: String,
    /// Operating system tag, e.g. `"windows"`, `"macos"`, `"linux"`.
    pub os: String,
    /// CPU architecture tag, e.g. `"x86_64"`, `"aarch64"`.
    pub arch: String,
    pub hardware: HardwareClass,
    pub recommended: bool,
    /// Logical model identifier (separate from on-disk file name) — the
    /// bundle is responsible for placing the model where the runtime
    /// expects it.
    pub model_id: String,
    /// The bundle archive that gets downloaded, verified, and extracted.
    pub bundle: Artifact,
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

/// Where hamm0r fetches the analyzer manifest. The manifest is a small
/// JSON file kept in this repo at `analyzer/manifest.json` and served
/// by GitHub's raw-content CDN. Bundle zips referenced by the manifest
/// live as GitHub Release assets — see `analyzer/README.md` for the
/// publish recipe.
pub const MANIFEST_URL: &str =
    "https://raw.githubusercontent.com/t0nigansel/hamm0r/main/analyzer/manifest.json";

/// Fetch the manifest for use inside a spawned task. The Tauri-facing
/// version of the same logic with logger plumbing lives in `mod.rs`.
pub async fn fetch_manifest_internal() -> anyhow::Result<AnalyzerManifest> {
    let resp = reqwest::get(MANIFEST_URL).await
        .map_err(|e| anyhow::anyhow!("could not reach analyzer manifest at {MANIFEST_URL}: {e}"))?;
    if !resp.status().is_success() {
        anyhow::bail!(
            "analyzer manifest fetch returned HTTP {} — try again later",
            resp.status()
        );
    }
    resp.json::<AnalyzerManifest>().await
        .map_err(|e| anyhow::anyhow!("analyzer manifest is not valid JSON: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn variant_serialises_with_new_bundle_shape() {
        let v = AnalyzerVariant {
            id: "v1".into(),
            label: "v1".into(),
            os: "linux".into(),
            arch: "x86_64".into(),
            hardware: HardwareClass::Generic,
            recommended: false,
            model_id: "qwen-test".into(),
            bundle: Artifact {
                url: "https://example/x.zip".into(),
                sha256: "abc".into(),
                size_bytes: 42,
            },
        };
        let json = serde_json::to_value(&v).unwrap();
        // Bundle is a single object, not split into model+runtime.
        assert!(json.get("bundle").is_some());
        assert!(json.get("model").is_none());
        assert!(json.get("runtime").is_none());
        assert_eq!(json["os"], "linux");
        assert_eq!(json["arch"], "x86_64");
        assert_eq!(json["model_id"], "qwen-test");
    }
}
