use std::{
    collections::{HashMap, HashSet},
    sync::LazyLock,
};

pub const MAX_RECOMMENDER_TERMS: usize = 48;

static RECOMMENDER_STOPWORDS: LazyLock<HashSet<&'static str>> = LazyLock::new(|| {
    HashSet::from([
        "about", "after", "again", "against", "all", "also", "and", "any", "are", "around",
        "because", "been", "before", "being", "between", "both", "but", "can", "could", "does",
        "doing", "done", "down", "during", "each", "even", "every", "for", "from", "had", "has",
        "have", "having", "here", "how", "into", "its", "just", "like", "many", "more", "most",
        "much", "not", "now", "off", "onto", "other", "our", "out", "over", "really", "should",
        "since", "some", "such", "than", "that", "the", "their", "them", "then", "there", "these",
        "they", "this", "those", "through", "under", "until", "very", "was", "were", "what",
        "when", "where", "which", "while", "who", "will", "with", "would", "your",
    ])
});

/// What the stemmer makes of stopwords, like `thi` from "this". Terms stored
/// before stopwords were checked ahead of stemming are full of these.
static RECOMMENDER_STOPWORD_STEMS: LazyLock<HashSet<String>> = LazyLock::new(|| {
    RECOMMENDER_STOPWORDS
        .iter()
        .map(|word| stem_recommender_term(word.to_string()))
        .filter(|stem| !RECOMMENDER_STOPWORDS.contains(stem.as_str()))
        .collect()
});

/// Pieces of URLs. Article text has its URLs stripped before extraction, but
/// terms stored before that are full of these.
const URL_TERMS: [&str; 16] = [
    "http", "https", "www", "com", "org", "net", "html", "htm", "php", "png", "jpg", "jpeg", "gif",
    "svg", "webp", "utm",
];

const URL_PREFIXES: [&str; 3] = ["http://", "https://", "www."];

/// Replace placeholder in template with data.
pub fn render_template(template: &str, data: &[(&str, &str)]) -> String {
    let mut result = String::from(template);

    for (placeholder, value) in data {
        result = result.replace(placeholder, value);
    }

    result
}

/// Convert uint to readable format. Example: `12345 -> 12,345`.
pub fn readable_uint(int_str: String) -> String {
    let mut s = String::new();
    for (i, char) in int_str.chars().rev().enumerate() {
        if i % 3 == 0 && i != 0 {
            s.insert(0, ',');
        }
        s.insert(0, char);
    }
    s
}

/// Whether a term carries no topic: a stopword, what the stemmer made of one,
/// a piece of a URL, or something with more digits than a name like `arm64`
/// has, such as a commit hash.
pub fn is_recommender_noise(term: &str) -> bool {
    RECOMMENDER_STOPWORDS.contains(term)
        || RECOMMENDER_STOPWORD_STEMS.contains(term)
        || URL_TERMS.contains(&term)
        || term.chars().filter(char::is_ascii_digit).count() > 3
}

/// A crude suffix stemmer. Its rules are what the stored terms were made
/// with, so changing them would split old and new articles' terms apart.
fn stem_recommender_term(mut term: String) -> String {
    if term.ends_with("ies") && term.len() > 4 {
        term.truncate(term.len() - 3);
        term.push('y');
    } else if term.ends_with("ing") && term.len() > 5 {
        term.truncate(term.len() - 3);
    } else if term.ends_with("ed") && term.len() > 4 {
        term.truncate(term.len() - 2);
    } else if term.ends_with('s') && term.len() > 3 && !term.ends_with("ss") {
        term.truncate(term.len() - 1);
    }

    term
}

fn normalize_recommender_term(token: &str) -> Option<String> {
    if token.len() < 3 || token.chars().all(|char| char.is_ascii_digit()) {
        return None;
    }

    let lowercase = token.to_ascii_lowercase();
    if is_recommender_noise(&lowercase) {
        return None;
    }

    let stemmed = stem_recommender_term(lowercase);
    (stemmed.len() >= 3 && !is_recommender_noise(&stemmed)).then_some(stemmed)
}

/// Blanks out URLs so link targets in the scraped markdown don't add their
/// hosts, path segments and hashes to the terms.
fn strip_urls(text: &str) -> String {
    let mut stripped = String::with_capacity(text.len());
    let mut rest = text;

    while let Some(start) = URL_PREFIXES
        .iter()
        .filter_map(|prefix| rest.find(prefix))
        .min()
    {
        let (before, url) = rest.split_at(start);
        stripped.push_str(before);
        stripped.push(' ');

        // The URL starts with a letter, so `end` is never 0 and `rest` shrinks.
        let end = url
            .find(|char: char| {
                char.is_whitespace()
                    || matches!(char, '(' | ')' | '[' | ']' | '<' | '>' | '"' | '\'')
            })
            .unwrap_or(url.len());
        rest = url.split_at(end).1;
    }

    stripped.push_str(rest);
    stripped
}

fn score_recommender_terms(text: &str, weight: f64, scores: &mut HashMap<String, f64>) {
    for token in strip_urls(text).split(|char: char| !char.is_ascii_alphanumeric()) {
        let Some(term) = normalize_recommender_term(token) else {
            continue;
        };

        let length_bonus = ((term.len().saturating_sub(4)) as f64).min(6.0) * 0.08;
        *scores.entry(term).or_insert(0.0) += weight + length_bonus;
    }
}

pub fn extract_recommender_terms(title: &str, content: Option<&str>) -> Vec<String> {
    let mut scores = HashMap::new();
    score_recommender_terms(title, 4.0, &mut scores);

    if let Some(content) = content {
        score_recommender_terms(content, 1.0, &mut scores);
    }

    let mut ranked_terms = scores.into_iter().collect::<Vec<_>>();
    ranked_terms.sort_by(|(left_term, left_score), (right_term, right_score)| {
        right_score
            .total_cmp(left_score)
            .then_with(|| right_term.len().cmp(&left_term.len()))
            .then_with(|| left_term.cmp(right_term))
    });

    ranked_terms
        .into_iter()
        .take(MAX_RECOMMENDER_TERMS)
        .map(|(term, _)| term)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stopwords_are_dropped_before_stemming() {
        let terms = extract_recommender_terms(
            "This is having a moment",
            Some("During the build, this does things to the compiler"),
        );

        for junk in ["thi", "hav", "dur", "doe"] {
            assert!(!terms.contains(&junk.to_string()), "{junk} in {terms:?}");
        }
        assert!(terms.contains(&"compiler".to_string()));
    }

    #[test]
    fn urls_and_hashes_are_not_terms() {
        let terms = extract_recommender_terms(
            "Fixing arm64 builds",
            Some(
                "See [the docs](https://github.com/rust-lang/rust/blob/master/README.md), \
                 www.example.org/page and commit 1e963f82914ddfb47820034c5c85205a362ed73f.",
            ),
        );

        for junk in ["http", "https", "com", "blob", "readme", "example", "page"] {
            assert!(!terms.contains(&junk.to_string()), "{junk} in {terms:?}");
        }
        assert!(!terms.iter().any(|term| term.starts_with("1e963f")));
        assert!(terms.contains(&"arm64".to_string()));
        assert!(terms.contains(&"doc".to_string()));
    }

    #[test]
    fn recognizes_noise_in_stored_terms() {
        for junk in ["thi", "hav", "http", "com", "1e963f82914ddfb4782"] {
            assert!(is_recommender_noise(junk), "{junk}");
        }
        for term in ["rust", "arm64", "compiler", "github"] {
            assert!(!is_recommender_noise(term), "{term}");
        }
    }
}
