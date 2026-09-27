//! Shared file-name helpers for module labels.
//!
//! Module labels arrive as untrusted bytes from the capture: they can contain
//! path separators, control characters, trailing dots or Windows-reserved
//! stems. Both the desktop "save" flow and the headless `--extract` flow must
//! map a label to the same safe fragment, so the rules live here instead of
//! being duplicated per caller.
//!
//! Contract of [`sanitize`]:
//! - drops control characters, maps `/ \ : * ? " < > |` to `_`;
//! - caps the result at 64 characters;
//! - trims surrounding dots, spaces, underscores and whitespace;
//! - falls back to `"module"` when nothing remains;
//! - maps stems reserved on Windows (`CON`, `PRN`, `AUX`, `NUL`, `COM1`–`COM9`,
//!   `LPT1`–`LPT9`, matched before the first `.`, case-insensitively) to
//!   `"module"`.
//!
//! [`escape_label`] is the lighter companion for terminal output: it only
//! neutralises control characters (including `\n` and `ESC`, so ANSI escape
//! sequences cannot survive) and preserves every other character, including
//! path separators. Use it for stdout/stderr text; never for file names.

/// Turn a module label into a safe file-name fragment.
///
/// See the module documentation for the full contract.
pub(crate) fn sanitize(label: &str) -> String {
    let cleaned: String = label
        .chars()
        .filter(|character| !character.is_control())
        .map(|character| match character {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
            other => other,
        })
        .collect();
    let capped: String = cleaned.chars().take(64).collect();
    let trimmed = capped
        .trim()
        .trim_end_matches(['.', ' '])
        .trim_matches(|character: char| character == '_' || character.is_whitespace());
    if trimmed.is_empty() {
        return "module".to_owned();
    }
    let stem = trimmed.split('.').next().unwrap_or(trimmed);
    if is_windows_reserved(stem) {
        return "module".to_owned();
    }
    trimmed.to_owned()
}

/// Neutralise terminal control characters in a label for stdout/stderr.
///
/// Every control character (newlines, `ESC`, `NUL`, …) becomes `_`; all other
/// characters — including `/`, whitespace and Unicode — are preserved. The
/// extracted file content is never passed through here, only display text
/// such as `--list` columns and `--extract --stdout` part headers.
pub(crate) fn escape_label(label: &str) -> String {
    label
        .chars()
        .map(|character| {
            if character.is_control() {
                '_'
            } else {
                character
            }
        })
        .collect()
}

/// Whether a file-name stem is reserved on Windows (`CON`, `PRN`, …).
fn is_windows_reserved(stem: &str) -> bool {
    const RESERVED: [&str; 22] = [
        "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
        "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
    ];
    RESERVED.contains(&stem.to_ascii_uppercase().as_str())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_keeps_the_historical_contract() {
        assert_eq!(sanitize("/ip/firewall/filter"), "ip_firewall_filter");
        assert_eq!(sanitize("log"), "log");
        assert_eq!(sanitize("a\x00b\x1Fc"), "abc");
        assert_eq!(sanitize("/"), "module");
        assert_eq!(sanitize("CON.txt"), "module");
        let long = "x".repeat(100);
        assert_eq!(sanitize(&long).chars().count(), 64);
    }
}
