//! Keys that play tiles, and the system-wide shortcut that brings the app forward.

/// Keys already used by app-wide playback shortcuts (Pause All, Fade Out All).
pub const RESERVED: [char; 2] = [' ', '-'];

/// The hotkey for typed text, or `None` if it isn't a single assignable letter or digit.
/// Stored lowercased.
pub fn normalized(text: &str) -> Option<String> {
    let lowered = text.to_lowercase();
    let mut chars = lowered.chars();
    let c = chars.next()?;
    (chars.next().is_none() && c.is_ascii_alphanumeric() && !RESERVED.contains(&c))
        .then(|| c.to_string())
}

/// A global shortcut needs Ctrl, Alt or the Command/Windows key so it can't swallow plain
/// typing in every other app. `accelerator` uses Tauri's form, e.g. `Ctrl+Alt+S`.
pub fn is_valid_global_shortcut(accelerator: &str) -> bool {
    let parts: Vec<&str> = accelerator.split('+').map(str::trim).collect();
    let Some((key, modifiers)) = parts.split_last() else {
        return false;
    };
    let strong = |m: &&str| {
        matches!(
            m.to_ascii_lowercase().as_str(),
            "ctrl"
                | "control"
                | "alt"
                | "option"
                | "super"
                | "cmd"
                | "command"
                | "commandorcontrol"
        )
    };
    !key.is_empty() && modifiers.iter().any(strong)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_letters_and_digits_only() {
        assert_eq!(normalized("Q").as_deref(), Some("q"));
        assert_eq!(normalized("7").as_deref(), Some("7"));
        assert_eq!(normalized("ab"), None);
        assert_eq!(normalized("é"), None);
        assert_eq!(normalized(" "), None);
        assert_eq!(normalized("-"), None);
        assert_eq!(normalized(""), None);
    }

    #[test]
    fn global_shortcuts_need_a_strong_modifier() {
        assert!(is_valid_global_shortcut("Ctrl+Alt+S"));
        assert!(is_valid_global_shortcut("Super+Shift+F5"));
        assert!(!is_valid_global_shortcut("Shift+S"));
        assert!(!is_valid_global_shortcut("S"));
        assert!(!is_valid_global_shortcut("Ctrl+"));
    }
}
