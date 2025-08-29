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
// --- ПРАВИЛЬНЫЕ ИМПОРТЫ ---
use toml_edit::{DocumentMut, Item, Value, InlineTable, Array};

pub struct CargoExpert<'a> {
    llm: &'a LLMInterface,
}

impl<'a> CargoExpert<'a> {
    pub fn new(llm: &'a LLMInterface) -> Self {
        Self { llm }
    }

    pub async fn fix_manifest_issue(&self, issue: &CompilerMessage) -> Result<()> {
        println!("    -> Detected a potential Cargo.toml issue. Engaging Cargo Expert.");
        
        let suggestion = self.llm.generate_cargo_fix(&issue.message).await?;
        
        println!(
            "    -> LLM suggested adding crate `{}`, version `{}`, features `{:?}`", 
            suggestion.crate_name, suggestion.version, suggestion.features
        );

        let cargo_toml_path = "Cargo.toml";
        let original_content = fs::read_to_string(cargo_toml_path).await.context("Failed to read Cargo.toml")?;
        let mut doc = original_content.parse::<DocumentMut>().context("Failed to parse Cargo.toml")?;

        if let Some(deps) = doc["dependencies"].as_table_mut() {
            // --- ФИНАЛЬНОЕ ИСПРАВЛЕНИЕ: ПРАВИЛЬНОЕ СОЗДАНИЕ `Value` ---
            let dep_item = if suggestion.features.is_empty() {
                // `Item::Value` принимает `Value`, а `Value::from` создает его из строки
                Item::Value(Value::from(suggestion.version))
            } else {
                // Создаем inline-таблицу для `version` и `features`
                let mut details_table = InlineTable::new();

                // `Value::from` создает `Value` из строки
                details_table.insert("version", Value::from(suggestion.version));
                
                let mut features_array = Array::new();
                for feature in suggestion.features {
                    features_array.push(feature);
                }
                // `Value::from` также может создать `Value` из `Array`
                details_table.insert("features", Value::from(features_array));
                
                Item::Value(details_table.into())
            };
            
            deps.insert(&suggestion.crate_name, dep_item);
            // --- КОНЕЦ ИСПРАВЛЕНИЯ ---
        } else {
            anyhow::bail!("Could not find [dependencies] table in Cargo.toml");
        }
        
        let new_content = doc.to_string();

        if new_content.trim() == original_content.trim() {
            println!("    -> No effective changes were made to Cargo.toml. Skipping.");
            return Ok(());
        }
        
        println!("    -> Verifying the suggested Cargo.toml changes...");
        if self.verify_fix(&new_content, &issue.message).await? {
            println!("    -> Verification successful! Applying changes to Cargo.toml.");
            fs::write(cargo_toml_path, new_content).await?;
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

        if let Ok(path) = fs::try_exists(temp_dir.path().join("Cargo.lock")).await {
            if path {
                fs::remove_file(temp_dir.path().join("Cargo.lock")).await?;
            }
        }

        let mut child = Command::new("cargo").arg("check").arg("--message-format=json").current_dir(temp_dir.path()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn()?;
        let messages = Arc::new(Mutex::new(Vec::new()));
        let mut threads = Vec::new();
        
        if let Some(stdout) = child.stdout.take() {
            let messages_clone = Arc::clone(&messages);
            threads.push(thread::spawn(move || {
                let reader = BufReader::new(stdout);
                for line in reader.lines().flatten() {
                    if let Ok(msg) = serde_json::from_str::<CargoMessage>(&line) {
                       if msg.reason == "compiler-message" {
                           if let Some(compiler_msg) = msg.message {
                               messages_clone.lock().unwrap().push(compiler_msg);
                           }
                       }
                    }
                }
            }));
        }
        if let Some(stderr) = child.stderr.take() {
            let messages_clone = Arc::clone(&messages);
            threads.push(thread::spawn(move || {
                let reader = BufReader::new(stderr);
                 for line in reader.lines().flatten() {
                    if let Ok(msg) = serde_json::from_str::<CargoMessage>(&line) {
                       if msg.reason == "compiler-message" {
                           if let Some(compiler_msg) = msg.message {
                               messages_clone.lock().unwrap().push(compiler_msg);
                           }
                       }
                    }
                }
            }));
        }

        for t in threads { t.join().unwrap(); }
        child.wait()?;

        let all_messages = Arc::try_unwrap(messages).unwrap().into_inner().unwrap();
        let new_errors: Vec<CompilerMessage> = all_messages.into_iter().filter(|m| m.level == "error").collect();
        
        if new_errors.is_empty() { return Ok(true); }
        let original_error_still_exists = new_errors.iter().any(|e| e.message.contains(original_error_message));
        Ok(!original_error_still_exists)
    }
}

async fn copy_dir_all(src: impl AsRef<Path>, dst: impl AsRef<Path>) -> std::io::Result<()> {
    fs::create_dir_all(&dst).await?;
    for entry in WalkDir::new(src.as_ref()).into_iter().filter_map(|e| e.ok()).filter(|e| !e.path().to_string_lossy().contains("target") && !e.path().to_string_lossy().contains(".git")) {
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