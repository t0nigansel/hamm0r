use reqwest::blocking::Client;
use reqwest::header::{HeaderMap, HeaderValue, CONTENT_TYPE};
use serde::Deserialize;

use crate::{build_judge_prompt, parse_model_output, AnalyzerError, JudgeInput, JudgeOutput};

#[derive(Debug, Clone)]
pub enum HostedJudgeProvider {
    AzureOpenAi,
}

#[derive(Debug, Clone)]
pub enum HostedJudgeApiStyle {
    Auto,
    ChatCompletions,
    Responses,
}

#[derive(Debug, Clone)]
pub struct HostedJudgeConfig {
    pub provider: HostedJudgeProvider,
    pub endpoint: String,
    pub deployment: String,
    pub api_style: HostedJudgeApiStyle,
    pub api_version: Option<String>,
    pub api_key: String,
    pub max_input_chars: u32,
    pub max_output_tokens: u32,
    pub request_timeout_seconds: u32,
    pub max_retries: u32,
}

#[derive(Debug, Clone, Copy)]
pub struct HostedJudgeConfigInput<'a> {
    pub provider: &'a str,
    pub endpoint: &'a str,
    pub deployment: &'a str,
    pub api_style: &'a str,
    pub api_version: Option<&'a str>,
    pub api_key: &'a str,
    pub max_input_chars: u32,
    pub max_output_tokens: u32,
    pub request_timeout_seconds: u32,
    pub max_retries: u32,
}

impl HostedJudgeConfig {
    pub fn model_used(&self) -> String {
        match self.provider {
            HostedJudgeProvider::AzureOpenAi => {
                format!("azure_openai:{}", self.deployment.trim())
            }
        }
    }
}

pub fn judge_with_hosted(
    input: &JudgeInput,
    config: &HostedJudgeConfig,
) -> Result<JudgeOutput, AnalyzerError> {
    let prompt = build_judge_prompt(input)?;
    let prompt = trim_to_chars(&prompt, config.max_input_chars as usize);

    match config.provider {
        HostedJudgeProvider::AzureOpenAi => judge_with_azure(&prompt, config),
    }
}

fn judge_with_azure(
    prompt: &str,
    config: &HostedJudgeConfig,
) -> Result<JudgeOutput, AnalyzerError> {
    match config.api_style {
        HostedJudgeApiStyle::ChatCompletions => judge_with_azure_chat_completions(prompt, config),
        HostedJudgeApiStyle::Responses => judge_with_azure_responses(prompt, config),
        HostedJudgeApiStyle::Auto => match judge_with_azure_chat_completions(prompt, config) {
            Ok(output) => Ok(output),
            Err(chat_err) => match judge_with_azure_responses(prompt, config) {
                Ok(output) => Ok(output),
                Err(resp_err) => Err(AnalyzerError::LlmInference(format!(
                    "Hosted Judge auto mode failed for both chat_completions and responses. chat_completions: {}; responses: {}",
                    chat_err, resp_err
                ))),
            },
        },
    }
}

fn judge_with_azure_chat_completions(
    prompt: &str,
    config: &HostedJudgeConfig,
) -> Result<JudgeOutput, AnalyzerError> {
    let endpoint = config.endpoint.trim().trim_end_matches('/');
    let deployment = config.deployment.trim();
    if endpoint.is_empty() {
        return Err(AnalyzerError::LlmInference(
            "Hosted Judge endpoint is empty.".to_owned(),
        ));
    }
    if deployment.is_empty() {
        return Err(AnalyzerError::LlmInference(
            "Hosted Judge deployment is empty.".to_owned(),
        ));
    }
    if config.api_key.trim().is_empty() {
        return Err(AnalyzerError::LlmInference(
            "Hosted Judge API key is empty.".to_owned(),
        ));
    }

    let api_version = config
        .api_version
        .as_deref()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or("2024-10-21");
    let url = format!(
        "{endpoint}/openai/deployments/{deployment}/chat/completions?api-version={api_version}"
    );
    let client = build_client(config)?;

    let body = build_azure_chat_completions_body(prompt, config.max_output_tokens);

    let attempts = config.max_retries.max(1);
    let mut last_error = None;
    for _ in 0..attempts {
        match client.post(&url).json(&body).send() {
            Ok(response) => {
                if !response.status().is_success() {
                    let status = response.status();
                    let body = response.text().unwrap_or_default();
                    return Err(AnalyzerError::LlmInference(format!(
                        "Hosted Judge request failed with HTTP {}: {}",
                        status,
                        body.trim()
                    )));
                }
                let payload: AzureChatCompletionResponse = response.json().map_err(|e| {
                    AnalyzerError::LlmInference(format!("Hosted Judge response decode failed: {e}"))
                })?;
                let content = payload
                    .choices
                    .first()
                    .and_then(|choice| choice.message.content.as_deref())
                    .unwrap_or_default();
                return Ok(parse_model_output(content, &config.model_used()));
            }
            Err(err) => {
                last_error = Some(err);
            }
        }
    }

    Err(AnalyzerError::LlmInference(format!(
        "Hosted Judge request failed: {}",
        last_error
            .map(|e| e.to_string())
            .unwrap_or_else(|| "unknown network error".to_owned())
    )))
}

fn judge_with_azure_responses(
    prompt: &str,
    config: &HostedJudgeConfig,
) -> Result<JudgeOutput, AnalyzerError> {
    let endpoint = config.endpoint.trim().trim_end_matches('/');
    let deployment = config.deployment.trim();
    if endpoint.is_empty() {
        return Err(AnalyzerError::LlmInference(
            "Hosted Judge endpoint is empty.".to_owned(),
        ));
    }
    if deployment.is_empty() {
        return Err(AnalyzerError::LlmInference(
            "Hosted Judge deployment is empty.".to_owned(),
        ));
    }
    if config.api_key.trim().is_empty() {
        return Err(AnalyzerError::LlmInference(
            "Hosted Judge API key is empty.".to_owned(),
        ));
    }

    let url = format!("{endpoint}/openai/v1/responses");
    let client = build_client(config)?;
    let body = serde_json::json!({
        "model": deployment,
        "input": prompt,
        "max_output_tokens": config.max_output_tokens,
    });

    let attempts = config.max_retries.max(1);
    let mut last_error = None;
    for _ in 0..attempts {
        match client.post(&url).json(&body).send() {
            Ok(response) => {
                if !response.status().is_success() {
                    let status = response.status();
                    let body = response.text().unwrap_or_default();
                    return Err(AnalyzerError::LlmInference(format!(
                        "Hosted Judge responses request failed with HTTP {}: {}",
                        status,
                        body.trim()
                    )));
                }
                let payload: AzureResponsesResponse = response.json().map_err(|e| {
                    AnalyzerError::LlmInference(format!(
                        "Hosted Judge responses decode failed: {e}"
                    ))
                })?;
                let content = payload
                    .output_text
                    .unwrap_or_else(|| extract_output_text(&payload.output));
                return Ok(parse_model_output(&content, &config.model_used()));
            }
            Err(err) => {
                last_error = Some(err);
            }
        }
    }

    Err(AnalyzerError::LlmInference(format!(
        "Hosted Judge responses request failed: {}",
        last_error
            .map(|e| e.to_string())
            .unwrap_or_else(|| "unknown network error".to_owned())
    )))
}

fn build_client(config: &HostedJudgeConfig) -> Result<Client, AnalyzerError> {
    let mut headers = HeaderMap::new();
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    headers.insert(
        "api-key",
        HeaderValue::from_str(config.api_key.trim()).map_err(|e| {
            AnalyzerError::LlmInference(format!("Hosted Judge API key header is invalid: {e}"))
        })?,
    );

    Client::builder()
        .timeout(std::time::Duration::from_secs(u64::from(
            config.request_timeout_seconds.max(1),
        )))
        .default_headers(headers)
        .build()
        .map_err(|e| AnalyzerError::LlmInference(format!("Hosted Judge client init failed: {e}")))
}

fn build_azure_chat_completions_body(prompt: &str, max_output_tokens: u32) -> serde_json::Value {
    serde_json::json!({
        "messages": [
            {
                "role": "user",
                "content": prompt,
            }
        ],
        "max_completion_tokens": max_output_tokens,
        "response_format": { "type": "json_object" },
    })
}

fn trim_to_chars(input: &str, max_chars: usize) -> String {
    if input.chars().count() <= max_chars {
        return input.to_owned();
    }
    input.chars().take(max_chars).collect()
}

#[derive(Debug, Deserialize)]
struct AzureChatCompletionResponse {
    choices: Vec<AzureChoice>,
}

#[derive(Debug, Deserialize)]
struct AzureChoice {
    message: AzureMessage,
}

#[derive(Debug, Deserialize)]
struct AzureMessage {
    content: Option<String>,
}

#[derive(Debug, Deserialize)]
struct AzureResponsesResponse {
    #[serde(default)]
    output_text: Option<String>,
    #[serde(default)]
    output: Vec<AzureResponsesOutput>,
}

#[derive(Debug, Deserialize)]
struct AzureResponsesOutput {
    #[serde(default)]
    content: Vec<AzureResponsesContent>,
}

#[derive(Debug, Deserialize)]
struct AzureResponsesContent {
    #[serde(rename = "type")]
    kind: Option<String>,
    text: Option<String>,
}

fn extract_output_text(output: &[AzureResponsesOutput]) -> String {
    output
        .iter()
        .flat_map(|item| item.content.iter())
        .find_map(|content| {
            (content.kind.as_deref() == Some("output_text"))
                .then(|| content.text.clone())
                .flatten()
        })
        .unwrap_or_default()
}

pub fn build_hosted_config(input: HostedJudgeConfigInput<'_>) -> anyhow::Result<HostedJudgeConfig> {
    let provider = match input.provider.trim().to_ascii_lowercase().as_str() {
        "azure_openai" => HostedJudgeProvider::AzureOpenAi,
        other => anyhow::bail!("unsupported Hosted Judge provider '{}'", other),
    };
    let api_style = match input.api_style.trim().to_ascii_lowercase().as_str() {
        "auto" => HostedJudgeApiStyle::Auto,
        "chat_completions" => HostedJudgeApiStyle::ChatCompletions,
        "responses" => HostedJudgeApiStyle::Responses,
        other => anyhow::bail!("unsupported Hosted Judge api style '{}'", other),
    };

    let config = HostedJudgeConfig {
        provider,
        endpoint: input.endpoint.trim().to_owned(),
        deployment: input.deployment.trim().to_owned(),
        api_style,
        api_version: input
            .api_version
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_owned),
        api_key: input.api_key.trim().to_owned(),
        max_input_chars: input.max_input_chars,
        max_output_tokens: input.max_output_tokens,
        request_timeout_seconds: input.request_timeout_seconds,
        max_retries: input.max_retries,
    };
    Ok(config)
}

#[cfg(test)]
mod tests {
    use super::build_azure_chat_completions_body;

    #[test]
    fn azure_chat_completions_body_uses_max_completion_tokens() {
        let body = build_azure_chat_completions_body("hello", 321);

        assert_eq!(body["max_completion_tokens"].as_u64(), Some(321));
        assert!(body.get("max_tokens").is_none());
        assert!(body.get("temperature").is_none());
        assert_eq!(body["messages"][0]["content"].as_str(), Some("hello"));
    }
}
