use crate::{CargoMessage, CompilerMessage};
use super::llm_interface::{LLMInterface, CargoSuggestionDetails};
use anyhow::{Context, Result};
use std::io::{BufReader, BufRead};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;
use tokio::fs;
use toml_edit::{DocumentMut, Item, Value, InlineTable, Array};

pub struct CargoExpert <'a> {
    llm: &'a LLMInterface,
}

impl<'a> CargoExpert<'a> {
    pub fn new(llm: &'a LLMInterface) -> Self { Self { llm } }

    /// Правит конкретный Cargo.toml по относительному пути `manifest_rel_path`
    /// Возвращает Ok(true), если изменения применены (и проверка прошла).
    pub async fn fix_manifest_issue_at(&self, issue: &CompilerMessage, manifest_rel_path: &str) -> Result<bool> {
        println!("    -> Detected a potential Cargo.toml issue. Engaging Cargo Expert.");

        // 1) Пытаемся спросить LLM
        let suggestion = match self.llm.generate_cargo_fix(&issue.message).await {
            Ok(s) => s,
            Err(e) => {
                eprintln!("    -> LLM cargo suggestion failed: {e}. Using heuristic fallback.");
                self.heuristic_for_common_errors(&issue.message)
            }
        };

        println!(
            "    -> Suggested adding crate `{}`, version `{}`, features `{:?}`",
            suggestion.crate_name, suggestion.version, suggestion.features
        );

        // 2) Читаем нужный Cargo.toml и вносим изменения
        let original_content = fs::read_to_string(manifest_rel_path)
            .await
            .with_context(|| format!("Failed to read {}", manifest_rel_path))?;
        let mut doc = original_content.parse::<DocumentMut>()
            .context("Failed to parse Cargo.toml")?;

        // гарантируем наличие [dependencies]
        if doc.get("dependencies").is_none() {
            doc["dependencies"] = toml_edit::table();
        }

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
            anyhow::bail!("Could not find or create [dependencies] table");
        }

        let new_content = doc.to_string();
        if new_content.trim() == original_content.trim() {
            println!("    -> No effective changes were made to Cargo.toml. Skipping.");
            return Ok(false);
        }

        println!("    -> Verifying the suggested Cargo.toml changes...");
        if self.verify_fix(manifest_rel_path, &new_content, &issue.message).await? {
            println!("    -> Verification successful! Applying changes to {}.", manifest_rel_path);
            fs::write(manifest_rel_path, new_content).await?;
            Ok(true)
        } else {
            println!("    -> Verification failed. Skipping manifest change.");
            Ok(false)
        }
    }

    /// Простая эвристика на популярные ошибки манифеста.
    fn heuristic_for_common_errors(&self, error_msg: &str) -> CargoSuggestionDetails {
        let msg_l = error_msg.to_lowercase();
        // cannot find derive macro `Serialize` / `Deserialize`
        if msg_l.contains("derive macro `serialize`") || msg_l.contains("derive macro `deserialize`")
        || msg_l.contains("cannot find derive macro `serialize`") || msg_l.contains("cannot find derive macro `deserialize`") {
            return CargoSuggestionDetails {
                crate_name: "serde".to_string(),
                version: "1".to_string(),
                features: vec!["derive".to_string()],
            };
        }
        // unresolved import serde_json
        if msg_l.contains("use of undeclared crate or module `serde_json`")
            || msg_l.contains("cannot find crate `serde_json`")
            || msg_l.contains("unresolved import `serde_json`") {
            return CargoSuggestionDetails {
                crate_name: "serde_json".to_string(),
                version: "1".to_string(),
                features: vec![],
            };
        }
        // По умолчанию — предлагаем serde с derive
        CargoSuggestionDetails {
            crate_name: "serde".to_string(),
            version: "1".to_string(),
            features: vec!["derive".to_string()],
        }
    }

    async fn verify_fix(&self, manifest_rel_path: &str, new_cargo_toml: &str, original_error_message: &str) -> Result<bool> {
        let tmp = tempfile::tempdir()?;
        // копируем весь репо
        copy_sources(".", tmp.path()).await?;

        // перезаписываем КОНКРЕТНЫЙ манифест в копии
        let manifest_dest = tmp.path().join(manifest_rel_path);
        if let Some(parent) = manifest_dest.parent() {
            tokio::fs::create_dir_all(parent).await.ok();
        }
        tokio::fs::write(&manifest_dest, new_cargo_toml).await?;

        // собираем весь воркспейс/крейта из корня
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

            let all = Arc::try_unwrap(msgs).unwrap().into_inner().unwrap();
            let errors: Vec<&CompilerMessage> = all.iter().filter(|m| m.level == "error").collect();
            let ok = if errors.is_empty() {
                true
            } else {
                // фикс успешен, если исходная ошибка исчезла
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
