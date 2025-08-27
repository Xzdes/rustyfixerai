// src/modules/patch_engine.rs

use super::llm_interface::LLMInterface;
use super::knowledge_cache::KnowledgeCache;
use std::path::Path;
use tokio::fs;
use std::process::Command;

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

    /// Главный метод: ищет в кэше, генерирует, верифицирует и применяет исправление.
    pub async fn run(&self) -> Result<(), Box<dyn std::error::Error>> {
        let original_code = fs::read_to_string(self.file_path).await?;
        let mut suggested_full_code = String::new();

        // 1. Сначала ищем решение в локальном кэше
        if !self.no_cache {
            if let Some(cached_solution) = self.cache.lookup(&self.error_signature)? {
                println!("    -> Found a potential solution in the knowledge cache.");
                suggested_full_code = self.apply_patch(&original_code, &cached_solution);
            }
        }

        // 2. Если в кэше нет, генерируем новое решение с помощью LLM
        if suggested_full_code.is_empty() {
            println!("    -> No solution in cache. Asking LLM to generate a new version of the file.");
            suggested_full_code = self.llm.generate_full_fix(
                self.error_message,
                &original_code,
                self.web_context,
            ).await?;
        }

        // 3. Верифицируем предложенный код
        if self.verify_fix(&suggested_full_code).await? {
            // 4. Сохраняем успешное решение в кэш (если оно было сгенерировано, а не из кэша)
            if !self.no_cache && self.cache.lookup(&self.error_signature)?.is_none() {
                let patch = self.create_patch(&original_code, &suggested_full_code);
                self.cache.store(&self.error_signature, &patch)?;
                println!("    -> Stored new successful solution in the knowledge cache.");
            }
            
            // 5. Применяем исправление (перезаписываем оригинальный файл)
            fs::write(self.file_path, suggested_full_code).await?;
            Ok(())
        } else {
            Err("Generated fix failed verification.".into())
        }
    }

    /// Проверяет, компилируется ли проект после полной замены файла.
    async fn verify_fix(&self, full_code: &str) -> Result<bool, std::io::Error> {
        let temp_dir = std::env::temp_dir();
        let temp_project_path = temp_dir.join("rusty-fixer-temp-project");

        if temp_project_path.exists() {
            fs::remove_dir_all(&temp_project_path).await?;
        }
        copy_dir_all(".", &temp_project_path).await?;
        
        let target_path_in_temp = temp_project_path.join(self.file_path);
        fs::write(&target_path_in_temp, full_code).await?;

        let output = Command::new("cargo")
            .arg("check")
            .current_dir(&temp_project_path)
            .output()?;

        if !output.status.success() {
            println!("    -> Verification failed. New error:");
            let error_output = String::from_utf8_lossy(&output.stderr);
            println!("{}", error_output.lines().take(15).collect::<Vec<_>>().join("\n"));
        }

        fs::remove_dir_all(&temp_project_path).await?;
        
        Ok(output.status.success())
    }

    /// Создает diff-патч между оригинальным и исправленным кодом.
    fn create_patch(&self, original: &str, modified: &str) -> String {
        let patch = diff::lines(original, modified);
        let mut patch_str = String::new();
        for diff_result in patch {
            match diff_result {
                diff::Result::Left(l) => patch_str.push_str(&format!("-{}\n", l)),
                diff::Result::Right(r) => patch_str.push_str(&format!("+{}\n", r)),
                diff::Result::Both(b, _) => patch_str.push_str(&format!(" {}\n", b)),
            }
        }
        patch_str
    }

    /// Применяет diff-патч к оригинальному коду, чтобы восстановить исправленную версию.
    fn apply_patch(&self, original: &str, patch_str: &str) -> String {
        let mut new_lines = Vec::new();
        let original_lines: Vec<&str> = original.lines().collect();
        let mut original_idx = 0;

        for patch_line in patch_str.lines() {
            if patch_line.starts_with('+') {
                new_lines.push(&patch_line[1..]);
            } else if patch_line.starts_with(' ') {
                if original_idx < original_lines.len() {
                    new_lines.push(original_lines[original_idx]);
                    original_idx += 1;
                }
            } else if patch_line.starts_with('-') {
                original_idx += 1;
            }
        }
        new_lines.join("\n")
    }
}

/// Вспомогательная функция для рекурсивного копирования директорий.
async fn copy_dir_all(src: impl AsRef<Path>, dst: impl AsRef<Path>) -> std::io::Result<()> {
    fs::create_dir_all(&dst).await?;
    let mut entries = fs::read_dir(src).await?;
    while let Some(entry) = entries.next_entry().await? {
        let ty = entry.file_type().await?;
        let entry_path = entry.path();
        
        if let Some(file_name) = entry_path.file_name() {
            if file_name == "target" || file_name.to_string_lossy().starts_with('.') {
                continue;
            }
        }

        if ty.is_dir() {
            Box::pin(copy_dir_all(entry.path(), dst.as_ref().join(entry.file_name()))).await?;
        } else {
            fs::copy(entry.path(), dst.as_ref().join(entry.file_name())).await?;
        }
    }
    Ok(())
}