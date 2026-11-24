//! Shared file-name helpers for module labels.
//!
//! Module labels arrive as untrusted bytes from the capture: they can contain
//! path separators, control characters, trailing dots or Windows-reserved
//! stems. Both the desktop "save" flow and the headless `--extract` flow must
//! map a label to the same safe fragment, so the rules live here instead of
//! being duplicated per caller.
//!
//! Contract of [`sanitize`]:
//! - drops control and bidi/zero-width format characters, maps
//!   `/ \ : * ? " < > |` to `_`;
//! - caps the result at 64 characters *and* 120 UTF-8 bytes, so a label made
//!   of four-byte characters cannot exceed the common 255-byte file-name limit
//!   once `.txt` and a disambiguating suffix are appended;
//! - trims surrounding dots, spaces, underscores and whitespace;
//! - falls back to `"module"` when nothing remains;
//! - maps stems reserved on Windows (`CON`, `PRN`, `AUX`, `NUL`, `COM1`–`COM9`,
//!   `LPT1`–`LPT9`, matched before the first `.`, case-insensitively) to
//!   `"module"`.
//!
//! [`escape_label`] is the lighter companion for terminal output: it only
//! neutralises control, bidi and zero-width format characters (including
//! `\n`, `ESC` and U+202E, so ANSI escape sequences and right-to-left spoofing
//! cannot survive) and preserves every other character, including path
//! separators. Use it for stdout/stderr text; never for file names.

/// Maximum characters retained from a label for a file name.
const MAX_FILENAME_CHARS: usize = 64;

/// Maximum UTF-8 bytes retained from a label for a file name.
///
/// Common filesystems cap a component at 255 bytes. The 120-byte cap leaves
/// room for `.txt`, a `-123456` collision suffix and the multi-byte rounding
/// of the stem logic, while still allowing the historical 64 ASCII characters.
const MAX_FILENAME_BYTES: usize = 120;

/// Turn a module label into a safe file-name fragment.
///
/// See the module documentation for the full contract.
pub(crate) fn sanitize(label: &str) -> String {
    let cleaned: String = label
        .chars()
        .filter(|character| !is_unsafe_display_char(*character))
        .map(|character| match character {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
            other => other,
        })
        .collect();
    let capped = cap_filename(&cleaned);
    let trimmed = capped
        .trim()
        .trim_end_matches(['.', ' '])
        .trim_matches(|character: char| character == '_' || character.is_whitespace());
    if trimmed.is_empty() {
        return "module".to_owned();
    }
    let stem = trimmed.split('.').next().unwrap_or(trimmed);
    // Win32 strips trailing spaces and dots from the stem before matching
    // reserved device names, so `CON .txt` and `CON..txt` are reserved too.
    if is_windows_reserved(stem.trim_end_matches(['.', ' ']).trim()) {
        return "module".to_owned();
    }
    trimmed.to_owned()
}

/// Neutralise terminal control characters in a label for stdout/stderr.
///
/// Every control, bidi and zero-width format character becomes `_`; all other
/// characters — including `/`, whitespace and Unicode — are preserved. The
/// extracted file content is never passed through here, only display text
/// such as `--list` columns and `--extract --stdout` part headers.
pub(crate) fn escape_label(label: &str) -> String {
    label
        .chars()
        .map(|character| {
            if is_unsafe_display_char(character) {
                '_'
            } else {
                character
            }
        })
        .collect()
}

/// Cap a cleaned label on both characters and bytes without splitting a char.
fn cap_filename(cleaned: &str) -> String {
    let mut capped = String::new();
    for (index, character) in cleaned.chars().enumerate() {
        if index >= MAX_FILENAME_CHARS || capped.len() + character.len_utf8() > MAX_FILENAME_BYTES {
            break;
        }
        capped.push(character);
    }
    capped
}

/// Whether a character is unsafe in a terminal or a file name.
///
/// `char::is_control` only covers Cc. Unicode format characters (Cf) such as
/// U+202E RIGHT-TO-LEFT OVERRIDE or U+200B ZERO WIDTH SPACE are not controls
/// but can spoof a displayed name or split a visible word, so they are treated
/// the same way. Line/paragraph separators are included for the same reason.
fn is_unsafe_display_char(character: char) -> bool {
    character.is_control()
        || matches!(
            character,
            '\u{061C}'
                | '\u{200B}'..='\u{200F}'
                | '\u{2028}' | '\u{2029}'
                | '\u{202A}'..='\u{202E}'
                | '\u{2060}'..='\u{206F}'
                | '\u{FEFF}'
        )
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
        // Win32 matches reserved stems after stripping trailing spaces and
        // dots, so these are still device names on Windows.
        assert_eq!(sanitize("CON .txt"), "module");
        assert_eq!(sanitize("COM1. .log"), "module");
        assert_eq!(sanitize("lpt9"), "module");
        // A stem that only starts like a device name is not reserved.
        assert_eq!(sanitize("CONSOLE.txt"), "CONSOLE.txt");
        let long = "x".repeat(100);
        assert_eq!(sanitize(&long).chars().count(), 64);
    }

    #[test]
    fn multibyte_labels_fit_the_filesystem_component_limit() {
        // 64 four-byte emoji would be 256 bytes before the `.txt` suffix and
        // exceed the common 255-byte limit, aborting `--extract --all` midway.
        let label = "🚀".repeat(64);
        let sanitized = sanitize(&label);
        assert!(sanitized.len() <= 120, "{} bytes", sanitized.len());
        assert_ne!(sanitized, "");
        assert!(sanitized.is_char_boundary(sanitized.len()));
    }

    #[test]
    fn bidi_and_zero_width_format_characters_are_neutralised() {
        assert_eq!(escape_label("a\u{202E}b"), "a_b");
        assert_eq!(escape_label("a\u{200B}b"), "a_b");
        assert_eq!(sanitize("a\u{202E}b"), "ab");
    }
}
