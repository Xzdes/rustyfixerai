// src/modules/llm_interface.rs

use serde::{Deserialize, Serialize};

// --- КОНФИГУРАЦІЯ ---
const OLLAMA_API_URL: &str = "http://127.0.0.1:11434/api/chat";
const LLM_MODEL_NAME: &str = "llama3:8b";

// --- СТРУКТУРИ ДЛЯ OLLAMA API ---

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

// --- УЛУЧШЕННАЯ СТРУКТУРА ПЛАНА ---

#[derive(Debug, Deserialize)]
pub struct AnalysisPlan {
    pub error_summary: String,
    pub search_keywords: String,
    pub involved_crate: Option<String>,
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

    /// Анализирует ошибку компилятора и возвращает УЛУЧШЕННЫЙ план действий.
    pub async fn analyze_error(&self, error_message: &str) -> Result<AnalysisPlan, Box<dyn std::error::Error>> {
        let prompt = format!(
            r#"You are a Rust compiler expert. Your task is to analyze a compiler error and create a plan for finding a solution.

**CRITICAL RULES:**
1.  Summarize the error in one simple sentence.
2.  Generate a set of 3-4 natural language search keywords. The "search_keywords" value MUST be a single string.
3.  **If the error message mentions a type or trait from an external crate (e.g., "the trait `serde::Deserialize` is not implemented for `MyStruct`"), identify the crate's name (e.g., "serde"). If not, set "involved_crate" to null.**
4.  Your output MUST be a valid JSON object.

**Compiler Error:**
"{}"

**Output Format:**
Return ONLY a valid JSON object with the keys "error_summary", "search_keywords", and "involved_crate".

**Good Example 1 (Crate involved):**
Compiler Error: "the trait bound `MyStruct: Serialize` is not satisfied. the trait `Serialize` is not implemented for `MyStruct`"
Your Output:
{{
  "error_summary": "A struct is missing the required 'Serialize' trait implementation.",
  "search_keywords": "rust trait Serialize is not implemented for struct",
  "involved_crate": "serde"
}}

**Good Example 2 (No crate involved):**
Compiler Error: "cannot assign twice to immutable variable `x`"
Your Output:
{{
  "error_summary": "A variable was reassigned without being declared as mutable.",
  "search_keywords": "rust cannot assign twice to immutable variable fix",
  "involved_crate": null
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

    // --- КАРДИНАЛЬНО НОВЫЙ МЕТОД ГЕНЕРАЦИИ ИСПРАВЛЕНИЯ ---
    pub async fn generate_full_fix(
        &self,
        error_message: &str,
        full_code: &str, // <-- Принимаем ВЕСЬ КОД
        web_context: &str,
    ) -> Result<String, Box<dyn std::error::Error>> {
        let prompt = format!(
            r#"You are an expert Rust programmer. Your task is to fix a piece of Rust code.

**CRITICAL RULES:**
1.  You will be given the full source code of a file and a compiler error.
2.  Your task is to fix the code to resolve the error. You might need to change a different line than the one reported in the error, or add/remove lines.
3.  **Your output MUST BE ONLY the complete, corrected, full source code for the file.** Do not add explanations, markdown code blocks, or any other text. Just the raw, fixed code.

--- COMPILER ERROR ---
{}

--- FULL SOURCE CODE ---
```rust
{}
```

--- CONTEXT FROM ONLINE SEARCH (for your reference) ---
{}

---
Your Corrected Full Source Code:
"#,
            error_message, full_code, web_context
        );
        
        let request_payload = OllamaRequest {
            model: LLM_MODEL_NAME,
            messages: vec![Message { role: "user", content: &prompt }],
            stream: false,
            format: "", 
        };

        let res = self.client.post(OLLAMA_API_URL)
            .json(&request_payload)
            .send()
            .await?;

        if !res.status().is_success() {
            return Err(format!("Ollama API request failed with status {}", res.status()).into());
        }

        let ollama_response = res.json::<OllamaResponse>().await?;
        // Очищаем потенциальные "обертки" от LLM
        let cleaned_fix = ollama_response.message.content
            .trim()
            .trim_start_matches("```rust")
            .trim_start_matches("```")
            .trim_end_matches("```")
            .trim()
            .to_string();
            
        Ok(cleaned_fix)
    }
}
