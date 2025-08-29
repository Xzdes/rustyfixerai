// src/modules/llm_interface.rs

use serde::{Deserialize, Serialize};
use anyhow::{Result, anyhow, Context};
use std::fmt::Debug;

// --- КОНФИГУРАЦИЯ ---
const OLLAMA_API_URL: &str = "http://127.0.0.1:11434/api/chat";
// Возвращаемся к вашей модели, как вы и просили
const LLM_MODEL_NAME: &str = "llama3:8b"; 

// --- OLLAMA API STRUCTURES ---
#[derive(Serialize)]
struct OllamaRequest<'a> {
    model: &'a str,
    messages: Vec<Message<'a>>,
    stream: bool,
    format: &'a str,
}

#[derive(Serialize)]
struct Message<'a> {
    role: &'a str,
    content: &'a str,
}

#[derive(Deserialize)]
struct OllamaResponse {
    message: ResponseMessage,
}

#[derive(Deserialize)]
struct ResponseMessage {
    content: String,
}

// --- НОВАЯ, УЛЬТРА-НАДЕЖНАЯ СТРУКТУРА ДАННЫХ ---
#[derive(Deserialize, Debug)]
pub struct CargoSuggestionDetails {
    pub crate_name: String,
    pub version: String,
    pub features: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub struct AnalysisPlan {
    pub error_summary: String,
    pub search_queries: Vec<String>,
    pub involved_crate: Option<String>,
}

pub struct LLMInterface {
    client: reqwest::Client,
}

impl LLMInterface {
    pub fn new() -> Self {
        Self {
            client: reqwest::Client::new(),
        }
    }
    
    async fn make_ollama_request(&self, prompt: &str, format: &str) -> Result<String> {
        let request_payload = OllamaRequest {
            model: LLM_MODEL_NAME,
            messages: vec![Message { role: "user", content: prompt }],
            stream: false,
            format,
        };

        let res = self.client.post(OLLAMA_API_URL)
            .json(&request_payload)
            .send()
            .await
            .context("Failed to send request to Ollama")?;

        if !res.status().is_success() {
            return Err(anyhow!("Ollama API request failed with status {}", res.status()));
        }

        let ollama_response = res.json::<OllamaResponse>().await.context("Failed to parse Ollama response shell")?;
        Ok(ollama_response.message.content)
    }

    async fn send_request_and_extract_json<T: for<'de> Deserialize<'de> + Debug>(
        &self,
        initial_prompt: &str,
    ) -> Result<T> {
        let raw_response = self.make_ollama_request(initial_prompt, "json").await?;
        if let Ok(parsed) = serde_json::from_str::<T>(&raw_response) {
            return Ok(parsed);
        }

        println!("    -> LLM response was not clean JSON. Attempting self-extraction...");
        let extraction_prompt = format!(
            "Extract only the valid JSON object from the following text. Do not add any explanation, conversational text, or markdown. Only the JSON object itself.\n\nTEXT:\n---\n{}\n---",
            raw_response
        );
        let cleaner_response = self.make_ollama_request(&extraction_prompt, "json").await?;
        serde_json::from_str::<T>(&cleaner_response).map_err(|e| {
            anyhow!(
                "Failed to parse JSON even after self-extraction.\nError: {}\nOriginal Response: '{}'\nCleaned Response: '{}'",
                e, raw_response, cleaner_response
            )
        })
    }
    
    pub async fn analyze_error(&self, error_message: &str) -> Result<AnalysisPlan> {
        let prompt = format!(
            r#"Analyze a Rust compiler error and create a plan.
**CRITICAL RULES:**
1. Your output MUST be a valid JSON object.
2. The JSON must have keys "error_summary", "search_queries", and "involved_crate".

**Compiler Error:**
"{}"
"#,
            error_message
        );
        self.send_request_and_extract_json(&prompt).await
    }

    pub async fn generate_full_fix(&self, error_message: &str, full_code: &str, web_context: &str) -> Result<String> {
        let prompt = format!(
            r#"Fix the Rust code.
**RULES:**
1. Your output MUST BE ONLY the complete, corrected, full source code for the file. 
2. Do not add explanations or markdown.

--- COMPILER ERROR ---
{}
--- FULL SOURCE CODE ---
{}
--- CONTEXT FROM ONLINE SEARCH ---
{}
---
Your Corrected Full Source Code:"#,
            error_message, full_code, web_context
        );
        let raw_fix = self.make_ollama_request(&prompt, "").await?;
        Ok(raw_fix.trim().trim_start_matches("```rust").trim_start_matches("```").trim_end_matches("```").trim().to_string())
    }
    
    // --- ПОЛНОСТЬЮ НОВЫЙ ПРОМПТ ДЛЯ ИЗВЛЕЧЕНИЯ СЫРЫХ ДАННЫХ ---
    pub async fn generate_cargo_fix(&self, error_message: &str) -> Result<CargoSuggestionDetails> {
        let prompt = format!(
            r#"Analyze a Rust error about a missing dependency.
**TASK:** Extract the crate name, a suitable version, and any required features.
**CRITICAL RULES:**
1. Your output MUST be a valid JSON object.
2. The JSON must have keys: `crate_name` (string), `version` (string, e.g., "1.0"), and `features` (an array of strings, e.g., ["derive"]). If no features are needed, return an empty array.

--- COMPILER ERROR ---
{}

--- JSON OUTPUT EXAMPLE ---
{{
  "crate_name": "serde",
  "version": "1.0",
  "features": ["derive"]
}}
"#,
            error_message
        );
        self.send_request_and_extract_json(&prompt).await
    }
}