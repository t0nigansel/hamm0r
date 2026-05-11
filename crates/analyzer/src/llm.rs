use std::num::NonZeroU32;
use std::path::{Path, PathBuf};

use llama_cpp_2::{
    context::params::LlamaContextParams,
    llama_backend::LlamaBackend,
    llama_batch::LlamaBatch,
    model::{params::LlamaModelParams, AddBos, LlamaModel},
    sampling::LlamaSampler,
};

const MAX_NEW_TOKENS: usize = 512;
const CTX_SIZE: u32 = 4096;

pub struct LlmJudge {
    backend: LlamaBackend,
    model: LlamaModel,
    /// Human-readable model name derived from the file stem.
    pub model_id: String,
}

impl LlmJudge {
    pub fn load(model_path: &Path) -> anyhow::Result<Self> {
        let backend =
            LlamaBackend::init().map_err(|e| anyhow::anyhow!("llama backend init: {e}"))?;

        let model_params = LlamaModelParams::default().with_n_gpu_layers(99);
        let model = LlamaModel::load_from_file(&backend, model_path, &model_params)
            .map_err(|e| anyhow::anyhow!("model load from {}: {e}", model_path.display()))?;

        let model_id = model_path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("unknown-llm")
            .to_owned();

        Ok(Self {
            backend,
            model,
            model_id,
        })
    }

    /// Run the judge prompt through the model and return the raw text output.
    ///
    /// The caller is responsible for parsing the returned string as JSON.
    pub fn infer(&self, judge_prompt: &str) -> anyhow::Result<String> {
        let prompt = format_qwen25_prompt(judge_prompt);

        let ctx_params = LlamaContextParams::default().with_n_ctx(NonZeroU32::new(CTX_SIZE));
        let mut ctx = self
            .model
            .new_context(&self.backend, ctx_params)
            .map_err(|e| anyhow::anyhow!("context create: {e}"))?;

        let tokens = self
            .model
            .str_to_token(&prompt, AddBos::Always)
            .map_err(|e| anyhow::anyhow!("tokenize: {e}"))?;
        let n_tokens = tokens.len();
        anyhow::ensure!(n_tokens > 0, "prompt tokenized to zero tokens");

        let mut batch = LlamaBatch::new(n_tokens, 1);
        for (i, &tok) in tokens.iter().enumerate() {
            batch
                .add(tok, i as i32, &[0], i == n_tokens - 1)
                .map_err(|e| anyhow::anyhow!("batch add: {e}"))?;
        }
        ctx.decode(&mut batch)
            .map_err(|e| anyhow::anyhow!("initial decode: {e}"))?;

        let mut sampler = LlamaSampler::greedy();
        let eos = self.model.token_eos();
        let mut output_bytes: Vec<u8> = Vec::with_capacity(256);
        let mut n_cur = n_tokens;

        // Sample the first token from the initial batch (last position).
        let mut sample_idx = batch.n_tokens() - 1;

        for _ in 0..MAX_NEW_TOKENS {
            let token = sampler.sample(&ctx, sample_idx);

            if token == eos {
                break;
            }

            let piece = self
                .model
                .token_to_piece_bytes(token, 32, false, None)
                .map_err(|e| anyhow::anyhow!("token_to_piece: {e}"))?;
            output_bytes.extend_from_slice(&piece);

            // Stop once we have a complete JSON object (balanced braces).
            if is_complete_json(&output_bytes) {
                break;
            }

            batch.clear();
            batch
                .add(token, n_cur as i32, &[0], true)
                .map_err(|e| anyhow::anyhow!("batch add loop: {e}"))?;
            ctx.decode(&mut batch)
                .map_err(|e| anyhow::anyhow!("decode loop: {e}"))?;
            sample_idx = 0; // single token in the batch from here on
            n_cur += 1;
        }

        Ok(String::from_utf8_lossy(&output_bytes).into_owned())
    }
}

/// Scan `models_dir` for the first `.gguf` file and return its path.
pub fn find_model_file(models_dir: &Path) -> Option<PathBuf> {
    std::fs::read_dir(models_dir)
        .ok()?
        .flatten()
        .find_map(|entry| {
            let path = entry.path();
            (path.extension().and_then(|x| x.to_str()) == Some("gguf")).then_some(path)
        })
}

/// Format a judge prompt using the Qwen2.5 ChatML template.
fn format_qwen25_prompt(user_prompt: &str) -> String {
    format!(
        "<|im_start|>system\n\
         You are a concise security evaluation assistant. \
         Respond with valid JSON only — no prose, no code fences, nothing else.\
         <|im_end|>\n\
         <|im_start|>user\n\
         {user_prompt}\
         <|im_end|>\n\
         <|im_start|>assistant\n"
    )
}

/// Returns `true` once the accumulated output contains a balanced JSON object.
fn is_complete_json(bytes: &[u8]) -> bool {
    let opens = bytes.iter().filter(|&&b| b == b'{').count();
    let closes = bytes.iter().filter(|&&b| b == b'}').count();
    opens > 0 && opens == closes
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn complete_json_detection() {
        assert!(!is_complete_json(b""));
        assert!(!is_complete_json(b"{\"a\":1"));
        assert!(is_complete_json(b"{\"a\":1}"));
        assert!(is_complete_json(b"  {\"a\":{\"b\":2}}  "));
    }

    #[test]
    fn format_includes_chatml_tokens() {
        let p = format_qwen25_prompt("test");
        assert!(p.contains("<|im_start|>system"));
        assert!(p.contains("<|im_start|>user"));
        assert!(p.contains("<|im_start|>assistant"));
        assert!(p.contains("test"));
    }
}
