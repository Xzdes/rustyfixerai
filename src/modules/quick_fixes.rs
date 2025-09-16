use anyhow::{Context, Result};
use tokio::fs;

/// Если в файле встречается #[derive(Serialize|Deserialize)] и нет импорта serde,
/// добавляет строку `use serde::{Serialize, Deserialize};` в начало файла.
pub async fn ensure_serde_import(file_path: &str) -> Result<bool> {
    let content = fs::read_to_string(file_path)
        .await
        .with_context(|| format!("Failed to read {}", file_path))?;

    let needs_import = (content.contains("derive(Serialize") || content.contains("derive(Deserialize"))
        && !content.contains("use serde::Serialize")
        && !content.contains("use serde::{Serialize, Deserialize}")
        && !content.contains("use serde::{Deserialize, Serialize}");

    if !needs_import {
        return Ok(false);
    }

    // Вставляем импорт сразу после модульных атрибутов/комментариев или в самое начало
    let mut lines: Vec<String> = content.lines().map(|s| s.to_string()).collect();

    // Ищем первую «содержательную» строку (не пустую и не комментарий).
    let insert_at = lines
        .iter()
        .position(|l| {
            let t = l.trim_start();
            !(t.is_empty() || t.starts_with("//") || t.starts_with("#![") || t.starts_with("#[allow"))
        })
        .unwrap_or(0);

    lines.insert(insert_at, "use serde::{Serialize, Deserialize};".to_string());
    lines.insert(insert_at, "".to_string()); // пустая строка для красоты

    let new_content = lines.join("\n");
    fs::write(file_path, new_content).await
        .with_context(|| format!("Failed to write {}", file_path))?;

    // ВНИМАНИЕ: фигурные скобки в форматной строке нужно экранировать как {{ }}
    println!(
        "    -> QuickFix: inserted `use serde::{{Serialize, Deserialize}};` into {}",
        file_path
    );
    Ok(true)
}
