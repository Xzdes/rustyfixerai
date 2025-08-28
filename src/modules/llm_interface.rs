// src/modules/llm_interface.rs

use serde::{Deserialize, Serialize};
use anyhow::{Result, anyhow};

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

#[derive(Deserialize)]
struct OllamaTomlFixResponse {
    cargo_toml: String,
}


// --- ANALYSIS PLAN STRUCTURE ---

#[derive(Debug, Deserialize)]
pub struct AnalysisPlan {
    pub error_summary: String,
    pub search_queries: Vec<String>,
    pub involved_crate: Option<String>,
}

// --- PUBLIC INTERFACE STRUCTURE ---

pub struct LLMInterface {
    client: reqwest::Client,
}

impl LLMInterface {
    pub fn new() -> Self {
        Self {
            client: reqwest::Client::new(),
        }
    }

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
            return Err(anyhow!("Ollama API request failed with status {}", res.status()));
        }

        let ollama_response = res.json::<OllamaResponse>().await?;
        let analysis_plan: AnalysisPlan = serde_json::from_str(&ollama_response.message.content)?;
        Ok(analysis_plan)
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
            return Err(anyhow!("Ollama API request failed with status {}", res.status()));
        }

        let ollama_response = res.json::<OllamaResponse>().await?;
        let cleaned_fix = ollama_response.message.content
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
        cargo_toml_content: &str,
    ) -> Result<String> {
        let prompt = format!(
            r#"You are a Cargo.toml expert. A Rust project failed with an error.
**TASK:** Analyze the error and the `Cargo.toml`. Add the missing dependency required to fix the error.
**CRITICAL RULES:**
1.  Identify the missing crate from the error message.
2.  Add the dependency to the `[dependencies]` section. Use a common version like "1.0" or "0.12". If the error mentions a feature (like "derive" for serde), add that feature.
3.  Your output MUST be a valid JSON object with a single key "cargo_toml" containing the complete, corrected TOML file as a string.

--- COMPILER ERROR ---
{}

--- CURRENT Cargo.toml CONTENT ---
```toml
{}
```

--- JSON OUTPUT EXAMPLE ---
{{
  "cargo_toml": "[package]\nname = \"...\"\n...\n[dependencies]\nserde = {{ version = \"1.0\", features = [\"derive\"] }}\n..."
}}
"#,
            error_message, cargo_toml_content
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
            return Err(anyhow!("Ollama API request failed with status {}", res.status()));
        }

        let ollama_response = res.json::<OllamaResponse>().await?;
        let fix_data: OllamaTomlFixResponse = serde_json::from_str(&ollama_response.message.content)
            .map_err(|e| anyhow!("Failed to parse LLM JSON response for Cargo.toml fix: {}", e))?;
            
        Ok(fix_data.cargo_toml)
    }
}