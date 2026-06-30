//! Output safety guard: never emit filtered output that is larger than the raw
//! command output.
//!
//! A "compacting" filter that produces *more* tokens than the original wastes
//! the user's context and can confuse the reader. When that happens we fall back
//! to the raw command output, which is exactly what the command would have
//! produced unfiltered. This is RTK's most important correctness invariant:
//! using RTK must never be worse than not using it.

/// Approximate token count using whitespace-delimited words.
///
/// This matches the convention used by RTK's token-savings tests and is a cheap,
/// allocation-free proxy for real tokenizer output.
pub fn count_tokens(text: &str) -> usize {
    text.split_whitespace().count()
}

/// Return `filtered` only when it is no larger (in tokens) than `raw`; otherwise
/// return `raw`.
///
/// Ties keep `filtered` — when the two are equal in token count the compacted
/// form is preferred (it is usually more readable and never worse).
pub fn never_worse<'a>(raw: &'a str, filtered: &'a str) -> &'a str {
    if count_tokens(filtered) > count_tokens(raw) {
        raw
    } else {
        filtered
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_count_tokens_basic() {
        assert_eq!(count_tokens("one two three"), 3);
        assert_eq!(count_tokens("   spaced   out  "), 2);
        assert_eq!(count_tokens(""), 0);
    }

    #[test]
    fn test_never_worse_keeps_smaller_filtered() {
        let raw = "a b c d e f g h";
        let filtered = "a b c";
        assert_eq!(never_worse(raw, filtered), filtered);
    }

    #[test]
    fn test_never_worse_falls_back_when_filtered_bigger() {
        // A filter that inflated the output must be discarded in favor of raw.
        let raw = "ok";
        let filtered = "📊 summary header\n   +1 added, -0 removed\nok";
        assert_eq!(never_worse(raw, filtered), raw);
    }

    #[test]
    fn test_never_worse_tie_prefers_filtered() {
        let raw = "a b c";
        let filtered = "x y z";
        assert_eq!(never_worse(raw, filtered), filtered);
    }

    #[test]
    fn test_never_worse_empty_filtered() {
        let raw = "a b c";
        assert_eq!(never_worse(raw, ""), "");
    }

    #[test]
    fn test_never_worse_empty_raw_keeps_raw_when_filtered_nonempty() {
        // Empty raw means the command produced nothing; any non-empty "filtered"
        // is strictly worse, so we return the empty raw.
        assert_eq!(never_worse("", "noise added"), "");
    }
}
