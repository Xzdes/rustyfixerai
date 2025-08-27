// src/modules/patch_engine.rs

use super::llm_interface::LLMInterface;
use std::path::Path;
use tokio::fs;
use std::process::Command;

pub struct PatchEngine<'a> {
    llm: &'a LLMInterface,
    error_message: &'a str,
    file_path: &'a str,
    web_context: &'a str,
}

impl<'a> PatchEngine<'a> {
    // Конструктор теперь не принимает line_number, так как мы работаем со всем файлом
    pub fn new(
        llm: &'a LLMInterface,
        error_message: &'a str,
        file_path: &'a str,
        web_context: &'a str,
    ) -> Self {
        Self {
            llm,
            error_message,
            file_path,
            web_context,
        }
    }

    /// Главный метод: генерирует, верифицирует и применяет исправление ко всему файлу.
    pub async fn run(&self) -> Result<(), Box<dyn std::error::Error>> {
        // 1. Читаем ВЕСЬ исходный код из файла
        let original_code = fs::read_to_string(self.file_path).await?;

        // 2. Генерируем ПОЛНУЮ ИСПРАВЛЕННУЮ ВЕРСИЮ файла от LLM
        let suggested_full_code = self.llm.generate_full_fix(
            self.error_message,
            &original_code,
            self.web_context,
        ).await?;
        
        println!("    -> LLM suggested a new version of the file.");

        // 3. Верифицируем ПОЛНЫЙ ФАЙЛ
        if self.verify_fix(&suggested_full_code).await? {
            // 4. Применяем исправление (перезаписываем оригинальный файл)
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

        // Очищаем предыдущую временную директорию, если она осталась
        if temp_project_path.exists() {
            fs::remove_dir_all(&temp_project_path).await?;
        }
        
        // Копируем весь проект во временную директорию
        copy_dir_all(".", &temp_project_path).await?;
        
        // Перезаписываем наш целевой файл во временном проекте новым кодом
        let target_path_in_temp = temp_project_path.join(self.file_path);
        fs::write(&target_path_in_temp, full_code).await?;

        // Запускаем `cargo check` (не --quiet), чтобы получить вывод в случае ошибки
        let output = Command::new("cargo")
            .arg("check")
            .current_dir(&temp_project_path)
            .output()?;

        // Отладочный вывод, если верификация провалилась
        if !output.status.success() {
            println!("    -> Verification failed. New error:");
            let error_output = String::from_utf8_lossy(&output.stderr);
            // Выводим только первые 15 строк ошибки для краткости
            println!("{}", error_output.lines().take(15).collect::<Vec<_>>().join("\n"));
        }

        // Обязательно очищаем за собой
        fs::remove_dir_all(&temp_project_path).await?;
        
        Ok(output.status.success())
    }
}

/// Вспомогательная функция для рекурсивного копирования директорий.
/// Игнорирует папку `target`, чтобы ускорить процесс.
async fn copy_dir_all(src: impl AsRef<Path>, dst: impl AsRef<Path>) -> std::io::Result<()> {
    fs::create_dir_all(&dst).await?;
    let mut entries = fs::read_dir(src).await?;
    while let Some(entry) = entries.next_entry().await? {
        let ty = entry.file_type().await?;
        let entry_path = entry.path();
        
        // Пропускаем директорию `target` и скрытые директории (например, .git)
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