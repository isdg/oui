//! Extras mixed into random words: punctuation, numbers, capitals and code
//! symbols, each a `--with` value.

use clap::ValueEnum;
use fastrand::Rng;

#[derive(ValueEnum, Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Extra {
    /// Sentences: capitalised starts, commas, quotes, `.` `?` `!` endings.
    #[value(alias = "p")]
    Punctuation,
    /// Numbers in place of some words: counts, years, decimals.
    #[value(alias = "n")]
    Numbers,
    /// Some words capitalised.
    #[value(alias = "c")]
    Capitals,
    /// Code symbols around and between words: `(x)`, `x::`, `->`, `!=`.
    #[value(alias = "s")]
    Symbols,
}

impl Extra {
    pub fn name(self) -> &'static str {
        match self {
            Extra::Punctuation => "punctuation",
            Extra::Numbers => "numbers",
            Extra::Capitals => "capitals",
            Extra::Symbols => "symbols",
        }
    }
}

/// Chance a word becomes a number, gets a symbol, or is capitalised.
const NUMBER: f32 = 0.12;
const SYMBOL: f32 = 0.15;
const CAPITAL: f32 = 0.2;

const WRAPS: [(&str, &str); 14] = [
    ("(", ")"),
    ("[", "]"),
    ("{", "}"),
    ("<", ">"),
    ("\"", "\""),
    ("'", "'"),
    ("", "()"),
    ("", ";"),
    ("", "::"),
    ("&", ""),
    ("*", ""),
    ("#", ""),
    ("$", ""),
    ("@", ""),
];
const OPERATORS: [&str; 14] = [
    "=", "==", "!=", "->", "=>", "+=", "&&", "||", "+", "-", "*", "/", "%", "|",
];

/// Applies extras to a stream of words, keeping sentence state between
/// batches so a timed test can top up mid-sentence.
#[derive(Debug, Clone, Default)]
pub struct Decorator {
    extras: Vec<Extra>,
    /// Words left in the current sentence; 0 starts a new one.
    left: usize,
}

impl Decorator {
    pub fn new(extras: &[Extra]) -> Self {
        Decorator { extras: extras.to_vec(), left: 0 }
    }

    pub fn reset(&mut self) {
        self.left = 0;
    }

    pub fn apply(&mut self, words: Vec<String>, rng: &mut Rng) -> Vec<String> {
        if self.extras.is_empty() {
            return words;
        }
        let mut out = Vec::with_capacity(words.len());
        for word in words {
            self.decorate(word, rng, &mut out);
        }
        out
    }

    fn has(&self, extra: Extra) -> bool {
        self.extras.contains(&extra)
    }

    fn decorate(
        &mut self,
        mut word: String,
        rng: &mut Rng,
        out: &mut Vec<String>,
    ) {
        if self.has(Extra::Numbers) && rng.f32() < NUMBER {
            word = number(rng);
        }
        let mut operator = None;
        if self.has(Extra::Symbols) && rng.f32() < SYMBOL {
            if rng.bool() {
                let (open, close) = WRAPS[rng.usize(..WRAPS.len())];
                word = format!("{open}{word}{close}");
            } else {
                operator = Some(OPERATORS[rng.usize(..OPERATORS.len())]);
            }
        }
        if self.has(Extra::Capitals) && rng.f32() < CAPITAL {
            word = capitalise(&word);
        }
        if self.has(Extra::Punctuation) {
            word = self.punctuate(word, rng);
        }
        out.push(word);
        out.extend(operator.map(str::to_owned));
    }

    fn punctuate(&mut self, word: String, rng: &mut Rng) -> String {
        let word = if self.left == 0 {
            self.left = rng.usize(4..=12);
            capitalise(&word)
        } else {
            word
        };
        self.left -= 1;
        if self.left == 0 {
            let end = match rng.u8(0..20) {
                0..=14 => '.',
                15..=17 => '?',
                _ => '!',
            };
            return format!("{word}{end}");
        }
        match rng.u8(0..100) {
            0..=9 => format!("{word},"),
            10..=11 => format!("{word};"),
            12 => format!("{word}:"),
            13..=15 => format!("\"{word}\""),
            16..=17 => format!("({word})"),
            _ => word,
        }
    }
}

fn number(rng: &mut Rng) -> String {
    match rng.u8(0..20) {
        0..=11 => rng.u32(0..1000).to_string(),
        12..=16 => rng.u32(1900..2031).to_string(),
        _ => format!("{}.{:02}", rng.u32(0..100), rng.u32(0..100)),
    }
}

/// `word` with its first letter in upper case, wherever that letter is.
fn capitalise(word: &str) -> String {
    let Some(i) = word.find(char::is_alphabetic) else {
        return word.to_string();
    };
    let (head, tail) = word.split_at(i);
    let mut chars = tail.chars();
    let first = chars.next().map(|c| c.to_uppercase().collect::<String>());
    format!("{head}{}{}", first.unwrap_or_default(), chars.as_str())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(n: usize) -> Vec<String> {
        ["alpha", "beta", "gamma", "дело"]
            .iter()
            .cycle()
            .take(n)
            .map(|w| w.to_string())
            .collect()
    }

    fn decorate(extras: &[Extra], n: usize) -> Vec<String> {
        Decorator::new(extras).apply(words(n), &mut Rng::with_seed(42))
    }

    fn ends_sentence(w: &str) -> bool {
        w.ends_with(['.', '?', '!'])
    }

    #[test]
    fn no_extras_leave_words_alone() {
        assert_eq!(decorate(&[], 20), words(20));
    }

    #[test]
    fn punctuation_makes_capitalised_sentences() {
        let out = decorate(&[Extra::Punctuation], 400);
        assert!(out[0].starts_with('A'), "{:?}", &out[..5]);
        let starts =
            out.windows(2).filter(|w| ends_sentence(&w[0])).map(|w| &w[1]);
        for start in starts {
            let first = start.chars().find(|c| c.is_alphabetic()).unwrap();
            assert!(first.is_uppercase(), "{start} starts a sentence");
        }
        let sentences = out.iter().filter(|w| ends_sentence(w)).count();
        assert!((400 / 12..=400 / 4).contains(&sentences), "{sentences}");
        assert!(out.iter().any(|w| w.ends_with(',')));
    }

    #[test]
    fn numbers_replace_some_words() {
        let out = decorate(&[Extra::Numbers], 500);
        let numeric =
            |w: &&String| w.chars().all(|c| c.is_ascii_digit() || c == '.');
        let n = out.iter().filter(numeric).count();
        assert!((25..=100).contains(&n), "{n} numbers in 500");
        assert_eq!(out.len(), 500);
    }

    #[test]
    fn symbols_wrap_words_or_add_operators() {
        let out = decorate(&[Extra::Symbols], 500);
        assert!(out.len() > 500, "operators are words of their own");
        assert!(out.iter().any(|w| OPERATORS.contains(&w.as_str())));
        assert!(out.iter().any(|w| w.starts_with('(') && w.ends_with(')')));
    }

    #[test]
    fn capitals_reach_cyrillic_and_wrapped_words() {
        assert_eq!(capitalise("дело"), "Дело");
        assert_eq!(capitalise("(beta)"), "(Beta)");
        assert_eq!(capitalise("42"), "42");
        let out = decorate(&[Extra::Capitals], 500);
        let caps =
            out.iter().filter(|w| w.starts_with(char::is_uppercase)).count();
        assert!((50..=150).contains(&caps), "{caps}");
    }

    #[test]
    fn sentences_carry_over_between_batches() {
        let mut d = Decorator::new(&[Extra::Punctuation]);
        let mut rng = Rng::with_seed(1);
        let first = d.apply(words(3), &mut rng);
        let second = d.apply(words(30), &mut rng);
        if !ends_sentence(first.last().unwrap()) {
            assert!(!second[0].starts_with(char::is_uppercase), "mid-sentence");
        }
        d.reset();
        let start = &d.apply(words(1), &mut rng)[0];
        assert!(start
            .chars()
            .find(|c| c.is_alphabetic())
            .unwrap()
            .is_uppercase());
    }

    #[test]
    fn values_accept_short_names() {
        assert_eq!(Extra::from_str("p", true), Ok(Extra::Punctuation));
        assert_eq!(Extra::from_str("symbols", true), Ok(Extra::Symbols));
    }
}
