// src/modules/cargo_expert.rs

use crate::{CargoMessage, CompilerMessage};
use super::llm_interface::LLMInterface;
use anyhow::{Context, Result};
use std::io::{BufReader, BufRead};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;
use tempfile;
use tokio::fs;
use walkdir::WalkDir;
use std::path::Path;

pub struct CargoExpert<'a> {
    llm: &'a LLMInterface,
}

impl<'a> CargoExpert<'a> {
    pub fn new(llm: &'a LLMInterface) -> Self {
        Self { llm }
    }

    pub async fn fix_manifest_issue(&self, issue: &CompilerMessage) -> Result<()> {
        println!("    -> Detected a potential Cargo.toml issue. Engaging Cargo Expert.");
        
        let cargo_toml_path = "Cargo.toml";
        let original_content = fs::read_to_string(cargo_toml_path)
            .await
            .context("Failed to read Cargo.toml")?;

        let suggested_content = self.llm.generate_cargo_fix(&issue.message, &original_content).await?;
        
        println!("    -> LLM suggested the following change for Cargo.toml:\n---\n{}\n---", suggested_content);

        if suggested_content.trim() == original_content.trim() {
            println!("    -> LLM suggested no changes to Cargo.toml. Assuming it's a code issue instead.");
            return Ok(());
        }

        println!("    -> Verifying the suggested Cargo.toml changes...");
        if self.verify_fix(&suggested_content, &issue.message).await? {
            println!("    -> Verification successful! Applying changes to Cargo.toml.");
            fs::write(cargo_toml_path, suggested_content).await?;
        } else {
            anyhow::bail!("Suggested Cargo.toml changes failed verification: the original issue was not resolved.");
        }

        Ok(())
    }

    async fn verify_fix(&self, new_content: &str, original_error_message: &str) -> Result<bool> {
        let temp_dir = tempfile::Builder::new().prefix("rusty-fixer-cargo-").tempdir()?;
        let project_root = std::env::current_dir()?;

        copy_dir_all(&project_root, temp_dir.path()).await?;
        
        let temp_cargo_path = temp_dir.path().join("Cargo.toml");
        fs::write(temp_cargo_path, new_content).await?;

        let temp_lock_path = temp_dir.path().join("Cargo.lock");
        if temp_lock_path.exists() {
            fs::remove_file(temp_lock_path).await?;
        }

        // --- НАДЕЖНЫЙ СБОР ОШИБОК ИЗ ОБОИХ ПОТОКОВ ---
        let mut child = Command::new("cargo")
            .arg("check")
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
        child.wait()?;

        let all_messages = Arc::try_unwrap(messages).unwrap().into_inner().unwrap();
        let new_errors: Vec<CompilerMessage> = all_messages.into_iter().filter(|m| m.level == "error").collect();
        
        // ----------------------------------------------------

        if new_errors.is_empty() {
            return Ok(true);
        }

        let original_error_still_exists = new_errors
            .iter()
            .any(|e| e.message.contains(original_error_message));

        Ok(!original_error_still_exists)
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