//! `--mode funny`: random walks over which word follows which in real
//! prose, so a few words in a row read like the language and a sentence's
//! worth reads like nonsense.

use std::collections::{HashMap, HashSet};

use fastrand::Rng;

use crate::words::Passage;

/// Which words follow which in a body of prose, lowercased and stripped
/// of the punctuation around them.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Chain {
    words: Vec<String>,
    /// The words after each word, each as often as it follows it.
    next: Vec<Vec<u32>>,
    /// The words after each pair the prose continues in more than one way;
    /// elsewhere a walk falls back to `next`, so it never just retraces a
    /// passage.
    branches: HashMap<(u32, u32), Vec<u32>>,
    /// Each passage's first word, where walks start and restart.
    starts: Vec<u32>,
}

impl Chain {
    pub fn new(passages: &[Passage]) -> Self {
        let mut chain = Chain::default();
        let mut ids: HashMap<String, u32> = HashMap::new();
        let mut pairs: HashMap<(u32, u32), Vec<u32>> = HashMap::new();
        for passage in passages {
            let mut walk = Vec::new();
            for word in passage.words.iter().filter_map(|w| bare(w)) {
                let id = *ids.entry(word.clone()).or_insert_with(|| {
                    chain.words.push(word);
                    chain.next.push(Vec::new());
                    u32::try_from(chain.words.len() - 1).unwrap_or(u32::MAX)
                });
                walk.push(id);
            }
            chain.starts.extend(walk.first());
            for w in walk.windows(2) {
                chain.next[w[0] as usize].push(w[1]);
            }
            for w in walk.windows(3) {
                pairs.entry((w[0], w[1])).or_default().push(w[2]);
            }
        }
        let branching =
            |next: &Vec<u32>| next.iter().collect::<HashSet<_>>().len() > 1;
        pairs.retain(|_, next| branching(next));
        chain.branches = pairs;
        chain
    }

    /// Every word the chain knows, once.
    pub fn words(&self) -> &[String] {
        &self.words
    }

    /// `n` words walked from a random passage's first word, restarting
    /// from another wherever the prose never went on.
    pub fn walk(&self, n: usize, rng: &mut Rng) -> Vec<String> {
        if self.starts.is_empty() {
            return Vec::new();
        }
        let mut walk: Vec<u32> = Vec::with_capacity(n);
        while walk.len() < n {
            let next = match *walk.as_slice() {
                [.., a, b] => self.branches.get(&(a, b)).or(self.after(b)),
                [.., b] => self.after(b),
                [] => None,
            };
            let pick =
                |from: &[u32], rng: &mut Rng| from[rng.usize(..from.len())];
            walk.push(match next {
                Some(next) => pick(next, rng),
                None => pick(&self.starts, rng),
            });
        }
        walk.into_iter().map(|id| self.words[id as usize].clone()).collect()
    }

    fn after(&self, word: u32) -> Option<&Vec<u32>> {
        Some(&self.next[word as usize]).filter(|next| !next.is_empty())
    }
}

/// A word as `--mode funny` types it: lowercase but for `I`, without the
/// punctuation around it; `None` for numbers and lone punctuation.
fn bare(word: &str) -> Option<String> {
    let word = word.trim_matches(|c: char| !c.is_alphabetic());
    if word.is_empty() || word.chars().any(|c| c.is_numeric()) {
        return None;
    }
    if word == "I" || word.starts_with("I'") {
        return Some(word.to_string());
    }
    Some(word.to_lowercase())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::words;

    fn chain(texts: &[&str]) -> Chain {
        let passages: Vec<Passage> = texts
            .iter()
            .map(|t| Passage { words: words::split(t), cite: None })
            .collect();
        Chain::new(&passages)
    }

    #[test]
    fn words_lose_case_and_surrounding_punctuation() {
        assert_eq!(bare("«Hello,»").as_deref(), Some("hello"));
        assert_eq!(bare("peut-être.").as_deref(), Some("peut-être"));
        assert_eq!(bare("don't").as_deref(), Some("don't"));
        assert_eq!(bare("I'm").as_deref(), Some("I'm"));
        assert_eq!(bare("Ёлка!").as_deref(), Some("ёлка"));
        assert_eq!((bare("—"), bare("1815,")), (None, None));
    }

    #[test]
    fn the_vocabulary_holds_each_word_once() {
        let c = chain(&["The cat saw the dog.", "A dog saw — 1815 — me."]);
        assert_eq!(c.words(), ["the", "cat", "saw", "dog", "a", "me"]);
    }

    #[test]
    fn walks_only_take_steps_the_prose_took() {
        let texts = ["the cat sat on the mat", "a dog sat on a log by the cat"];
        let c = chain(&texts);
        let pairs: HashSet<(String, String)> = texts
            .iter()
            .flat_map(|t| {
                let w = words::split(t);
                let pairs: Vec<_> = w
                    .windows(2)
                    .map(|p| (p[0].clone(), p[1].clone()))
                    .collect();
                pairs
            })
            .collect();
        let mut rng = Rng::with_seed(9);
        let walk = c.walk(200, &mut rng);
        assert_eq!(walk.len(), 200);
        for p in walk.windows(2) {
            let step = (p[0].clone(), p[1].clone());
            let restart = p[1] == "the" || p[1] == "a";
            assert!(pairs.contains(&step) || restart, "{step:?}");
        }
    }

    #[test]
    fn pairs_branch_only_where_the_prose_does() {
        let c = chain(&["a b c a b d", "x y z x y z"]);
        let id = |w: &str| c.words.iter().position(|v| v == w).unwrap() as u32;
        assert!(c.branches.contains_key(&(id("a"), id("b"))));
        assert!(!c.branches.contains_key(&(id("x"), id("y"))), "always z");
    }

    #[test]
    fn walks_mix_passages_instead_of_retracing_one() {
        let c = chain(&[
            "one day the old man went to the sea and the sea was calm",
            "every night the old woman sang to the moon and the moon was cold",
        ]);
        let mut rng = Rng::with_seed(4);
        let text = c.walk(400, &mut rng).join(" ");
        assert!(text.contains("the sea") && text.contains("the moon"));
        assert!(text.contains("old man") && text.contains("old woman"));
    }

    #[test]
    fn an_empty_chain_walks_nowhere() {
        let mut rng = Rng::with_seed(0);
        assert!(Chain::default().walk(5, &mut rng).is_empty());
        assert!(chain(&["— 12 —"]).walk(5, &mut rng).is_empty());
    }
}
