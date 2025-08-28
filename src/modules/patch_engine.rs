// src/modules/patch_engine.rs

use crate::{CargoMessage, CompilerMessage};
use super::llm_interface::LLMInterface;
use super::knowledge_cache::KnowledgeCache;
use std::path::Path;
use tokio::fs;
use std::process::{Command, Stdio};
use anyhow::{Result, Context, bail};
use walkdir::WalkDir;
use tempfile;
use std::io::{BufReader, BufRead};
use std::sync::{Arc, Mutex};
use std::thread;

pub enum VerificationResult {
    Success,
    Failure(String),
}

pub struct PatchEngine<'a> {
    llm: &'a LLMInterface,
    cache: &'a KnowledgeCache,
    error_signature: String,
    error_message: &'a str,
    file_path: &'a str,
    web_context: &'a str,
    no_cache: bool,
}

impl<'a> PatchEngine<'a> {
    pub fn new(
        llm: &'a LLMInterface,
        cache: &'a KnowledgeCache,
        error_signature: String,
        error_message: &'a str,
        file_path: &'a str,
        web_context: &'a str,
        no_cache: bool,
    ) -> Self {
        Self { llm, cache, error_signature, error_message, file_path, web_context, no_cache }
    }

    pub async fn run_and_self_correct(&self) -> Result<()> {
        const MAX_ATTEMPTS: u32 = 2;
        let original_code = fs::read_to_string(self.file_path).await?;
        let mut last_error = self.error_message.to_string();

        for attempt in 1..=MAX_ATTEMPTS {
            println!("    -> Fix attempt {} of {}", attempt, MAX_ATTEMPTS);
            let suggested_full_code = self.generate_code_suggestion(&original_code, &last_error).await?;

            match self.verify_fix(&suggested_full_code).await {
                Ok(VerificationResult::Success) => {
                    println!("    -> Verification successful!");
                    if !self.no_cache && self.cache.lookup(&self.error_signature)?.is_none() {
                        self.cache.store(&self.error_signature, &suggested_full_code)?;
                        println!("    -> Stored new successful solution in the knowledge cache.");
                    }
                    fs::write(self.file_path, suggested_full_code).await?;
                    return Ok(());
                }
                Ok(VerificationResult::Failure(new_error)) => {
                    let first_line = new_error.lines().next().unwrap_or("Unknown verification error").to_string();
                    println!("    -> Verification failed. New error: {}", first_line);
                    if attempt < MAX_ATTEMPTS {
                        println!("    -> Attempting self-correction...");
                        last_error = new_error;
                    } else {
                        bail!("Fix failed after {} attempts. Last error: {}", MAX_ATTEMPTS, first_line);
                    }
                }
                Err(e) => return Err(e),
            }
        }
        bail!("Could not find a working solution.")
    }

    async fn generate_code_suggestion(&self, original_code: &str, error_message: &str) -> Result<String> {
        if !self.no_cache {
            if let Some(cached_solution) = self.cache.lookup(&self.error_signature)? {
                println!("    -> Found a potential solution in the knowledge cache.");
                return Ok(cached_solution);
            }
        }
        println!("    -> No solution in cache. Asking LLM to generate a new version.");
        self.llm.generate_full_fix(error_message, original_code, self.web_context).await
    }
    
    async fn verify_fix(&self, full_code: &str) -> Result<VerificationResult> {
        let temp_dir = tempfile::Builder::new().prefix("rusty-fixer-code-").tempdir()?;
        copy_dir_all(".", temp_dir.path()).await.context("Failed to copy project to temp dir")?;
        
        let target_path_in_temp = temp_dir.path().join(self.file_path);
        fs::write(&target_path_in_temp, full_code).await?;

        let run_cargo = |cmd: &str| -> Result<(bool, Vec<CompilerMessage>)> {
            let mut child = Command::new("cargo")
                .arg(cmd)
                .arg("--message-format=json")
                .current_dir(temp_dir.path())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()?;
            
            let messages = Arc::new(Mutex::new(Vec::new()));
            let mut threads = Vec::new();
            
            let messages_clone_out = Arc::clone(&messages);
            if let Some(stdout) = child.stdout.take() {
                threads.push(thread::spawn(move || {
                    let reader = BufReader::new(stdout);
                    for line in reader.lines().flatten() {
                        if let Ok(msg) = serde_json::from_str::<CargoMessage>(&line) {
                        if msg.reason == "compiler-message" {
                            if let Some(compiler_msg) = msg.message {
                                messages_clone_out.lock().unwrap().push(compiler_msg);
                            }
                        }
                        }
                    }
                }));
            }

            let messages_clone_err = Arc::clone(&messages);
            if let Some(stderr) = child.stderr.take() {
                threads.push(thread::spawn(move || {
                    let reader = BufReader::new(stderr);
                    for line in reader.lines().flatten() {
                        if let Ok(msg) = serde_json::from_str::<CargoMessage>(&line) {
                        if msg.reason == "compiler-message" {
                            if let Some(compiler_msg) = msg.message {
                                messages_clone_err.lock().unwrap().push(compiler_msg);
                            }
                        }
                        }
                    }
                }));
            }

            for t in threads {
                t.join().unwrap();
            }
            let status = child.wait()?;
            let all_messages = Arc::try_unwrap(messages).unwrap().into_inner().unwrap();
            Ok((status.success(), all_messages))
        };

        // 1. Запускаем `cargo check`
        let (check_success, check_messages) = run_cargo("check")?;
        if !check_success {
            let first_error = check_messages
                .iter()
                .find(|m| m.level == "error")
                .map(|m| m.message.clone())
                .unwrap_or_else(|| "Unknown check error".to_string());
            return Ok(VerificationResult::Failure(first_error));
        }

        // 2. Если check прошел, запускаем `cargo test`
        println!("    -> `cargo check` passed. Now running tests...");
        let (_, test_messages) = run_cargo("test")?;
        let test_errors: Vec<_> = test_messages.iter().filter(|m| m.level == "error").collect();
            
        if !test_errors.is_empty() {
             let first_error = test_errors.first().unwrap().message.clone();
             return Ok(VerificationResult::Failure(first_error));
        }

        Ok(VerificationResult::Success)
    }
}

async fn copy_dir_all(src: impl AsRef<Path>, dst: impl AsRef<Path>) -> std::io::Result<()> {
    fs::create_dir_all(&dst).await?;
    for entry in WalkDir::new(src.as_ref())
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| !e.path().to_string_lossy().contains("target") && !e.path().to_string_lossy().contains(".git"))
    {
        let relative_path = entry.path().strip_prefix(src.as_ref()).unwrap();
        let dst_file_path = dst.as_ref().join(relative_path);

        if entry.file_type().is_dir() {
            fs::create_dir_all(&dst_file_path).await?;
        } else {
            if let Some(parent) = dst_file_path.parent() {
                fs::create_dir_all(parent).await?;
            }
            fs::copy(entry.path(), &dst_file_path).await?;
        }
    }
    Ok(())
}