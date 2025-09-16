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
    Failure(String), // подробное сообщение об ошибке верификации
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
        const MAX_ATTEMPTS: u32 = 3;

        // 1) Читаем исходник
        let original_code = fs::read_to_string(self.file_path).await
            .with_context(|| format!("Failed to read {}", self.file_path))?;

        // 2) Если есть валидный кэш — используем
        if !self.no_cache {
            if let Some(cached) = self.cache.lookup(&self.error_signature)? {
                match self.verify_fix(&cached).await? {
                    VerificationResult::Success => {
                        fs::write(self.file_path, cached).await?;
                        println!("    -> Applied solution from local knowledge cache.");
                        return Ok(());
                    }
                    VerificationResult::Failure(msg) => {
                        println!("    -> Cached solution failed verification: {}", first_line(&msg));
                    }
                }
            }
        }

        // 3) Генерация фикса + самокоррекции на основе подробной ошибки
        let mut last_error_context = self.error_message.to_string();
        for attempt in 1..=MAX_ATTEMPTS {
            println!("    -> Fix attempt {} of {}", attempt, MAX_ATTEMPTS);
            let suggestion = self.generate_code_suggestion(&original_code, &last_error_context).await?;
            match self.verify_fix(&suggestion).await? {
                VerificationResult::Success => {
                    println!("    -> Verification successful!");
                    if !self.no_cache {
                        self.cache.store(&self.error_signature, &suggestion)?;
                    }
                    fs::write(self.file_path, suggestion).await?;
                    return Ok(());
                }
                VerificationResult::Failure(new_err) => {
                    println!("    -> Verification failed: {}", first_line(&new_err));
                    if attempt < MAX_ATTEMPTS {
                        println!("    -> Attempting self-correction with fresh error context...");
                        last_error_context = new_err;
                    } else {
                        bail!("Fix failed after {} attempts.", MAX_ATTEMPTS);
                    }
                }
            }
        }
        Ok(())
    }

    async fn generate_code_suggestion(&self, original_code: &str, error_context: &str) -> Result<String> {
        // Передаем ВЕСЬ контекст ошибки (последний провал проверки), чтобы LLM чётко понимал расхождение типов и место
        self.llm.generate_full_fix(error_context, original_code, self.web_context).await
    }

    async fn verify_fix(&self, new_code: &str) -> Result<VerificationResult> {
        match self.verify_in_temp(new_code).await? {
            None => Ok(VerificationResult::Success),
            Some(err) => Ok(VerificationResult::Failure(err)),
        }
    }

    /// Возвращает None, если всё ок; иначе Some(подробное сообщение об ошибке)
    async fn verify_in_temp(&self, new_code: &str) -> Result<Option<String>> {
        // Создаём временную копию репозитория и запускаем там проверки
        let temp = TempDir::new().context("Failed to create temp dir")?;
        copy_dir_all(".", temp.path()).await?;

        // Перезаписываем только целевой файл
        let dst_file = temp.path().join(self.file_path);
        if let Some(parent) = dst_file.parent() { fs::create_dir_all(parent).await.ok(); }
        fs::write(&dst_file, new_code).await?;

        // Общий раннер cargo c парсингом ошибок
        let mut collect_first_error = |what: &str| -> Result<Option<String>> {
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
            let _status = child.wait()?;
            let all = Arc::try_unwrap(messages).unwrap().into_inner().unwrap();

            // выбираем ПЕРВУЮ ошибку и формируем понятный текст
            let mut errors: Vec<CompilerMessage> = all.into_iter().filter(|m| m.level == "error").collect();
            if errors.is_empty() {
                return Ok(None);
            }
            // отсортируем по строке первого спана
            errors.sort_by_key(|m| m.spans.first().map_or(usize::MAX, |s| s.line_start));
            let e = &errors[0];
            let loc = e.spans.first().map(|s| format!("{}:{}",
                s.file_name.replace('/', std::path::MAIN_SEPARATOR_STR),
                s.line_start
            )).unwrap_or_else(|| "<unknown>".into());
            let code = e.code.as_ref().map(|c| format!(" [{}]", c.code)).unwrap_or_default();
            let msg = format!("{}{} at {}\n{}", e.message, code, loc, stringify_spans(e));
            Ok(Some(msg))
        };

        // 1) cargo check
        if let Some(err) = collect_first_error("check")? {
            return Ok(Some(err));
        }

        // 2) cargo test (если тесты падают — это тоже контекст для LLM)
        if let Some(err) = collect_first_error("test")? {
            return Ok(Some(err));
        }

        Ok(None)
    }
}

fn stringify_spans(e: &CompilerMessage) -> String {
    let mut out = String::new();
    for s in &e.spans {
        out.push_str(&format!(
            "- at {}:{}{}\n",
            s.file_name.replace('/', std::path::MAIN_SEPARATOR_STR),
            s.line_start,
            s.suggested_replacement.as_ref().map(|r| format!(" (suggested: {})", r)).unwrap_or_default()
        ));
    }
    out
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
            tokio::fs::copy(entry.path(), &dst_path).await?;
        }
    }
    Ok(())
}

fn first_line(s: &str) -> String {
    s.lines().next().unwrap_or(s).to_string()
}
