use crate::{CargoMessage, CompilerMessage};
use super::llm_interface::LLMInterface;
use anyhow::{Context, Result};
use std::io::{BufReader, BufRead};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;
use tokio::fs;
use toml_edit::{DocumentMut, Item, Value, InlineTable, Array};

pub struct CargoExpert<'a> {
    llm: &'a LLMInterface,
}

impl<'a> CargoExpert<'a> {
    pub fn new(llm: &'a LLMInterface) -> Self { Self { llm } }

    pub async fn fix_manifest_issue(&self, issue: &CompilerMessage) -> Result<()> {
        println!("    -> Detected a potential Cargo.toml issue. Engaging Cargo Expert.");

        let suggestion = self.llm.generate_cargo_fix(&issue.message).await?;

        println!(
            "    -> LLM suggested adding crate `{}`, version `{}`, features `{:?}`",
            suggestion.crate_name, suggestion.version, suggestion.features
        );

        let cargo_toml_path = "Cargo.toml";
        let original_content = fs::read_to_string(cargo_toml_path)
            .await
            .context("Failed to read Cargo.toml")?;
        let mut doc = original_content.parse::<DocumentMut>()
            .context("Failed to parse Cargo.toml")?;

        if let Some(deps) = doc["dependencies"].as_table_mut() {
            let dep_item = if suggestion.features.is_empty() {
                Item::Value(Value::from(suggestion.version))
            } else {
                let mut table = InlineTable::new();
                table.insert("version", Value::from(suggestion.version));
                let mut features = Array::new();
                for f in suggestion.features {
                    features.push(f);
                }
                table.insert("features", Value::from(features));
                Item::Value(table.into())
            };
            deps.insert(&suggestion.crate_name, dep_item);
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
            println!("    -> Verification failed. Skipping manifest change.");
        }

        Ok(())
    }

    async fn verify_fix(&self, new_cargo_toml: &str, original_error_message: &str) -> Result<bool> {
        let tmp = tempfile::tempdir()?;
        tokio::fs::write(tmp.path().join("Cargo.toml"), new_cargo_toml).await?;
        // Копируем исходники (простая версия: текущая директория = корень)
        copy_sources(".", tmp.path()).await?;

        let run_cargo = |what: &str| -> Result<(bool, Vec<CompilerMessage>)> {
            let mut child = Command::new("cargo")
                .current_dir(tmp.path())
                .args([what, "--message-format=json"])
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()?;

            let msgs: Arc<Mutex<Vec<CompilerMessage>>> = Arc::new(Mutex::new(Vec::new()));
            let msgs_out = Arc::clone(&msgs);
            let msgs_err = Arc::clone(&msgs);

            let mut ths = Vec::new();
            if let Some(stdout) = child.stdout.take() {
                ths.push(thread::spawn(move || {
                    let r = BufReader::new(stdout);
                    for line in r.lines().flatten() {
                        if let Ok(m) = serde_json::from_str::<CargoMessage>(&line) {
                            if m.reason == "compiler-message" {
                                if let Some(cm) = m.message { msgs_out.lock().unwrap().push(cm); }
                            }
                        }
                    }
                }));
            }
            if let Some(stderr) = child.stderr.take() {
                ths.push(thread::spawn(move || {
                    let r = BufReader::new(stderr);
                    for line in r.lines().flatten() {
                        if let Ok(m) = serde_json::from_str::<CargoMessage>(&line) {
                            if m.reason == "compiler-message" {
                                if let Some(cm) = m.message { msgs_err.lock().unwrap().push(cm); }
                            }
                        }
                    }
                }));
            }
            for t in ths { t.join().unwrap(); }
            let _status = child.wait()?;

            // анализируем сообщения
            let all = Arc::try_unwrap(msgs).unwrap().into_inner().unwrap();
            let errors: Vec<&CompilerMessage> = all.iter().filter(|m| m.level == "error").collect();
            let ok = if errors.is_empty() {
                true
            } else {
                // фикс считаем успешным, если исходная ошибка исчезла
                !errors.iter().any(|e| e.message.contains(original_error_message))
            };

            Ok((ok, all))
        };

        let (ok, _msgs) = run_cargo("check")?;
        Ok(ok)
    }
}

async fn copy_sources(src: &str, dst: &std::path::Path) -> Result<()> {
    let src_path = std::path::Path::new(src);
    let mut stack = vec![src_path.to_path_buf()];
    while let Some(path) = stack.pop() {
        let meta = tokio::fs::metadata(&path).await?;
        if meta.is_dir() {
            let mut rd = tokio::fs::read_dir(&path).await?;
            while let Some(ent) = rd.next_entry().await? {
                let p = ent.path();
                // пропускаем target/.git/локальную БД кэша
                let s = p.to_string_lossy();
                if s.contains("target") || s.contains(".git") || s.contains(".rusty_fixer_cache.db") { continue; }
                stack.push(p);
            }
        } else {
            let rel = path.strip_prefix(src_path).unwrap();
            let dst_path = dst.join(rel);
            if let Some(parent) = dst_path.parent() { tokio::fs::create_dir_all(parent).await?; }
            tokio::fs::copy(&path, &dst_path).await?;
        }
    }
    Ok(())
}
