// src/modules/llm_interface.rs

use serde::{Deserialize, Serialize};

// --- КОНФИГУРАЦИЯ ---
const OLLAMA_API_URL: &str = "http://127.0.0.1:11434/api/chat";
const LLM_MODEL_NAME: &str = "llama3:8b";

// --- СТРУКТУРЫ ДЛЯ OLLAMA API ---

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

// --- СТРУКТУРА ДЛЯ НАШЕГО ПЛАНА ---

#[derive(Debug, Deserialize)]
pub struct AnalysisPlan {
    pub error_summary: String,
    pub search_keywords: String,
}

// --- ПУБЛИЧНАЯ СТРУКТУРА ИНТЕРФЕЙСА ---

pub struct LLMInterface {
    client: reqwest::Client,
}

impl LLMInterface {
    pub fn new() -> Self {
        Self {
            client: reqwest::Client::new(),
        }
    }

    /// Анализирует ошибку компилятора и возвращает план действий.
    pub async fn analyze_error(&self, error_message: &str) -> Result<AnalysisPlan, Box<dyn std::error::Error>> {
        let prompt = format!(
            r#"You are a Rust compiler expert. Your task is to analyze a compiler error message and create a concise plan for finding a solution.

**CRITICAL RULES:**
1.  Summarize the error in one simple sentence.
2.  Generate a set of 3-4 natural language search keywords a human would use to find the solution online.
3.  Your output MUST be a valid JSON object, and nothing else.

**Compiler Error:**
"{}"

**Output Format:**
Return ONLY a valid JSON object with the keys "error_summary" and "search_keywords".

**Example:**
Compiler Error: "cannot assign twice to immutable variable `x`"
Your Output:
{{
  "error_summary": "A variable was reassigned without being declared as mutable.",
  "search_keywords": "rust cannot assign twice to immutable variable fix"
}}
"#,
            error_message
        );

        let request_payload = OllamaRequest {
            model: LLM_MODEL_NAME,
            messages: vec![Message { role: "user", content: &prompt }],
            stream: false,
            format: "json",
        };

        let res = self.client.post(OLLAMA_API_URL)
            .json(&request_payload)
            .send()
            .await?;

        if !res.status().is_success() {
            return Err(format!("Ollama API request failed with status {}", res.status()).into());
        }

        let ollama_response = res.json::<OllamaResponse>().await?;
        let plan: AnalysisPlan = serde_json::from_str(&ollama_response.message.content)?;

        Ok(plan)
    }
}