//! Dev-only Ollama backend for the judge.
//!
//! This module exists so we can run the analyzer end-to-end on a Windows
//! box that lacks the MSVC + CMake toolchain needed to build
//! `llama-cpp-2`. It calls Ollama's HTTP API instead of loading a GGUF
//! in-process. Architecturally divergent — Stack.md forbids sidecars in
//! shipped builds — so this is gated behind a runtime URL: pass `None`
//! and the rest of the pipeline never touches it.
//!
//! The contract matches `LlmJudge` so callers can swap backends without
//! reshaping the orchestration in `pipeline.rs`.
//!
//! See `docs/analyzorPlan.md` for the full discussion of the dev path.
//!
//! ## Wire format
//!
//!   POST {base_url}/api/generate
//!   { "model": "...", "prompt": "...", "stream": false }
//!
//!   200 { "response": "...", ... }

use serde::{Deserialize, Serialize};

/// HTTP-backed judge that talks to a local Ollama instance.
pub struct OllamaJudge {
    client: reqwest::blocking::Client,
    base_url: String,
    pub model_id: String,
}

impl OllamaJudge {
    /// Construct an `OllamaJudge` pointing at `base_url` (e.g.
    /// `http://localhost:11434`) and using `model_id` as the Ollama
    /// model tag (e.g. `qwen2.5:3b`).
    pub fn new(base_url: &str, model_id: &str) -> anyhow::Result<Self> {
        let client = reqwest::blocking::Client::builder()
            // No connect-timeout: localhost should be instant; let
            // request-timeout cover the slow generation path.
            .timeout(std::time::Duration::from_secs(120))
            .build()
            .map_err(|e| anyhow::anyhow!("ollama client init: {e}"))?;
        Ok(Self {
            client,
            base_url: base_url.trim_end_matches('/').to_owned(),
            model_id: model_id.to_owned(),
        })
    }

    /// Send the judge prompt to Ollama and return the raw model output.
    ///
    /// Caller parses the result as JSON; this matches `LlmJudge::infer`.
    pub fn infer(&self, judge_prompt: &str) -> anyhow::Result<String> {
        let url = format!("{}/api/generate", self.base_url);
        let req = GenerateRequest {
            model: &self.model_id,
            prompt: judge_prompt,
            stream: false,
            // Greedy decoding for reproducibility and to discourage the
            // "creative-prose" failure mode where the model writes around
            // the requested JSON. Matches the `LlamaSampler::greedy()`
            // setting in `llm.rs`.
            options: GenerateOptions { temperature: 0.0 },
        };
        let resp = self
            .client
            .post(&url)
            .json(&req)
            .send()
            .map_err(|e| anyhow::anyhow!("POST {url}: {e}"))?;
        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().unwrap_or_default();
            anyhow::bail!("ollama returned {status}: {body}");
        }
        let parsed: GenerateResponse = resp
            .json()
            .map_err(|e| anyhow::anyhow!("ollama response decode: {e}"))?;
        Ok(parsed.response)
    }
}

#[derive(Serialize)]
struct GenerateRequest<'a> {
    model: &'a str,
    prompt: &'a str,
    stream: bool,
    options: GenerateOptions,
}

#[derive(Serialize)]
struct GenerateOptions {
    temperature: f32,
}

#[derive(Deserialize)]
struct GenerateResponse {
    response: String,
}

/// Run the judge prompt through Ollama and return a parsed `JudgeOutput`.
/// Mirrors `judge_with_llm` in `lib.rs` so the pipeline can swap backends
/// without restructuring its dispatch logic.
pub fn judge_with_ollama(
    input: &crate::JudgeInput,
    judge: &OllamaJudge,
) -> Result<crate::JudgeOutput, crate::AnalyzerError> {
    let judge_prompt = crate::build_judge_prompt(input)?;
    let raw = judge
        .infer(&judge_prompt)
        .map_err(|e| crate::AnalyzerError::LlmInference(e.to_string()))?;
    Ok(parse_response(&raw, &judge.model_id))
}

fn parse_response(raw: &str, model_id: &str) -> crate::JudgeOutput {
    use storage::verdicts::JudgeVerdict;

    #[derive(Deserialize)]
    struct LlmJson {
        verdict: String,
        confidence: Option<f32>,
        reason: Option<String>,
    }

    // The model occasionally wraps JSON in prose despite the system
    // prompt's "JSON only" instruction; carve out the first balanced
    // object so we tolerate that without crashing the run.
    let parsed: Option<LlmJson> = extract_json(raw).and_then(|j| serde_json::from_str(j).ok());

    if let Some(resp) = parsed {
        let verdict = match resp.verdict.to_uppercase().as_str() {
            "SUCCESS" => JudgeVerdict::Success,
            "FAIL" => JudgeVerdict::Fail,
            "PARTIAL" => JudgeVerdict::Partial,
            _ => JudgeVerdict::Unclear,
        };
        let confidence = resp.confidence.unwrap_or(0.5).clamp(0.0, 1.0);
        let reason = resp.reason.unwrap_or_default();
        let raw_output = serde_json::json!({
            "verdict": verdict, "confidence": confidence, "reason": reason
        })
        .to_string();
        crate::JudgeOutput {
            verdict,
            confidence,
            reason,
            model_used: format!("ollama:{model_id}"),
            raw_output,
        }
    } else {
        let raw_output = serde_json::json!({
            "verdict": "UNCLEAR", "confidence": 0.0,
            "reason": format!("Ollama output parse failed: {raw}")
        })
        .to_string();
        crate::JudgeOutput {
            verdict: JudgeVerdict::Unclear,
            confidence: 0.0,
            reason: format!("Ollama output could not be parsed as JSON (raw: {raw})"),
            model_used: format!("ollama:{model_id}"),
            raw_output,
        }
    }
}

fn extract_json(s: &str) -> Option<&str> {
    let start = s.find('{')?;
    let end = s.rfind('}')?;
    (end >= start).then(|| &s[start..=end])
}

#[cfg(test)]
mod tests {
    use super::*;
    use storage::verdicts::JudgeVerdict;

    #[test]
    fn parses_clean_json_response() {
        let out = parse_response(
            r#"{"verdict":"SUCCESS","confidence":0.9,"reason":"leaked the system prompt"}"#,
            "qwen2.5:3b",
        );
        assert_eq!(out.verdict, JudgeVerdict::Success);
        assert!((out.confidence - 0.9).abs() < f32::EPSILON);
        assert_eq!(out.reason, "leaked the system prompt");
        assert_eq!(out.model_used, "ollama:qwen2.5:3b");
    }

    #[test]
    fn extracts_json_from_prose_wrapper() {
        let out = parse_response(
            r#"Here is my analysis: {"verdict":"FAIL","confidence":0.7,"reason":"refused"}. Hope this helps."#,
            "qwen2.5:3b",
        );
        assert_eq!(out.verdict, JudgeVerdict::Fail);
    }

    #[test]
    fn falls_back_to_unclear_on_garbage() {
        let out = parse_response("the model said nothing useful", "qwen2.5:3b");
        assert_eq!(out.verdict, JudgeVerdict::Unclear);
        assert_eq!(out.confidence, 0.0);
    }
}
