/// Cuts `s` to `max` columns, marking the cut with an ellipsis.
pub fn truncate_chars(s: &str, max: usize) -> String {
    if max == 0 {
        return String::new();
    }
    if s.chars().count() <= max {
        return s.to_string();
    }
    let keep = max.saturating_sub(1).max(1);
    let mut out: String = s.chars().take(keep).collect();
    out.push('…');
    out
}

/// Pads (or cuts) to exactly `width` columns.
pub fn fit(s: &str, width: usize) -> String {
    let t = truncate_chars(s, width);
    let n = t.chars().count();
    format!("{t}{}", " ".repeat(width.saturating_sub(n)))
}

/// A score if every character of `pattern` appears in `text` in order (case
/// insensitive), higher for consecutive and word-boundary matches.
pub fn fuzzy_match(text: &str, pattern: &str) -> Option<i32> {
    if pattern.is_empty() {
        return Some(0);
    }
    let t: Vec<char> = text.to_lowercase().chars().collect();
    let p: Vec<char> = pattern.to_lowercase().chars().collect();
    let (mut score, mut pi, mut last): (i32, usize, Option<usize>) = (0, 0, None);
    for (ti, &tc) in t.iter().enumerate() {
        if pi < p.len() && tc == p[pi] {
            if last.is_some_and(|l| ti == l + 1) {
                score += 5;
            }
            if ti == 0 || matches!(t.get(ti.wrapping_sub(1)), Some('/' | '-' | '_' | '.' | ' ')) {
                score += 10;
            }
            last = Some(ti);
            pi += 1;
            score += 1;
        }
    }
    (pi == p.len()).then_some(score)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncates_and_fits() {
        assert_eq!(truncate_chars("abcdef", 4), "abc…");
        assert_eq!(fit("ab", 4), "ab  ");
    }

    #[test]
    fn fuzzy() {
        assert!(fuzzy_match("dbeaver", "dbv").is_some());
        assert!(fuzzy_match("kitty", "z").is_none());
    }
}
