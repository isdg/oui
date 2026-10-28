//! `--mode ngrams:N`: made-up words strung from the letter n-grams of real
//! prose, each n-gram drawn as often as the prose holds it.

use std::collections::{BTreeMap, HashSet};
use std::num::NonZeroUsize;

use fastrand::Rng;

use crate::words::Passage;

/// The N-letter runs within the words of a body of prose, lowercased.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ngrams {
    n: NonZeroUsize,
    /// Every distinct run of letters in the prose.
    words: Vec<String>,
    grams: Weighted<String>,
    /// N-grams in every made-up word: as many as come closest to the mean
    /// letters per run, at least one.
    chunks: usize,
}

impl Ngrams {
    pub fn new(passages: &[Passage], n: NonZeroUsize) -> Self {
        let mut grams = BTreeMap::new();
        let (mut letters, mut count) = (0, 0);
        let mut seen = HashSet::new();
        let mut words = Vec::new();
        let all = passages.iter().flat_map(|p| &p.words);
        for run in all.flat_map(|w| runs(w)) {
            let chars: Vec<char> = run.chars().collect();
            (letters, count) = (letters + chars.len(), count + 1);
            for gram in chars.windows(n.get()) {
                *grams.entry(gram.iter().collect()).or_default() += 1;
            }
            if seen.insert(run.clone()) {
                words.push(run);
            }
        }
        let per = count * n.get();
        let chunks = (letters + per / 2).checked_div(per).unwrap_or(0).max(1);
        Ngrams { n, words, grams: Weighted::new(grams), chunks }
    }

    /// Whether the prose holds no run of N letters.
    pub fn is_empty(&self) -> bool {
        self.grams.values.is_empty()
    }

    /// Every run of letters the prose holds, once.
    pub fn words(&self) -> &[String] {
        &self.words
    }

    /// Letters in each made-up word, the same for all of them.
    pub fn length(&self) -> usize {
        self.chunks * self.n.get()
    }

    /// `count` made-up words of [`Ngrams::length`] letters.
    pub fn take(&self, count: usize, rng: &mut Rng) -> Vec<String> {
        if self.is_empty() {
            return Vec::new();
        }
        let word = |rng: &mut Rng| {
            let grams = (0..self.chunks).map(|_| self.grams.pick(rng));
            grams.map(String::as_str).collect()
        };
        (0..count).map(|_| word(rng)).collect()
    }
}

/// The runs of letters in `word`, lowercased: `dit-il` holds `dit`, `il`.
fn runs(word: &str) -> impl Iterator<Item = String> + '_ {
    word.split(|c: char| !c.is_alphabetic())
        .filter(|run| !run.is_empty())
        .map(str::to_lowercase)
}

/// Values drawn in proportion to their counts.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Weighted<T> {
    values: Vec<T>,
    /// Running totals of the counts, the last one their sum.
    totals: Vec<usize>,
}

impl<T> Weighted<T> {
    fn new(counts: BTreeMap<T, usize>) -> Self {
        let mut sum = 0;
        let (values, totals) = counts
            .into_iter()
            .map(|(value, count)| {
                sum += count;
                (value, sum)
            })
            .unzip();
        Weighted { values, totals }
    }

    fn pick(&self, rng: &mut Rng) -> &T {
        let x = rng.usize(..self.totals[self.totals.len() - 1]);
        &self.values[self.totals.partition_point(|&t| t <= x)]
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;
    use crate::words;

    fn ngrams(texts: &[&str], n: usize) -> Ngrams {
        let passages: Vec<Passage> = texts
            .iter()
            .map(|t| Passage { words: words::split(t), cite: None })
            .collect();
        Ngrams::new(&passages, NonZeroUsize::new(n).unwrap())
    }

    fn grams(n: &Ngrams) -> BTreeSet<&str> {
        n.grams.values.iter().map(String::as_str).collect()
    }

    #[test]
    fn grams_stay_within_runs_of_letters() {
        let n = ngrams(&["Dit-il, «Ёж» 1815: don't!"], 2);
        assert_eq!(grams(&n), ["di", "il", "it", "on", "do", "ёж"].into());
        assert_eq!(n.words(), ["dit", "il", "ёж", "don", "t"]);
    }

    #[test]
    fn single_letters_count_each_occurrence() {
        let n = ngrams(&["aab a"], 1);
        assert_eq!(n.grams.values, ["a", "b"]);
        assert_eq!(n.grams.totals, [3, 4]);
    }

    #[test]
    fn words_are_strung_from_whole_grams() {
        let n = ngrams(&["these cats sat on their mats"], 2);
        let held = grams(&n);
        let mut rng = Rng::with_seed(3);
        let made = n.take(200, &mut rng);
        assert_eq!(made.len(), 200);
        for word in &made {
            let chars: Vec<char> = word.chars().collect();
            assert_eq!(chars.len(), 4, "{word}");
            for gram in chars.chunks(2) {
                let gram: String = gram.iter().collect();
                assert!(held.contains(gram.as_str()), "{word}");
            }
        }
    }

    #[test]
    fn words_take_the_mean_length_in_whole_grams() {
        let text = ["a bb cccc dddddd"];
        let lengths = [1, 2, 3, 5].map(|n| ngrams(&text, n).length());
        assert_eq!(lengths, [3, 4, 3, 5], "13 letters over 4 runs");
    }

    #[test]
    fn draws_follow_the_counts() {
        let n = ngrams(&["aaaaaaaaab"], 1);
        let mut rng = Rng::with_seed(7);
        let letters = n.take(500, &mut rng).concat();
        let bs = letters.chars().filter(|&c| c == 'b').count();
        let share = bs * 100 / letters.len();
        assert!((5..15).contains(&share), "{bs} of {}", letters.len());
    }

    #[test]
    fn prose_too_short_for_n_makes_nothing() {
        let n = ngrams(&["a bc — 12"], 3);
        assert!(n.is_empty());
        assert!(n.take(5, &mut Rng::with_seed(0)).is_empty());
    }
}
