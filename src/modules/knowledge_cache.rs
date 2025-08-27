// src/modules/knowledge_cache.rs

use rusqlite::{Connection, Result};
use std::path::Path;

const DB_FILE: &str = ".rusty_fixer_cache.db";

/// Структура для управления локальной базой данных (кэшем)
/// для хранения пар "ошибка -> успешное решение".
pub struct KnowledgeCache {
    conn: Connection,
}

impl KnowledgeCache {
    /// Создает новый экземпляр кэша.
    /// Инициализирует базу данных и таблицу, если они не существуют.
    pub fn new() -> Result<Self> {
        // База данных будет создана в текущей директории, где запущен инструмент
        let path = Path::new(DB_FILE);
        let conn = Connection::open(path)?;
        
        // Создаем таблицу, если она еще не создана.
        // `error_signature` - это уникальный ключ, основанный на коде ошибки и сообщении.
        // `solution_patch` - это полный текст исправленного файла.
        conn.execute(
            "CREATE TABLE IF NOT EXISTS solutions (
                error_signature TEXT PRIMARY KEY,
                solution_patch  TEXT NOT NULL,
                timestamp       INTEGER NOT NULL
            )",
            [],
        )?;
        
        Ok(Self { conn })
    }

    /// Генерирует уникальную "подпись" для ошибки, чтобы использовать ее как ключ.
    /// Мы берем код ошибки и первое предложение сообщения, чтобы быть устойчивыми
    /// к мелким изменениям в путях к файлам или именах переменных.
    pub fn create_signature(error_code: &str, error_message: &str) -> String {
        let first_sentence = error_message.split('.').next().unwrap_or("").trim();
        format!("{}:{}", error_code, first_sentence)
    }

    /// Ищет готовое решение в кэше по подписи ошибки.
    /// Возвращает `Some(solution_patch)`, если найдено, и `None` в противном случае.
    pub fn lookup(&self, signature: &str) -> Result<Option<String>> {
        let mut stmt = self.conn.prepare(
            "SELECT solution_patch FROM solutions WHERE error_signature = ?1 ORDER BY timestamp DESC LIMIT 1"
        )?;
        
        let mut rows = stmt.query_map([signature], |row| row.get(0))?;

        if let Some(result) = rows.next() {
            Ok(Some(result?))
        } else {
            Ok(None)
        }
    }

    /// Сохраняет новое, успешно примененное решение в кэш.
    /// Если запись с такой подписью уже существует, она будет перезаписана.
    pub fn store(&self, signature: &str, solution_patch: &str) -> Result<()> {
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();

        // `REPLACE INTO` - это удобная команда SQLite, которая делает `INSERT`
        // или `UPDATE`, если ключ уже существует.
        self.conn.execute(
            "REPLACE INTO solutions (error_signature, solution_patch, timestamp) VALUES (?1, ?2, ?3)",
            &[signature, solution_patch, &timestamp.to_string()],
        )?;
        
        Ok(())
    }
}