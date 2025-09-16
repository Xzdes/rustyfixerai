use anyhow::{anyhow, Context, Result};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde::de::DeserializeOwned;
use std::env;
use std::fmt::Debug;

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AnalysisPlan {
    pub error_summary: String,
    pub search_queries: Vec<String>,
    pub involved_crate: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct CargoSuggestionDetails {
    pub crate_name: String,
    pub version: String,
    pub features: Vec<String>,
}

pub struct LLMInterface {
    http_async: Client,
    base_url: String,
    model: String,
    timeout_secs: u64,
}

#[derive(Debug, Deserialize)]
struct OllamaChatResponse {
    message: OllamaMessage,
}

#[derive(Debug, Deserialize)]
struct OllamaMessage {
    content: String,
}

impl LLMInterface {
    pub fn new() -> Result<Self> {
        let base_url = env::var("OLLAMA_BASE_URL").unwrap_or_else(|_| "http://127.0.0.1:11434".to_string());
        let model = env::var("OLLAMA_MODEL").unwrap_or_else(|_| "llama3:8b".to_string());

        Ok(Self {
            http_async: Client::builder().build()?,
            base_url,
            model,
            timeout_secs: 120,
        })
    }

    async fn chat(&self, prompt: &str, format: &str) -> Result<String> {
        // format == "json" → пытаемся попросить модель отвечать JSON-ом
        let url = format!("{}/api/chat", self.base_url);
        let body = serde_json::json!({
          "model": self.model,
          "messages": [{"role":"user", "content": prompt}],
          "options": { "temperature": 0.2 },
          "format": if format.is_empty() { serde_json::Value::Null } else { serde_json::json!(format) }
        });

        let res = self.http_async.post(&url).json(&body).send().await?;
        if !res.status().is_success() {
            return Err(anyhow!("Ollama API request failed with status {}", res.status()));
        }
        let parsed = res.json::<OllamaChatResponse>().await
            .context("Failed to parse Ollama response")?;
        Ok(parsed.message.content)
    }

    async fn request_json<T: DeserializeOwned + Debug>(&self, initial_prompt: &str) -> Result<T> {
        let raw = self.chat(initial_prompt, "json").await?;
        if let Ok(parsed) = serde_json::from_str::<T>(&raw) {
            return Ok(parsed);
        }
        // Попытка самовосстановления JSON
        let extractor = format!(
            "Extract only the valid JSON object from the following text. Do not add anything.\n---\n{}\n---",
            raw
        );
        let cleaned = self.chat(&extractor, "json").await?;
        serde_json::from_str::<T>(&cleaned).map_err(|e| {
            anyhow!("Failed to parse JSON.\nError: {e}\nRaw: {raw}\nCleaned: {cleaned}")
        })
    }

    pub async fn analyze_error(&self, error_message: &str) -> Result<AnalysisPlan> {
        let prompt = format!(r#"
Analyze a Rust compiler error and create a plan.
Rules:
1) Return a JSON object with keys "error_summary", "search_queries", "involved_crate".
Compiler Error:
{error_message}
"#);
        self.request_json(&prompt).await
    }

    pub async fn generate_full_fix(&self, error_message: &str, full_code: &str, web_context: &str) -> Result<String> {
        let prompt = format!(r#"
Fix the Rust code.
RULES:
1) Your output MUST BE ONLY the complete, corrected, full source code for the file.
2) No explanations or markdown.

--- COMPILER ERROR ---
{error_message}
--- FULL SOURCE CODE ---
{full_code}
--- CONTEXT FROM ONLINE SEARCH ---
{web_context}
---
Your Corrected Full Source Code:
"#);
        let raw = self.chat(&prompt, "").await?;
        Ok(raw.trim()
            .trim_start_matches("```rust")
            .trim_start_matches("```")
            .trim_end_matches("```")
            .trim()
            .to_string())
    }

    pub async fn generate_cargo_fix(&self, error_message: &str) -> Result<CargoSuggestionDetails> {
        let prompt = format!(r#"
Analyze a Rust error about a missing dependency.
TASK: Extract the crate name, a suitable version, and any required features.
CRITICAL RULES:
1) Return a valid JSON object with keys crate_name, version, features (array of strings).

Compiler error:
{error_message}
"#);
        self.request_json::<CargoSuggestionDetails>(&prompt).await
    }
}
