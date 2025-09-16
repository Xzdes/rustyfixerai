use crate::CompilerMessage;

#[derive(Debug, PartialEq, Eq, Clone)]
pub enum IssueClassification {
    Code,
    CargoManifest,
    Linker,
    Unknown,
}

#[derive(Debug, Clone)]
pub struct DetectedIssue {
    pub classification: IssueClassification,
    pub message: CompilerMessage,
}

pub fn prioritize_and_classify(errors: &[CompilerMessage]) -> Option<DetectedIssue> {
    errors.first().map(|msg| DetectedIssue {
        classification: classify_message(msg),
        message: msg.clone(),
    })
}

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
