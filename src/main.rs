// src/main.rs

use colored::*;
use indicatif::{ProgressBar, ProgressStyle};
use serde::Deserialize;
use std::process::{Command, Stdio};
use std::io::{BufRead, BufReader};

// Объявляем наши модули
mod modules;
use modules::llm_interface::LLMInterface;
use modules::web_agent::WebAgent;

// --- СТРУКТУРЫ ДЛЯ ПАРСИНГА JSON ОТ CARGO ---
// Эти структуры должны быть полными, чтобы `serde` мог успешно
// десериализовать JSON-ответы от `cargo build`.

#[derive(Debug, Deserialize)]
struct CargoMessage {
    reason: String,
    message: Option<CompilerMessage>,
}

#[derive(Debug, Deserialize)]
struct CompilerMessage {
    message: String,
    level: String,
    code: Option<ErrorCode>,
    spans: Vec<Span>,
}

#[derive(Debug, Deserialize)]
struct ErrorCode {
    code: String,
}

#[derive(Debug, Deserialize)]
struct Span {
    file_name: String,
    line_start: usize,
    #[serde(default)] // Используем default, если поле отсутствует
    suggested_replacement: Option<String>,
}

// --- ОСНОВНАЯ ЛОГИКА ПРИЛОЖЕНИЯ ---

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("{}", "🚀 RustyFixerAI v0.1.0 - Initializing...".bold().yellow());

    // --- ШАГ 1: ЗАПУСК СБОРКИ И ПОИСК ОШИБОК ---
    let spinner = create_spinner("Running initial `cargo build`...");
    let build_errors = run_cargo_build()?;
    spinner.finish_and_clear();

    if build_errors.is_empty() {
        println!("{}", "✅ Build successful! No errors to fix.".green());
        return Ok(());
    }
    
    println!("{}", format!("❌ Build failed. Found {} error(s).", build_errors.len()).red());
    
    // Главный цикл обработки ошибок (пока обрабатываем только первую)
    if let Some(first_error) = build_errors.first() {
        println!("{}", "\n--- Analyzing First Error ---".bold().cyan());
        display_error_details(first_error);

        // --- ШАГ 2: АНАЛИЗ ОШИБКИ С ПОМОЩЬЮ LLM ---
        let llm_spinner = create_spinner("Asking local LLM to analyze the error...");
        let llm = LLMInterface::new();
        let analysis_plan = llm.analyze_error(&first_error.message).await?;
        llm_spinner.finish_with_message("LLM analysis complete.");

        println!("{}", "\n--- Autonomous Action Plan ---".bold().cyan());
        println!("- {}: {}", "Summary".bold(), analysis_plan.error_summary);
        println!("- {}: {}", "Search Keywords".bold(), analysis_plan.search_keywords);

        // --- ШАГ 3: СБОР КОНТЕКСТА ИЗ ИНТЕРНЕТА ---
        let web_spinner = create_spinner("Deploying Web Agent to gather context...");
        let agent = WebAgent::new();
        let web_context = agent.investigate(&analysis_plan.search_keywords).await?;
        web_spinner.finish_with_message("Web Agent investigation complete.");
        
        if web_context.is_empty() {
            println!("{}", "Web Agent could not find relevant context.".yellow());
        } else {
            println!("{}", "\n--- Collected Web Context (first 500 chars) ---".bold().cyan());
            // Выводим только часть для краткости
            let snippet: String = web_context.chars().take(500).collect();
            println!("{}...", snippet);
        }
    }

    println!("{}", "\n[NEXT STEP] Generating a code fix... (Implementation pending)".yellow());

    Ok(())
}

// --- ВСПОМОГАТЕЛЬНЫЕ ФУНКЦИИ ---

/// Запускает `cargo build` и возвращает вектор ошибок компиляции.
fn run_cargo_build() -> Result<Vec<CompilerMessage>, std::io::Error> {
    let mut child = Command::new("cargo")
        .arg("build")
        .arg("--message-format=json")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;

    let stdout = child.stdout.take().expect("Failed to open stdout");
    let reader = BufReader::new(stdout);
    let mut errors = Vec::new();

    for line in reader.lines() {
        if let Ok(line_content) = line {
            // Пропускаем строки, которые не являются JSON-объектами
            if !line_content.starts_with('{') {
                continue;
            }
            if let Ok(msg) = serde_json::from_str::<CargoMessage>(&line_content) {
                if msg.reason == "compiler-message" {
                    if let Some(compiler_msg) = msg.message {
                        if compiler_msg.level == "error" {
                            errors.push(compiler_msg);
                        }
                    }
                }
            }
        }
    }
    child.wait()?;
    Ok(errors)
}

/// Отображает детали ошибки в консоли в структурированном виде.
fn display_error_details(error: &CompilerMessage) {
    println!("- {}: {}", "Message".bold(), error.message);
    if let Some(code) = &error.code {
        println!("- {}: {}", "Error Code".bold(), code.code);
    }
    if let Some(span) = error.spans.first() {
        println!("- {}: {}", "File".bold(), span.file_name);
        println!("- {}: {}", "Line".bold(), span.line_start);
    }
}

/// Создает и возвращает новый, стилизованный экземпляр спиннера.
fn create_spinner(msg: &str) -> ProgressBar {
    let spinner = ProgressBar::new_spinner();
    spinner.enable_steady_tick(std::time::Duration::from_millis(120));
    spinner.set_style(
        ProgressStyle::with_template("{spinner:.blue} {msg}")
            .unwrap()
            .tick_strings(&["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"]),
    );
    spinner.set_message(msg.to_string());
    spinner
}