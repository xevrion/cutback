//! Terminal styling.
//!
//! Colour goes to a terminal and never to a pipe, the same rule git follows,
//! so `cutback log | grep` and friends see plain text. NO_COLOR and a dumb
//! TERM both turn it off, and CLICOLOR_FORCE turns it back on for the case
//! where someone is piping into a pager that understands escapes.

use std::io::IsTerminal;
use std::sync::OnceLock;

fn enabled() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| {
        if std::env::var_os("NO_COLOR").is_some() {
            return false;
        }
        if std::env::var_os("CLICOLOR_FORCE").is_some_and(|v| v != "0") {
            return true;
        }
        if std::env::var("TERM").is_ok_and(|t| t == "dumb") {
            return false;
        }
        std::io::stdout().is_terminal()
    })
}

macro_rules! style {
    ($name:ident, $code:literal) => {
        pub fn $name(text: &str) -> String {
            if enabled() {
                format!("\x1b[{}m{text}\x1b[0m", $code)
            } else {
                text.to_string()
            }
        }
    };
}

style!(bold, "1");
style!(dim, "2");
style!(red, "31");
style!(green, "32");
style!(yellow, "33");
style!(blue, "34");
style!(cyan, "36");

/// Commit ids, matching git's choice so the two read alike side by side.
pub fn commit_id(text: &str) -> String {
    yellow(text)
}

/// Names of things the user made: clips, tracks, effects, branches.
pub fn subject(text: &str) -> String {
    cyan(text)
}

/// Timecodes and durations.
pub fn time(text: &str) -> String {
    green(text)
}

/// Whether to style output. Shares the decision with the colour helpers so
/// that CLICOLOR_FORCE and NO_COLOR apply to everything consistently.
pub fn is_tty() -> bool {
    enabled()
}

/// Visible width of a string, ignoring escape sequences and counting a wide
/// character as two columns. Filenames in a video project routinely contain
/// CJK text and emoji, and treating those as one column each misaligns every
/// row after them.
pub fn width(text: &str) -> usize {
    let mut w = 0;
    let mut in_escape = false;
    for c in text.chars() {
        if in_escape {
            if c == 'm' {
                in_escape = false;
            }
            continue;
        }
        if c == '\x1b' {
            in_escape = true;
            continue;
        }
        w += char_width(c);
    }
    w
}

/// Approximate east asian width. This covers the ranges that actually turn up
/// in file names, rather than implementing the full property table.
fn char_width(c: char) -> usize {
    let c = c as u32;
    match c {
        // Combining marks take no space of their own.
        0x0300..=0x036F => 0,
        0x1100..=0x115F // Hangul jamo
        | 0x2E80..=0x303E // CJK radicals, kangxi
        | 0x3041..=0x33FF // Hiragana through CJK compatibility
        | 0x3400..=0x4DBF // CJK extension A
        | 0x4E00..=0x9FFF // CJK unified
        | 0xA000..=0xA4CF // Yi
        | 0xAC00..=0xD7A3 // Hangul syllables
        | 0xF900..=0xFAFF // CJK compatibility ideographs
        | 0xFE30..=0xFE6F // CJK compatibility forms
        | 0xFF00..=0xFF60 // Fullwidth forms
        | 0xFFE0..=0xFFE6
        | 0x1F300..=0x1F64F // Emoji
        | 0x1F900..=0x1F9FF
        | 0x20000..=0x3FFFD => 2,
        _ => 1,
    }
}

/// Shortens to `max` columns, putting an ellipsis in the middle so that both
/// the start of a name and its extension stay readable. Clip names in a video
/// project are often long and share a prefix, so cutting only the tail would
/// leave rows that all look the same.
pub fn ellipsize(text: &str, max: usize) -> String {
    if width(text) <= max || max < 6 {
        return text.to_string();
    }

    let keep = max - 1;
    let head_cols = keep.div_ceil(2);
    let tail_cols = keep - head_cols;

    let mut head = String::new();
    let mut used = 0;
    for c in text.chars() {
        let cw = char_width(c);
        if used + cw > head_cols {
            break;
        }
        head.push(c);
        used += cw;
    }

    let mut tail: Vec<char> = Vec::new();
    let mut used = 0;
    for c in text.chars().rev() {
        let cw = char_width(c);
        if used + cw > tail_cols {
            break;
        }
        tail.push(c);
        used += cw;
    }
    tail.reverse();

    format!("{head}…{}", tail.into_iter().collect::<String>())
}

/// Pads to `cols` visible columns.
pub fn pad(text: &str, cols: usize) -> String {
    let w = width(text);
    if w >= cols {
        text.to_string()
    } else {
        format!("{text}{}", " ".repeat(cols - w))
    }
}

/// Terminal width, for deciding how much room a line has.
pub fn terminal_width() -> usize {
    // COLUMNS is what a shell exports, and is the portable way to ask without
    // an ioctl crate. Fall back to the classic 80 when it is not set.
    std::env::var("COLUMNS")
        .ok()
        .and_then(|v| v.parse().ok())
        .filter(|w| *w >= 40)
        .unwrap_or(80)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn width_ignores_escapes() {
        assert_eq!(width("plain"), 5);
        assert_eq!(width("\x1b[31mred\x1b[0m"), 3);
    }

    #[test]
    fn wide_characters_count_double() {
        assert_eq!(width("日本語"), 6);
        assert_eq!(width("ab"), 2);
    }

    #[test]
    fn ellipsize_keeps_both_ends() {
        let out = ellipsize("VID_20251212_092005.mp4", 14);
        assert!(out.starts_with("VID_"), "{out}");
        assert!(out.ends_with("mp4"), "{out}");
        assert!(width(&out) <= 14, "{out} is {} wide", width(&out));
    }

    #[test]
    fn ellipsize_leaves_short_text_alone() {
        assert_eq!(ellipsize("short.mp4", 20), "short.mp4");
    }

    #[test]
    fn pad_accounts_for_wide_characters() {
        assert_eq!(width(&pad("日本", 6)), 6);
        assert_eq!(width(&pad("ab", 6)), 6);
    }
}
