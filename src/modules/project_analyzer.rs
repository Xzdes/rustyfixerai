// src/modules/project_analyzer.rs

use anyhow::Result;
use std::path::{Path, PathBuf};
use tokio::fs;
use walkdir::WalkDir;

/// Структура для анализа исходного кода проекта.
pub struct ProjectAnalyzer;

impl ProjectAnalyzer {
    pub fn new() -> Self {
        Self
    }

    /// Находит определение символа (struct, enum, fn) в проекте.
    /// Возвращает полный путь к файлу и его содержимое.
    pub async fn find_symbol_definition(
        &self,
        symbol_name: &str,
        project_root: &Path,
    ) -> Result<Option<(PathBuf, String)>> {
        // Мы будем искать в файлах .rs, игнорируя target и скрытые директории.
        for entry in WalkDir::new(project_root)
            .into_iter()
            .filter_map(Result::ok)
            .filter(|e| !e.path().to_string_lossy().contains("target"))
            .filter(|e| e.path().is_file() && e.path().extension().map_or(false, |ext| ext == "rs"))
        {
            let file_path = entry.path();
            let content = fs::read_to_string(file_path).await?;
            
            // Простой текстовый поиск. В будущем можно заменить на более умный парсинг.
            let search_patterns = [
                format!("struct {}", symbol_name),
                format!("enum {}", symbol_name),
                format!("fn {}", symbol_name),
                format!("trait {}", symbol_name),
                format!("type {}", symbol_name),
            ];

            if search_patterns.iter().any(|pattern| content.contains(pattern)) {
                return Ok(Some((file_path.to_path_buf(), content)));
            }
        }

        Ok(None)
    }
}