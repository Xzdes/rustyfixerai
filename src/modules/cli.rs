// src/modules/cli.rs

use clap::Parser;

/// RustyFixerAI: An autonomous AI assistant to fix Rust compilation errors.
///
/// Run this command in the root of your Rust project to automatically
/// find, analyze, and fix build errors.
#[derive(Parser, Debug)]
#[command(version = "2.0.0", author = "Your Name", about, long_about = None)]
pub struct CliArgs {
    /// Enables an additional pass to fix warnings after all errors are resolved.
    #[arg(long, default_value_t = false)]
    pub fix_warnings: bool,

    /// Forces the agent to ignore the local knowledge cache and always
    /// search online for solutions. Useful for getting the freshest fixes.
    #[arg(long, default_value_t = false)]
    pub no_cache: bool,

    /// [NOT IMPLEMENTED] Runs the tool in watch mode, automatically
    /// fixing errors on every file save.
    #[arg(long, default_value_t = false)]
    pub watch: bool,
}

/// Parses command line arguments on application startup.
pub fn parse_args() -> CliArgs {
    CliArgs::parse()
}