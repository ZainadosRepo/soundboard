//! Comparing names the way people read them: ignoring case and accents, numbers by value.

use std::cmp::Ordering;
use unicode_normalization::char::is_combining_mark;
use unicode_normalization::UnicodeNormalization;

/// Lowercase without accents, so "Écho" matches "echo".
pub fn fold(s: &str) -> String {
    s.nfd()
        .filter(|c| !is_combining_mark(*c))
        .collect::<String>()
        .to_lowercase()
}

/// Names compare ignoring case, accents and surrounding spaces.
pub fn same_name(a: &str, b: &str) -> bool {
    fold(a.trim()) == fold(b.trim())
}

/// Order of titles in the sidebar: "Set 2" before "Set 10", accents and case ignored.
pub fn natural_cmp(a: &str, b: &str) -> Ordering {
    let (a, b) = (fold(a), fold(b));
    let (mut a, mut b) = (a.chars().peekable(), b.chars().peekable());
    loop {
        match (a.peek().copied(), b.peek().copied()) {
            (None, None) => return Ordering::Equal,
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(x), Some(y)) if x.is_ascii_digit() && y.is_ascii_digit() => {
                let number = |it: &mut std::iter::Peekable<std::str::Chars>| {
                    let mut digits = String::new();
                    while let Some(c) = it.peek().filter(|c| c.is_ascii_digit()) {
                        digits.push(*c);
                        it.next();
                    }
                    digits
                };
                let (x, y) = (number(&mut a), number(&mut b));
                let (x, y) = (x.trim_start_matches('0'), y.trim_start_matches('0'));
                let order = x.len().cmp(&y.len()).then_with(|| x.cmp(y));
                if order != Ordering::Equal {
                    return order;
                }
            }
            (Some(x), Some(y)) => {
                if x != y {
                    return x.cmp(&y);
                }
                a.next();
                b.next();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_sort_by_value() {
        assert_eq!(natural_cmp("Set 2", "Set 10"), Ordering::Less);
        assert_eq!(natural_cmp("set 02", "Set 2"), Ordering::Equal);
    }

    #[test]
    fn accents_and_case_are_ignored() {
        assert_eq!(natural_cmp("Écho", "echo"), Ordering::Equal);
        assert!(same_name("  Bonjour ", "bonjour"));
        assert_eq!(fold("Éléphant"), "elephant");
    }
}
