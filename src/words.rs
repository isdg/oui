//! Word sources: the bundled lists and texts typed in order.

use fastrand::Rng;

/// Bundled word lists, keyed by the name `--language` takes.
pub const LANGUAGES: [(&str, &str); 2] = [
    ("english", include_str!("../resources/english.txt")),
    ("french", include_str!("../resources/french.txt")),
];

pub fn language(name: &str) -> Option<Vec<String>> {
    LANGUAGES.iter().find(|(n, _)| *n == name).map(|(_, text)| split(text))
}

pub fn split(text: &str) -> Vec<String> {
    text.split_whitespace().map(str::to_owned).collect()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Source {
    /// Words drawn at random from a list.
    Random(Vec<String>),
    /// A text typed in order, starting over once exhausted.
    Text(Vec<String>),
}

impl Source {
    pub fn words(&self) -> &[String] {
        match self {
            Source::Random(words) | Source::Text(words) => words,
        }
    }

    /// `n` words; a [`Source::Text`] continues from word `from`.
    pub fn take(&self, from: usize, n: usize, rng: &mut Rng) -> Vec<String> {
        match self {
            Source::Random(pool) => sample(pool, n, rng),
            Source::Text(text) if text.is_empty() => Vec::new(),
            Source::Text(text) => text
                .iter()
                .cycle()
                .skip(from % text.len())
                .take(n)
                .cloned()
                .collect(),
        }
    }
}

pub fn sample(pool: &[String], n: usize, rng: &mut Rng) -> Vec<String> {
    if pool.is_empty() {
        return Vec::new();
    }
    (0..n).map(|_| pool[rng.usize(..pool.len())].clone()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn owned(words: &[&str]) -> Vec<String> {
        words.iter().map(|w| w.to_string()).collect()
    }

    #[test]
    fn every_bundled_language_loads_without_duplicates() {
        for (name, _) in LANGUAGES {
            let words = language(name).unwrap();
            let mut unique = words.clone();
            unique.sort();
            unique.dedup();
            assert!(words.len() >= 150, "{name} has {} words", words.len());
            assert_eq!(unique.len(), words.len(), "{name} has duplicates");
        }
    }

    #[test]
    fn unknown_language_is_none() {
        assert_eq!(language("klingon"), None);
    }

    #[test]
    fn text_continues_from_offset_and_wraps() {
        let text = Source::Text(owned(&["a", "b", "c"]));
        let mut rng = Rng::with_seed(1);
        assert_eq!(text.take(2, 4, &mut rng), owned(&["c", "a", "b", "c"]));
    }

    #[test]
    fn random_draws_only_from_pool() {
        let pool = owned(&["x", "y"]);
        let mut rng = Rng::with_seed(7);
        let words = Source::Random(pool.clone()).take(0, 50, &mut rng);
        assert_eq!(words.len(), 50);
        assert!(words.iter().all(|w| pool.contains(w)));
    }

    #[test]
    fn empty_sources_yield_nothing() {
        let mut rng = Rng::with_seed(0);
        assert!(Source::Random(Vec::new()).take(0, 5, &mut rng).is_empty());
        assert!(Source::Text(Vec::new()).take(3, 5, &mut rng).is_empty());
    }
}
