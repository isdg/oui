//! The sentences of prose, for the probes of `oui read`: where each ends
//! and which are worth asking about.

use std::ops::{Range, RangeInclusive};

/// The words a sentence asked about holds: fewer are too little to
/// recognise it by, more too much to take in at a glance.
pub const ELIGIBLE: RangeInclusive<usize> = 8..=35;

/// What a full stop follows without ending a sentence, lowercase.
const ABBREVIATIONS: [&str; 20] = [
    "mr", "mrs", "ms", "dr", "st", "prof", "rev", "hon", "messrs", "vs", "cf",
    "mme", "mlle", "mgr", "ст", "гл", "им", "св", "проф", "ул",
];

const OPENERS: [char; 5] = ['"', '\'', '(', '[', '-'];
const CLOSERS: [char; 4] = ['"', '\'', ')', ']'];

/// The sentences of `words`, each as its range. One ends at a word closing
/// in `.`, `?` or `!` before a word opening with a capital, unless the
/// stop follows an initial or an abbreviation; the last word ends one.
pub fn split(words: &[String]) -> Vec<Range<usize>> {
    let mut out = Vec::new();
    let mut start = 0;
    for (i, word) in words.iter().enumerate() {
        let rest = &words[i + 1..];
        if rest.is_empty() || (ends(word) && opens(rest)) {
            out.push(start..i + 1);
            start = i + 1;
        }
    }
    out
}

pub fn eligible(sentence: &[String]) -> bool {
    ELIGIBLE.contains(&sentence.len())
}

fn ends(word: &str) -> bool {
    let word = word.trim_end_matches(CLOSERS);
    if word.ends_with(['?', '!']) {
        return true;
    }
    let Some(stem) = word.strip_suffix('.') else { return false };
    let stem = stem.trim_start_matches(OPENERS).to_lowercase();
    let initial = stem.chars().count() == 1;
    // A stop inside the stem is an abbreviation's, as in `e.g.`: an
    // ellipsis is all stops, and may end a sentence.
    let dotted = stem.contains('.') && !stem.ends_with('.');
    !(initial || dotted || ABBREVIATIONS.contains(&stem.as_str()))
}

/// Whether the first of `words` holding more than openers, a dialogue
/// dash alone being none, starts with a capital.
fn opens(words: &[String]) -> bool {
    let first =
        words.iter().find_map(|w| w.trim_start_matches(OPENERS).chars().next());
    first.is_some_and(char::is_uppercase)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::words;

    fn sentences(text: &str) -> Vec<String> {
        let words = words::split(text);
        split(&words).into_iter().map(|s| words[s].join(" ")).collect()
    }

    #[test]
    fn stops_before_a_capital_end_sentences() {
        assert_eq!(
            sentences(
                "It rained. Was it late? No! he said. \"Go.\" (Then.) We"
            ),
            [
                "It rained.",
                "Was it late?",
                "No! he said.",
                "\"Go.\"",
                "(Then.)",
                "We"
            ]
        );
    }

    #[test]
    fn initials_and_abbreviations_end_none() {
        assert_eq!(
            sentences("Mr. Brooke met J. Smith, e.g. at St. Ives. He left."),
            ["Mr. Brooke met J. Smith, e.g. at St. Ives.", "He left."]
        );
        assert_eq!(
            sentences("В 1812 г. Наполеон ждал. Потом ушёл."),
            ["В 1812 г. Наполеон ждал.", "Потом ушёл."]
        );
        assert_eq!(
            sentences("Mme. Verdurin sourit. Elle sortit."),
            ["Mme. Verdurin sourit.", "Elle sortit."]
        );
    }

    #[test]
    fn dialogue_dashes_and_ellipses_are_seen_through() {
        assert_eq!(
            sentences("— Viens… dit-il. — Non… Elle rit."),
            ["- Viens... dit-il.", "- Non...", "Elle rit."]
        );
    }

    #[test]
    fn the_ranges_cover_the_words_in_order() {
        let words = words::split("One. Two three. four");
        assert_eq!(split(&words), [0..1, 1..4]);
        assert!(split(&[]).is_empty());
    }

    #[test]
    fn eligible_sentences_are_neither_short_nor_long() {
        let of = |n: usize| vec!["w".to_string(); n];
        assert!(!eligible(&of(7)) && eligible(&of(8)));
        assert!(eligible(&of(35)) && !eligible(&of(36)));
    }
}
