// src/modules/issue_detector.rs

use crate::CompilerMessage;

#[derive(Debug, PartialEq, Eq, Clone)]
pub enum IssueClassification {
    /// An error in Rust code (the most common case)
    Code,
    /// An error related to dependencies or features in Cargo.toml
    CargoManifest,
    /// A linker error (reserved for future use)
    Linker,
    /// An unknown error type
    Unknown,
}

#[derive(Debug, Clone)]
pub struct DetectedIssue {
    pub classification: IssueClassification,
    pub message: CompilerMessage,
}

/// Analyzes a list of errors and returns the highest priority one to fix.
pub fn prioritize_and_classify(errors: &[CompilerMessage]) -> Option<DetectedIssue> {
    errors.first().map(|msg| {
        DetectedIssue {
            classification: classify_message(msg),
            message: msg.clone(),
        }
    })
}

/// Classifies a single compiler message.
fn classify_message(message: &CompilerMessage) -> IssueClassification {
    let error_text = &message.message;

    let cargo_keywords = [
        "cannot find crate",
        "can't find crate",
        "unresolved import",
        "no such extern crate",
        "cannot find derive macro",
    ];

    if cargo_keywords.iter().any(|&kw| error_text.contains(kw)) {
        return IssueClassification::CargoManifest;
    }
    
    IssueClassification::Code
}