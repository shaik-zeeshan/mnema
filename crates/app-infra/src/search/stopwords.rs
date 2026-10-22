//! Filler words Capture Search neither matches nor highlights ("kvm switch on
//! amazon" must not require — or mark — every "on"). Dropped per query only when
//! something else is left to search: an all-filler query ("the who") and quoted
//! phrases keep every word. Quick Look's highlighter mirrors this list
//! (`apps/desktop/src/lib/quick-recall/context-highlight.ts`) — keep them in sync.

const SEARCH_STOPWORDS: &[&str] = &[
    "a", "an", "and", "are", "as", "at", "be", "been", "being", "but", "by", "can", "could", "did",
    "do", "does", "for", "from", "had", "has", "have", "he", "her", "him", "his", "how", "i", "if",
    "in", "into", "is", "it", "its", "me", "my", "of", "on", "or", "our", "over", "she", "should",
    "so", "some", "than", "that", "the", "their", "them", "then", "they", "this", "to", "was",
    "we", "were", "what", "when", "where", "who", "why", "will", "with", "would", "you", "your",
    "about", "all", "any", "may", "might", "shall",
];

/// True when every alphanumeric run in `word` is a stopword ("on", "on-the").
/// A word with no alphanumerics is not a stopword (it matches nothing anyway).
pub(super) fn is_stopword(word: &str) -> bool {
    let mut parts = word
        .split(|ch: char| !ch.is_alphanumeric())
        .filter(|part| !part.is_empty())
        .peekable();
    parts.peek().is_some()
        && parts.all(|part| SEARCH_STOPWORDS.contains(&part.to_lowercase().as_str()))
}

#[cfg(test)]
mod tests {
    use super::is_stopword;

    #[test]
    fn stopwords_are_whole_words_case_insensitively() {
        assert!(is_stopword("On"));
        assert!(is_stopword("on-the"));
        assert!(!is_stopword("online"));
        assert!(!is_stopword("kvm"));
        assert!(!is_stopword("--"));
    }
}
