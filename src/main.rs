use colored::*;
use indicatif::{ProgressBar, ProgressStyle};
use serde::Deserialize;
use std::process::{Command, Stdio};
use std::io::{BufRead, BufReader};
use std::sync::{Arc, Mutex};
use std::thread;
use anyhow::{Result, Context};

mod modules;
use modules::cli::{CliArgs, parse_args};
use modules::knowledge_cache::KnowledgeCache;
use modules::llm_interface::LLMInterface;
use modules::web_agent::WebAgent;
use modules::patch_engine::PatchEngine;
use modules::issue_detector::{self, IssueClassification};
use modules::cargo_expert::CargoExpert;
use modules::project_analyzer::ProjectAnalyzer;

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

#[tokio::main]
async fn main() -> Result<()> {
    let args: CliArgs = parse_args();
    println!("{}", "🚀 RustyFixerAI v2.0.0".bold().yellow());

    let spinner = create_spinner("Preparing subsystems...");
    let cache = KnowledgeCache::new().context("Failed to init knowledge cache")?;
    let llm = LLMInterface::new()?;
    let web = WebAgent::new();
    let cargo_expert = CargoExpert::new(&llm);
    let _analyzer = ProjectAnalyzer::new();
    spinner.finish_with_message("Subsystems ready.");

    loop {
        let (errors, warnings) = run_cargo_and_collect("build")
            .context("Cargo build failed to execute")?;

        if errors.is_empty() {
            println!("{}", "✅ No errors found.".green().bold());
            if args.fix_warnings && !warnings.is_empty() {
                println!("{}", "⚠️ Fix-warnings pass enabled".yellow().bold());
                // Здесь можно добавить отдельный проход фиксов ворнингов (по аналогии с ошибками)
            }
            break;
        }

        let Some(issue) = issue_detector::prioritize_and_classify(&errors) else {
            println!("{}", "No actionable errors.".yellow());
            break;
        };

        println!("\n{} {}", "Selected issue:".bold(), issue.message.message);
        display_issue_details(&issue.message);

        match issue.classification {
            IssueClassification::CargoManifest => {
                if let Err(e) = cargo_expert.fix_manifest_issue(&issue.message).await {
                    eprintln!("{} {e:#}", "Cargo manifest fix failed:".red().bold());
                    break;
                }
            }
            IssueClassification::Code | IssueClassification::Unknown => {
                let Some(span) = issue.message.spans.first() else {
                    eprintln!("{}", "Compiler message has no spans; skipping.".red());
                    break;
                };
                let target_file = span.file_name.clone();

                let plan = llm.analyze_error(&issue.message.message).await?;
                let web_context = web.investigate(&plan).await.unwrap_or_default();

                let signature = format!("{}::{}", issue.message.message, target_file);
                let patch_engine = PatchEngine::new(
                    &llm,
                    &cache,
                    signature,
                    &issue.message.message,
                    &target_file,
                    &web_context,
                    args.no_cache,
                );

                if let Err(e) = patch_engine.run_and_self_correct().await {
                    eprintln!("{} {e:#}", "Failed to fix code:".red().bold());
                    break;
                }
            }
            IssueClassification::Linker => {
                eprintln!("{}", "Linker issue type is not implemented yet.".yellow());
                break;
            }
        }
    }

    Ok(())
}

fn run_cargo_and_collect(cmd: &str) -> Result<(Vec<CompilerMessage>, Vec<CompilerMessage>)> {
    let mut child = Command::new("cargo")
        .args([cmd, "--message-format=json"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("Failed to spawn cargo {cmd}"))?;

    let messages: Arc<Mutex<Vec<CompilerMessage>>> = Arc::new(Mutex::new(Vec::new()));
    let messages_out = Arc::clone(&messages);
    let messages_err = Arc::clone(&messages);

    let mut threads = Vec::new();

    if let Some(stdout) = child.stdout.take() {
        threads.push(thread::spawn(move || {
            let reader = BufReader::new(stdout);
            for line in reader.lines().flatten() {
                if let Ok(msg) = serde_json::from_str::<CargoMessage>(&line) {
                    if msg.reason == "compiler-message" {
                        if let Some(compiler_msg) = msg.message {
                            messages_out.lock().unwrap().push(compiler_msg);
                        }
                    }
                }
            }
        }));
    }

    if let Some(stderr) = child.stderr.take() {
        threads.push(thread::spawn(move || {
            let reader = BufReader::new(stderr);
            for line in reader.lines().flatten() {
                if let Ok(msg) = serde_json::from_str::<CargoMessage>(&line) {
                    if msg.reason == "compiler-message" {
                        if let Some(compiler_msg) = msg.message {
                            messages_err.lock().unwrap().push(compiler_msg);
                        }
                    }
                }
            }
        }));
    }

    for t in threads {
        t.join().unwrap();
    }
    let status = child.wait()?;
    if !status.success() {
        // Нормальная ситуация при ошибках компиляции; продолжаем парсить
    }

    let mut all = Arc::try_unwrap(messages).unwrap().into_inner().unwrap();
    let mut errors: Vec<_> = all.iter().cloned().filter(|m| m.level == "error").collect();
    let mut warnings: Vec<_> = all.drain(..).filter(|m| m.level == "warning").collect();

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
            .tick_strings(&["⠋","⠙","⠹","⠸","⠼","⠴","⠦","⠧","⠇","⠏"]),
    );
    spinner.set_message(msg.to_string());
    spinner
}
