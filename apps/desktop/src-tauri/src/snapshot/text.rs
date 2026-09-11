//! Conservative display redaction, applied before shortening metadata text.
use regex::Regex;
use std::sync::OnceLock;

pub fn redact(text: &str) -> String {
    static PATTERNS: OnceLock<[Regex; 5]> = OnceLock::new();
    let patterns = PATTERNS.get_or_init(|| [
        Regex::new(r"(?s)-----BEGIN [^-\n]*PRIVATE KEY-----.*?(?:-----END [^-\n]*PRIVATE KEY-----|\z)").unwrap(),
        Regex::new(r#"(?i)\b[\w.-]*(?:token|secret|password|cookie|api[_ -]?key|private[_ -]?key|authorization|credential|access[_ -]?key)[\w.-]*["'`*]*\s*(?:[:=|]|\bis\b)\s*.*"#).unwrap(),
        Regex::new(r"(?i)\b(?:Bearer|Basic)\s+[A-Za-z0-9_./+~=-]+").unwrap(),
        Regex::new(r"(?i)[a-z][a-z0-9+.-]*://[^/\s@]+@").unwrap(),
        Regex::new(r"\b(?:sk-[A-Za-z0-9_-]{8,}|gh[pousr]_[A-Za-z0-9_]{8,}|github_pat_[A-Za-z0-9_]{8,}|AIza[A-Za-z0-9_-]{20,}|AKIA[A-Z0-9]{16}|eyJ[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+)\b").unwrap(),
    ]);
    let text = patterns[0].replace_all(text, "[REDACTED]");
    let mut lines = Vec::new();
    let mut sensitive_indent = None;
    for line in text.lines() {
        let indent = line.len() - line.trim_start().len();
        if let Some(previous) = sensitive_indent {
            if line.trim().is_empty() || indent > previous {
                continue;
            }
            sensitive_indent = None;
        }
        let mut line = line.to_owned();
        if patterns[1].is_match(&line) {
            line = patterns[1].replace_all(&line, "[REDACTED]").into_owned();
            sensitive_indent = Some(indent);
        }
        for pattern in &patterns[2..] {
            line = pattern.replace_all(&line, "[REDACTED]").into_owned();
        }
        lines.push(line);
    }
    lines.join("\n")
}
