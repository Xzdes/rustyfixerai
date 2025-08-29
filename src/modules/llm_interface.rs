// src/modules/llm_interface.rs

use serde::{Deserialize, Serialize};
use anyhow::{Result, anyhow, Context}; // <-- ИСПРАВЛЕНИЕ ЗДЕСЬ
use std::fmt::Debug;

// --- CONFIGURATION ---
const OLLAMA_API_URL: &str = "http://127.0.0.1:11434/api/chat";
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

#[derive(Deserialize, Debug)]
pub struct OllamaCargoSuggestion {
    pub crate_name: String,
    pub dependency_line: String,
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

    /// Внутренний метод для отправки запроса в Ollama.
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

    /// **КЛЮЧЕВОЙ НОВЫЙ МЕТОД** с логикой самоочистки.
    async fn send_request_and_extract_json<T: for<'de> Deserialize<'de> + Debug>(
        &self,
        initial_prompt: &str,
    ) -> Result<T> {
        // --- ШАГ 1: Первая попытка получить ответ ---
        let raw_response = self.make_ollama_request(initial_prompt, "json").await?;

        // --- Оптимистичный путь: если ответ уже чистый JSON, парсим его ---
        if let Ok(parsed) = serde_json::from_str::<T>(&raw_response) {
            return Ok(parsed);
        }

        // --- ШАГ 2: Если парсинг не удался, запускаем самоочистку ---
        println!("    -> LLM response was not clean JSON. Attempting self-extraction...");
        let extraction_prompt = format!(
            "Extract only the valid JSON object from the following text. Do not add any explanation, conversational text, or markdown. Only the JSON object itself.\n\nTEXT:\n---\n{}\n---",
            raw_response
        );
        
        let cleaner_response = self.make_ollama_request(&extraction_prompt, "json").await?;

        // --- Финальная попытка распарсить очищенный ответ ---
        serde_json::from_str::<T>(&cleaner_response).map_err(|e| {
            anyhow!(
                "Failed to parse JSON even after self-extraction.\nError: {}\nOriginal Response: '{}'\nCleaned Response: '{}'",
                e, raw_response, cleaner_response
            )
        })
    }
    
    // --- ПУБЛИЧНЫЕ МЕТОДЫ ТЕПЕРЬ ИСПОЛЬЗУЮТ НАДЕЖНЫЙ `send_request_and_extract_json` ---

    pub async fn analyze_error(&self, error_message: &str) -> Result<AnalysisPlan> {
        let prompt = format!(
            r#"You are a Rust compiler expert. Your task is to analyze a compiler error and create a plan for finding a solution.
**CRITICAL RULES:**
1.  Summarize the error in one simple sentence.
2.  Generate a JSON array of 3 distinct, natural-language search queries a human would type to solve this.
3.  If the error mentions an external crate, identify it. Otherwise, set "involved_crate" to null.
4.  Your output MUST be a valid JSON object.
**Compiler Error:**
"{}"
**Output Format:**
Return ONLY a valid JSON object with the keys "error_summary", "search_queries", and "involved_crate"."#,
            error_message
        );
        self.send_request_and_extract_json(&prompt).await
    }

    pub async fn generate_full_fix(
        &self,
        error_message: &str,
        full_code: &str,
        web_context: &str,
    ) -> Result<String> {
        let prompt = format!(
            r#"You are an expert Rust programmer. Your task is to fix a piece of Rust code.
**CRITICAL RULES:**
1.  You will be given the full source code of a file and a compiler error.
2.  Your task is to fix the code to resolve the error.
3.  **Your output MUST BE ONLY the complete, corrected, full source code for the file.** Do not add explanations or markdown.
--- COMPILER ERROR ---
{}
--- FULL SOURCE CODE ---
```rust
{}
```
--- CONTEXT FROM ONLINE SEARCH ---
{}
---
Your Corrected Full Source Code:"#,
            error_message, full_code, web_context
        );
        
        let raw_fix = self.make_ollama_request(&prompt, "").await?;
        
        // Очистка от markdown для обычного текста все еще нужна
        let cleaned_fix = raw_fix
            .trim()
            .trim_start_matches("```rust")
            .trim_start_matches("```")
            .trim_end_matches("```")
            .trim()
            .to_string();
            
        Ok(cleaned_fix)
    }
    
    pub async fn generate_cargo_fix(
        &self,
        error_message: &str,
    ) -> Result<OllamaCargoSuggestion> {
        let prompt = format!(
            r#"You are a Cargo.toml expert. A Rust project failed with an error message about a missing crate.
**TASK:** Identify the missing crate and suggest how to add it to Cargo.toml.
**CRITICAL RULES:**
1.  Your output MUST be a valid JSON object.
2.  The JSON must have two keys: `crate_name` (the name of the crate, e.g., "serde") and `dependency_line` (the full line to add to Cargo.toml, e.g., "serde = {{ version = \"1.0\", features = [\"derive\"] }}").
3.  Do NOT add any explanation. ONLY the JSON object.

--- COMPILER ERROR ---
{}

--- JSON OUTPUT EXAMPLE ---
{{
  "crate_name": "serde",
  "dependency_line": "serde = {{ version = \"1.0\", features = [\"derive\"] }}"
}}
"#,
            error_message
        );
        self.send_request_and_extract_json(&prompt).await
    }
}