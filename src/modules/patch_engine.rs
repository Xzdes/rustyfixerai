// src/modules/patch_engine.rs

use super::llm_interface::LLMInterface;
use std::path::Path;
use tokio::fs;
use std::process::Command;

pub struct PatchEngine<'a> {
    llm: &'a LLMInterface,
    error_message: &'a str,
    file_path: &'a str,
    line_number: usize,
    web_context: &'a str,
}

impl<'a> PatchEngine<'a> {
    pub fn new(
        llm: &'a LLMInterface,
        error_message: &'a str,
        file_path: &'a str,
        line_number: usize,
        web_context: &'a str,
    ) -> Self {
        Self {
            llm,
            error_message,
            file_path,
            line_number,
            web_context,
        }
    }

    /// Главный метод: генерирует, верифицирует и применяет исправление.
    pub async fn run(&self) -> Result<(), Box<dyn std::error::Error>> {
        // 1. Сбор локального контекста (код вокруг ошибки)
        let local_code_context = self.get_local_code_context().await?;

        // 2. Генерация гипотезы исправления
        let suggested_fix = self.llm.generate_fix(
            self.error_message,
            &local_code_context,
            self.web_context,
        ).await?;

        // 3. Верификация исправления
        if self.verify_fix(&suggested_fix).await? {
            // 4. Применение исправления
            self.apply_fix(&suggested_fix).await?;
            Ok(())
        } else {
            Err("Generated fix failed verification.".into())
        }
    }

    /// Читает исходный файл и извлекает код вокруг строки с ошибкой.
    async fn get_local_code_context(&self) -> Result<String, std::io::Error> {
        let content = fs::read_to_string(self.file_path).await?;
        let lines: Vec<&str> = content.lines().collect();
        
        let start = self.line_number.saturating_sub(10);
        let end = (self.line_number + 10).min(lines.len());
        
        Ok(lines[start..end].join("\n"))
    }

    /// Проверяет, компилируется ли код после применения исправления.
    async fn verify_fix(&self, fix: &str) -> Result<bool, std::io::Error> {
        // Создаем временный файл для проверки
        let temp_dir = std::env::temp_dir();
        let temp_file_path = temp_dir.join(Path::new(self.file_path).file_name().unwrap());

        // Применяем исправление к содержимому и пишем во временный файл
        let original_content = fs::read_to_string(self.file_path).await?;
        let mut original_lines: Vec<String> = original_content.lines().map(String::from).collect();
        
        // Очень простая стратегия замены: заменяем строку с ошибкой.
        // В будущем здесь можно использовать более сложный diff/patch.
        if self.line_number > 0 && self.line_number <= original_lines.len() {
            original_lines[self.line_number - 1] = fix.to_string();
        }
        
        let modified_content = original_lines.join("\n");
        fs::write(&temp_file_path, modified_content).await?;

        // Создаем временную копию всего проекта для `cargo check`
        let temp_project_path = temp_dir.join("rusty-fixer-temp-project");
        if temp_project_path.exists() {
            fs::remove_dir_all(&temp_project_path).await?;
        }
        // Копируем текущую директорию во временную
        copy_dir_all(".", &temp_project_path).await?;
        
        // Заменяем оригинальный файл во временном проекте на наш измененный
        let target_path_in_temp = temp_project_path.join(self.file_path);
        fs::copy(&temp_file_path, &target_path_in_temp).await?;

        // Запускаем `cargo check` во временном проекте
        let output = Command::new("cargo")
            .arg("check")
            .arg("--quiet") // Нам не нужен детальный вывод, только код завершения
            .current_dir(&temp_project_path)
            .output()?;

        // Очищаем временные файлы и директории
        fs::remove_file(&temp_file_path).await?;
        fs::remove_dir_all(&temp_project_path).await?;
        
        Ok(output.status.success())
    }

    /// Перезаписывает оригинальный файл исправленным кодом.
    async fn apply_fix(&self, fix: &str) -> Result<(), std::io::Error> {
        let original_content = fs::read_to_string(self.file_path).await?;
        let mut original_lines: Vec<String> = original_content.lines().map(String::from).collect();

        if self.line_number > 0 && self.line_number <= original_lines.len() {
            original_lines[self.line_number - 1] = fix.to_string();
        }
        
        let modified_content = original_lines.join("\n");
        fs::write(self.file_path, modified_content).await?;
        Ok(())
    }
}

// Вспомогательная функция для рекурсивного копирования директорий
async fn copy_dir_all(src: impl AsRef<Path>, dst: impl AsRef<Path>) -> std::io::Result<()> {
    fs::create_dir_all(&dst).await?;
    let mut entries = fs::read_dir(src).await?;
    while let Some(entry) = entries.next_entry().await? {
        let ty = entry.file_type().await?;
        if ty.is_dir() {
            // --- ИСПРАВЛЕНИЕ ЗДЕСЬ ---
            // Мы "оборачиваем" рекурсивный вызов в Box::pin, чтобы разорвать
            // бесконечную вложенность типов.
            Box::pin(copy_dir_all(entry.path(), dst.as_ref().join(entry.file_name()))).await?;
        } else {
            fs::copy(entry.path(), dst.as_ref().join(entry.file_name())).await?;
        }
    }
    Ok(())
}