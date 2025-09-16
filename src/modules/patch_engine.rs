use crate::{CargoMessage, CompilerMessage};
use super::llm_interface::LLMInterface;
use super::knowledge_cache::KnowledgeCache;
use anyhow::{Result, Context, bail};
use std::path::Path;
use tokio::fs;
use std::process::{Command, Stdio};
use std::io::{BufReader, BufRead};
use std::sync::{Arc, Mutex};
use std::thread;
use tempfile::TempDir;
use walkdir::WalkDir;

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

        // 1) Читаем исходник
        let original_code = fs::read_to_string(self.file_path).await
            .with_context(|| format!("Failed to read {}", self.file_path))?;

        // 2) Если есть валидный кэш — используем
        if !self.no_cache {
            if let Some(cached) = self.cache.lookup(&self.error_signature)? {
                if self.verify_in_temp(&cached, self.error_message).await? {
                    fs::write(self.file_path, cached).await?;
                    println!("    -> Applied solution from local knowledge cache.");
                    return Ok(());
                }
            }
        }

        // 3) Генерация фикса + до 2 самокоррекций
        let mut last_error = self.error_message.to_string();
        for attempt in 1..=MAX_ATTEMPTS {
            println!("    -> Fix attempt {} of {}", attempt, MAX_ATTEMPTS);
            let suggestion = self.generate_code_suggestion(&original_code, &last_error).await?;
            match self.verify_fix(&suggestion).await {
                Ok(VerificationResult::Success) => {
                    println!("    -> Verification successful!");
                    if !self.no_cache {
                        self.cache.store(&self.error_signature, &suggestion)?;
                    }
                    fs::write(self.file_path, suggestion).await?;
                    return Ok(());
                }
                Ok(VerificationResult::Failure(new_err)) => {
                    let head = new_err.lines().next().unwrap_or("Unknown verification error").to_string();
                    println!("    -> Verification failed: {}", head);
                    if attempt < MAX_ATTEMPTS {
                        println!("    -> Attempting self-correction...");
                        last_error = new_err;
                    } else {
                        bail!("Fix failed after {} attempts.", MAX_ATTEMPTS);
                    }
                }
                Err(e) => bail!("Verification errored: {e:#}"),
            }
        }
        Ok(())
    }

    async fn generate_code_suggestion(&self, original_code: &str, error: &str) -> Result<String> {
        self.llm.generate_full_fix(error, original_code, self.web_context).await
    }

    async fn verify_fix(&self, new_code: &str) -> Result<VerificationResult> {
        self.verify_in_temp(new_code, self.error_message).await
            .map(|ok| if ok { VerificationResult::Success } else { VerificationResult::Failure("Unknown".into()) })
    }

    async fn verify_in_temp(&self, new_code: &str, original_error_message: &str) -> Result<bool> {
        // Создаём временную копию репозитория и запускаем там проверки
        let temp = TempDir::new().context("Failed to create temp dir")?;
        copy_dir_all(".", temp.path()).await?;

        // Перезаписываем только целевой файл
        let dst_file = temp.path().join(self.file_path);
        if let Some(parent) = dst_file.parent() { fs::create_dir_all(parent).await.ok(); }
        fs::write(&dst_file, new_code).await?;

        let run_cargo = |what: &str| -> Result<(bool, Vec<CompilerMessage>)> {
            let mut child = Command::new("cargo")
                .current_dir(temp.path())
                .args([what, "--message-format=json"])
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .with_context(|| format!("spawn cargo {what}"))?;

            let messages: Arc<Mutex<Vec<CompilerMessage>>> = Arc::new(Mutex::new(Vec::new()));
            let messages_out = Arc::clone(&messages);
            let messages_err = Arc::clone(&messages);

            let mut threads = Vec::new();

            if let Some(stdout) = child.stdout.take() {
                threads.push(thread::spawn(move || {
                    let reader = BufReader::new(stdout);
                    for line in reader.lines().flatten() {
                        if let Ok(msg) = serde_json::from_str::<CargoMessage>(&line) {
                            if msg.reason == "compiler-message" {
                                if let Some(cm) = msg.message {
                                    messages_out.lock().unwrap().push(cm);
                                }
                            }
                        }
                    }
                }));
            }
            if let Some(stderr) = child.stderr.take() {
                threads.push(thread::spawn(move || {
                    let reader = BufReader::new(stderr);
                    for line in reader.lines().flatten() {
                        if let Ok(msg) = serde_json::from_str::<CargoMessage>(&line) {
                            if msg.reason == "compiler-message" {
                                if let Some(cm) = msg.message {
                                    messages_err.lock().unwrap().push(cm);
                                }
                            }
                        }
                    }
                }));
            }

            for t in threads { t.join().unwrap(); }
            let status = child.wait()?;
            let all = Arc::try_unwrap(messages).unwrap().into_inner().unwrap();
            Ok((status.success(), all))
        };

        // 1) cargo check
        let (check_ok, check_msgs) = run_cargo("check")?;
        if !check_ok {
            let first_err = check_msgs.iter()
                .find(|m| m.level == "error")
                .map(|m| m.message.clone())
                .unwrap_or_else(|| "Unknown check error".to_string());
            return Ok(!first_err.contains(original_error_message));
        }

        // 2) cargo test
        let (_test_ok, test_msgs) = run_cargo("test")?;
        let test_errors: Vec<_> = test_msgs.iter().filter(|m| m.level == "error").collect();
        if !test_errors.is_empty() {
            let first = test_errors.first().unwrap().message.clone();
            return Ok(!first.contains(original_error_message));
        }

        Ok(true)
    }
}

async fn copy_dir_all(src: impl AsRef<Path>, dst: impl AsRef<Path>) -> std::io::Result<()> {
    fs::create_dir_all(&dst).await?;
    for entry in WalkDir::new(src.as_ref())
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| !e.path().to_string_lossy().contains("target")
              && !e.path().to_string_lossy().contains(".git")
              && !e.path().to_string_lossy().contains(".rusty_fixer_cache.db"))
    {
        let relative = entry.path().strip_prefix(src.as_ref()).unwrap();
        let dst_path = dst.as_ref().join(relative);
        if entry.file_type().is_dir() {
            fs::create_dir_all(&dst_path).await?;
        } else {
            if let Some(parent) = dst_path.parent() { fs::create_dir_all(parent).await?; }
            fs::copy(entry.path(), &dst_path).await?;
        }
    }
    Ok(())
}
