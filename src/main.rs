// src/main.rs

use colored::*;
use indicatif::{ProgressBar, ProgressStyle};
use serde::Deserialize;
use std::process::{Command, Stdio};
use std::io::{BufRead, BufReader};
use std::sync::{Arc, Mutex};
use std::thread;
use anyhow::Result;

// Объявляем все наши модули
mod modules;
use modules::cli::{CliArgs, parse_args};
use modules::knowledge_cache::KnowledgeCache;
use modules::llm_interface::LLMInterface;
use modules::web_agent::WebAgent;
use modules::patch_engine::PatchEngine;
use modules::issue_detector::{self, IssueClassification};
use modules::cargo_expert::CargoExpert;
use modules::project_analyzer::ProjectAnalyzer;

// --- СТРУКТУРЫ ДЛЯ ПАРСИНГА JSON ОТ CARGO ---

#[derive(Debug, Deserialize, Clone)]
pub struct CargoMessage {
    pub reason: String,
    pub message: Option<CompilerMessage>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct CompilerMessage {
    pub message: String,
    pub level: String,
    pub code: Option<ErrorCode>,
    pub spans: Vec<Span>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct ErrorCode {
    pub code: String,
}

#[derive(Debug, Deserialize, Clone)]
pub struct Span {
    pub file_name: String,
    pub line_start: usize,
    #[serde(default)]
    pub suggested_replacement: Option<String>,
}

// --- ОСНОВНАЯ ЛОГИКА ПРИЛОЖЕНИЯ ---

#[tokio::main]
async fn main() -> Result<()> {
    let args: CliArgs = parse_args();
    println!("{}", "🚀 RustyFixerAI v2.0.0 - Final Version".bold().yellow());

    // Инициализируем все наши инструменты и экспертов
    let cache = KnowledgeCache::new()?;
    let llm = LLMInterface::new();
    let web_agent = WebAgent::new();
    let project_analyzer = ProjectAnalyzer::new();
    let cargo_expert = CargoExpert::new(&llm);

    let mut session_report = SessionReport::new();
    const MAX_ITERATIONS: u32 = 10;

    // --- ГЛАВНЫЙ ИТЕРАТИВНЫЙ ЦИКЛ ИСПРАВЛЕНИЙ ---
    println!("{}", "\n--- Phase 1: Fixing Build Issues ---".bold().magenta());
    for i in 0..MAX_ITERATIONS {
        let spinner = create_spinner("Running `cargo build` to find issues...");
        let (errors, warnings) = run_cargo_build()?;
        spinner.finish_and_clear();

        if errors.is_empty() {
            println!("{}", "✅ No more errors to fix. Build successful!".green());
            session_report.remaining_warnings = warnings.len();
            break;
        }
        
        if i == MAX_ITERATIONS - 1 {
            println!("{}", "Reached max iterations for errors. Halting.".red().bold());
            break;
        }
        
        println!("{}", format!("❌ Build failed. Found {} error(s).", errors.len()).red());
        
        let issue_to_fix = match issue_detector::prioritize_and_classify(&errors) {
            Some(issue) => issue,
            None => {
                println!("{}", "Could not identify a priority issue.".yellow());
                break;
            }
        };

        // --- ВЫЗОВ СООТВЕТСТВУЮЩЕГО ЭКСПЕРТА ---
        let fix_successful = match issue_to_fix.classification {
            IssueClassification::Code => {
                process_code_issue(&issue_to_fix.message, &llm, &web_agent, &cache, &project_analyzer, &args).await?
            }
            IssueClassification::CargoManifest => {
                match cargo_expert.fix_manifest_issue(&issue_to_fix.message).await {
                    Ok(_) => {
                        session_report.manifest_fixed += 1;
                        true
                    },
                    Err(e) => {
                        eprintln!("{}", format!("Cargo Expert failed: {}", e).red());
                        false
                    }
                }
            }
            _ => {
                println!("Don't know how to handle this issue type yet. Halting.");
                false
            }
        };
        
        if !fix_successful {
            println!("{}", "Halting due to a failed fix attempt.".red().bold());
            break;
        }

        if issue_to_fix.classification == IssueClassification::Code {
             session_report.errors_fixed += 1;
        }
    }

    // --- ФИНАЛЬНЫЙ ОТЧЕТ ---
    println!("{}", "\n--- Session Report ---".bold().yellow());
    println!("- Code errors fixed: {}", session_report.errors_fixed);
    println!("- Cargo.toml issues fixed: {}", session_report.manifest_fixed);
    if args.fix_warnings {
        println!("- Warnings fixed: {}", session_report.warnings_fixed);
    }
    println!("----------------------");

    Ok(())
}

/// Обрабатывает ошибку в коде.
async fn process_code_issue(
    issue: &CompilerMessage,
    llm: &LLMInterface,
    agent: &WebAgent,
    cache: &KnowledgeCache,
    _analyzer: &ProjectAnalyzer,
    args: &CliArgs,
) -> Result<bool> {
    let file_path = if let Some(span) = issue.spans.first() {
        span.file_name.clone()
    } else {
        println!("{}", "Could not determine issue location. Skipping.".yellow());
        return Ok(true);
    };

    println!("{}", "\n--- Analyzing Code Issue ---".bold().cyan());
    display_issue_details(issue);
    
    let llm_spinner = create_spinner("Asking LLM for an action plan...");
    let analysis_plan = llm.analyze_error(&issue.message).await?;
    llm_spinner.finish_with_message("LLM analysis complete.");

    let web_spinner = create_spinner("Deploying Web Agent...");
    let web_context = agent.investigate(&analysis_plan).await?;
    web_spinner.finish_with_message("Web Agent investigation complete.");

    let patch_spinner = create_spinner("Generating and verifying a new version of the file...");
    
    let error_code = issue.code.as_ref().map_or("generic", |c| &c.code);
    let error_signature = KnowledgeCache::create_signature(error_code, &issue.message);

    let engine = PatchEngine::new(llm, cache, error_signature, &issue.message, &file_path, &web_context, args.no_cache);

    match engine.run_and_self_correct().await {
        Ok(_) => {
            patch_spinner.finish_with_message("Successfully applied a verified patch!");
            Ok(true)
        }
        Err(e) => {
            patch_spinner.finish_with_message("Failed to apply a fix.");
            eprintln!("{}", format!("Error: {}", e).red());
            Ok(false)
        }
    }
}

fn run_cargo_build() -> Result<(Vec<CompilerMessage>, Vec<CompilerMessage>), std::io::Error> {
    let mut child = Command::new("cargo")
        .arg("build")
        .arg("--message-format=json")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;

    let messages = Arc::new(Mutex::new(Vec::new()));
    let mut threads = Vec::new();

    if let Some(stdout) = child.stdout.take() {
        let messages_clone = Arc::clone(&messages);
        threads.push(thread::spawn(move || {
            let reader = BufReader::new(stdout);
            for line in reader.lines().flatten() {
                if line.starts_with('{') {
                    if let Ok(msg) = serde_json::from_str::<CargoMessage>(&line) {
                        if msg.reason == "compiler-message" {
                            if let Some(compiler_msg) = msg.message {
                                messages_clone.lock().unwrap().push(compiler_msg);
                            }
                        }
                    }
                }
            }
        }));
    }

    if let Some(stderr) = child.stderr.take() {
        let messages_clone = Arc::clone(&messages);
        threads.push(thread::spawn(move || {
            let reader = BufReader::new(stderr);
            for line in reader.lines().flatten() {
                if line.starts_with('{') {
                    if let Ok(msg) = serde_json::from_str::<CargoMessage>(&line) {
                        if msg.reason == "compiler-message" {
                            if let Some(compiler_msg) = msg.message {
                                messages_clone.lock().unwrap().push(compiler_msg);
                            }
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

    let all_messages = Arc::try_unwrap(messages).unwrap().into_inner().unwrap();
    
    let mut errors: Vec<CompilerMessage> = all_messages.iter().filter(|m| m.level == "error").cloned().collect();
    let mut warnings: Vec<CompilerMessage> = all_messages.iter().filter(|m| m.level == "warning").cloned().collect();

    let sort_key = |m: &CompilerMessage| m.spans.first().map_or(usize::MAX, |s| s.line_start);
    errors.sort_by_key(sort_key);
    warnings.sort_by_key(sort_key);
    
    Ok((errors, warnings))
}

fn display_issue_details(issue: &CompilerMessage) {
    let level_colored = if issue.level == "error" {
        issue.level.to_uppercase().red().bold()
    } else {
        issue.level.to_uppercase().yellow().bold()
    };
    
    println!("- {}: {}", "Level".bold(), level_colored);
    println!("- {}: {}", "Message".bold(), issue.message);
    if let Some(code) = &issue.code {
        println!("- {}: {}", "Code".bold(), code.code);
    }
    if let Some(span) = issue.spans.first() {
        println!("- {}: {}", "File".bold(), span.file_name);
        println!("- {}: {}", "Line".bold(), span.line_start);
    }
}

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

struct SessionReport {
    errors_fixed: usize,
    warnings_fixed: usize,
    manifest_fixed: usize,
    remaining_warnings: usize,
}
impl SessionReport {
    fn new() -> Self {
        Self { errors_fixed: 0, warnings_fixed: 0, manifest_fixed: 0, remaining_warnings: 0 }
    }
}