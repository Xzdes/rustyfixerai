// src/modules/cli.rs

use clap::Parser;

/// RustyFixerAI: Автономный AI-ассистент для исправления ошибок компиляции Rust.
///
/// Запустите эту команду в корневой директории вашего Rust-проекта,
/// чтобы автоматически найти, проанализировать и исправить ошибки сборки.
#[derive(Parser, Debug)]
#[command(version = "1.0.0", author = "Your Name", about, long_about = None)]
pub struct CliArgs {
    /// Включает дополнительный проход для исправления предупреждений (warnings)
    /// после того, как все критические ошибки (errors) были устранены.
    #[arg(long, default_value_t = false)]
    pub fix_warnings: bool,

    /// Заставляет агента игнорировать локальный кэш знаний и всегда
    /// выполнять поиск в интернете. Полезно для получения самых свежих решений.
    #[arg(long, default_value_t = false)]
    pub no_cache: bool,

    /// [ПОКА НЕ РЕАЛИЗОВАНО] Запускает инструмент в режиме наблюдения,
    /// автоматически исправляя ошибки при каждом сохранении файла.
    #[arg(long, default_value_t = false)]
    pub watch: bool,
}

/// Парсит аргументы командной строки при запуске приложения.
pub fn parse_args() -> CliArgs {
    CliArgs::parse()
}