// src/main.rs

use colored::*;
use indicatif::{ProgressBar, ProgressStyle};
use serde::Deserialize;
use std::process::{Command, Stdio};
use std::io::{BufRead, BufReader};
use std::sync::{Arc, Mutex};
use std::thread;

// Объявляем все наши модули
mod modules;
use modules::llm_interface::LLMInterface;
use modules::web_agent::WebAgent;
use modules::patch_engine::PatchEngine;

// --- СТРУКТУРЫ ДЛЯ ПАРСИНГА JSON ОТ CARGO ---

#[derive(Debug, Deserialize, Clone)]
struct CargoMessage {
    reason: String,
    message: Option<CompilerMessage>,
}

#[derive(Debug, Deserialize, Clone)]
struct CompilerMessage {
    message: String,
    level: String,
    code: Option<ErrorCode>,
    spans: Vec<Span>,
}

#[derive(Debug, Deserialize, Clone)]
struct ErrorCode {
    code: String,
}

#[derive(Debug, Deserialize, Clone)]
struct Span {
    file_name: String,
    line_start: usize,
    #[serde(default)]
    suggested_replacement: Option<String>,
}

// --- ОСНОВНАЯ ЛОГИКА ПРИЛОЖЕНИЯ ---

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("{}", "🚀 RustyFixerAI v0.4.0 - Full File Edit Mode".bold().yellow());

    let mut successful_fixes = 0;
    const MAX_ITERATIONS: u32 = 10;

    for i in 0..MAX_ITERATIONS {
        println!("{}", format!("\n--- Iteration {} ---", i + 1).bold().blue());
        let spinner = create_spinner("Running `cargo build` to find errors...");
        let build_errors = run_cargo_build()?;
        spinner.finish_and_clear();

        if build_errors.is_empty() {
            println!("{}", "✅ Build successful! No more errors to fix.".green().bold());
            break;
        }
        
        if i == MAX_ITERATIONS - 1 {
            println!("{}", "Reached max iterations. Halting.".red().bold());
            break;
        }
        
        println!("{}", format!("❌ Build failed. Found {} error(s).", build_errors.len()).red());
        
        let first_error = build_errors.first().unwrap().clone();
        
        let file_path = if let Some(span) = first_error.spans.first() {
            span.file_name.clone()
        } else {
            println!("{}", "Could not determine error location. Halting.".red().bold());
            break;
        };

        println!("{}", "--- Analyzing First Error (Top of the list) ---".bold().cyan());
        display_error_details(&first_error);

        let llm = LLMInterface::new();
        let llm_spinner = create_spinner("Asking LLM for an action plan...");
        let analysis_plan = llm.analyze_error(&first_error.message).await?;
        llm_spinner.finish_with_message("LLM analysis complete.");

        let web_spinner = create_spinner("Deploying Web Agent...");
        let agent = WebAgent::new();
        let web_context = agent.investigate(&analysis_plan).await?;
        web_spinner.finish_with_message("Web Agent investigation complete.");

        let patch_spinner = create_spinner("Generating and verifying a new version of the file...");
        
        // Вызываем новый конструктор PatchEngine без line_number
        let engine = PatchEngine::new(
            &llm,
            &first_error.message,
            &file_path,
            &web_context,
        );

        match engine.run().await {
            Ok(_) => {
                patch_spinner.finish_with_message("Successfully applied a verified patch!");
                successful_fixes += 1;
                continue; 
            }
            Err(e) => {
                patch_spinner.finish_with_message("Failed to apply a fix.");
                eprintln!("{}", format!("Error: {}", e).red());
                println!("{}", "Halting due to failed patch attempt.".red().bold());
                break;
            }
        }
    }

    println!("{}", "\n--- Session Report ---".bold().yellow());
    println!("- Total fixes applied: {}", successful_fixes);
    println!("----------------------");

    Ok(())
}

// --- ВСПОМОГАТЕЛЬНЫЕ ФУНКЦИИ ---

/// Запускает `cargo build` и возвращает **полный и отсортированный** вектор ошибок.
fn run_cargo_build() -> Result<Vec<CompilerMessage>, std::io::Error> {
    let mut child = Command::new("cargo")
        .arg("build")
        .arg("--message-format=json")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;

    let errors = Arc::new(Mutex::new(Vec::new()));
    let mut threads = Vec::new();

    if let Some(stdout) = child.stdout.take() {
        let errors_clone = Arc::clone(&errors);
        threads.push(thread::spawn(move || {
            let reader = BufReader::new(stdout);
            for line in reader.lines().flatten() {
                if line.starts_with('{') {
                    if let Ok(msg) = serde_json::from_str::<CargoMessage>(&line) {
                        if msg.reason == "compiler-message" && msg.message.as_ref().map_or(false, |m| m.level == "error") {
                            errors_clone.lock().unwrap().push(msg.message.unwrap());
                        }
                    }
                }
            }
        }));
    }

    if let Some(stderr) = child.stderr.take() {
        let errors_clone = Arc::clone(&errors);
        threads.push(thread::spawn(move || {
            let reader = BufReader::new(stderr);
            for line in reader.lines().flatten() {
                if line.starts_with('{') {
                    if let Ok(msg) = serde_json::from_str::<CargoMessage>(&line) {
                        if msg.reason == "compiler-message" && msg.message.as_ref().map_or(false, |m| m.level == "error") {
                            errors_clone.lock().unwrap().push(msg.message.unwrap());
                        }
                    }
                }
            }
        }));
    }

    for t in threads {
        t.join().expect("Thread panicked");
    }

    child.wait()?;

    let final_errors_mutex = Arc::try_unwrap(errors).unwrap_or_default();
    let mut final_errors = final_errors_mutex.into_inner().unwrap();
    
    final_errors.sort_by_key(|e| {
        e.spans.first().map_or(usize::MAX, |s| s.line_start)
    });
    
    Ok(final_errors)
}

/// Отображает детали ошибки в консоли.
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

/// Создает и возвращает новый экземпляр спиннера.
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