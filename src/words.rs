//! Word sources: the bundled lists, passages of real prose, and texts typed
//! in order.

use std::ffi::OsStr;
use std::fmt;
use std::fs;
use std::num::NonZeroUsize;
use std::path::Path;

use anyhow::{Context, Result};
use clap::builder::{EnumValueParser, PossibleValue, TypedValueParser};
use clap::{Arg, Command, ValueEnum};
use fastrand::Rng;

use crate::chain::Chain;
use crate::ngrams::Ngrams;

/// What a test types, as `--mode` takes it; see [`Mode::to_possible_value`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Words,
    Text,
    Classics,
    External,
    Funny,
    Practice,
    /// Made-up words of N-letter runs.
    Ngrams(NonZeroUsize),
}

impl Mode {
    /// Whether the mode types whole passages, which take no extras.
    pub fn passages(self) -> bool {
        matches!(self, Mode::Text | Mode::Classics | Mode::External)
    }
}

// By hand rather than derived, since `ngrams:N` carries its N.
impl ValueEnum for Mode {
    fn value_variants<'a>() -> &'a [Self] {
        &[
            Mode::Words,
            Mode::Text,
            Mode::Classics,
            Mode::External,
            Mode::Funny,
            Mode::Practice,
            Mode::Ngrams(NonZeroUsize::MIN),
        ]
    }

    fn from_str(input: &str, ignore_case: bool) -> Result<Self, String> {
        if let Some(n) = order(input, ignore_case) {
            return Ok(Mode::Ngrams(n));
        }
        let named = |mode: &&Mode| {
            let value = mode.to_possible_value();
            value.is_some_and(|v| v.matches(input, ignore_case))
        };
        let mode = Self::value_variants().iter().find(named).copied();
        mode.ok_or_else(|| format!("invalid variant: {input}"))
    }

    fn to_possible_value(&self) -> Option<PossibleValue> {
        let (name, aliases, help): (_, &[_], _) = match self {
            Mode::Words => {
                ("words", &["w"], "Random words from the language's list")
            }
            Mode::Text => (
                "text",
                &["t"],
                "Passages of real prose: the classics, your texts and \
                 external ones",
            ),
            Mode::Classics => (
                "classics",
                &["c"],
                "Passages of the bundled public-domain classics only",
            ),
            Mode::External => (
                "ext",
                &["external", "e"],
                "Passages `oui external` extracted into ~/.oui/external/",
            ),
            Mode::Funny => (
                "funny",
                &["funnywords", "f"],
                "Words that follow each other as in that prose: phrases \
                 that almost mean something",
            ),
            Mode::Practice => (
                "practice",
                &["p"],
                "Random words richest in the bigrams that cost your tests \
                 of the language the most time, misses included",
            ),
            Mode::Ngrams(_) => (
                "ngrams",
                &["n"],
                "Made-up words strung from the letters of that prose, each \
                 as often as it holds it; ngrams:N (or nN) strings N-letter \
                 runs instead, so n2 types bigrams",
            ),
        };
        Some(PossibleValue::new(name).aliases(aliases).help(help))
    }
}

/// The N of `ngrams:N` or `nN`.
fn order(input: &str, ignore_case: bool) -> Option<NonZeroUsize> {
    let input = match ignore_case {
        true => input.to_lowercase(),
        false => input.to_string(),
    };
    let n = input.strip_prefix("ngrams:").or_else(|| input.strip_prefix('n'));
    n?.parse().ok()
}

/// As history records it and `--mode` takes it back.
impl fmt::Display for Mode {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match (self, self.to_possible_value()) {
            (Mode::Ngrams(n), _) => write!(f, "ngrams:{n}"),
            (_, Some(value)) => f.write_str(value.get_name()),
            (_, None) => unreachable!("every mode has a value"),
        }
    }
}

/// `--mode` as clap's enum parser takes it, plus `ngrams:N`. Name it in
/// `value_parser = ModeParser`: `value_parser!` would pick the enum parser.
#[derive(Debug, Clone, Copy)]
pub struct ModeParser;

impl TypedValueParser for ModeParser {
    type Value = Mode;

    fn parse_ref(
        &self,
        cmd: &Command,
        arg: Option<&Arg>,
        value: &OsStr,
    ) -> Result<Mode, clap::Error> {
        let ignore_case = arg.is_some_and(Arg::is_ignore_case_set);
        if let Some(n) = value.to_str().and_then(|v| order(v, ignore_case)) {
            return Ok(Mode::Ngrams(n));
        }
        EnumValueParser::<Mode>::new().parse_ref(cmd, arg, value)
    }

    fn possible_values(
        &self,
    ) -> Option<Box<dyn Iterator<Item = PossibleValue> + '_>> {
        let values = Mode::value_variants().iter();
        Some(Box::new(values.filter_map(ValueEnum::to_possible_value)))
    }
}

/// Bundled word lists, keyed by the name `--language` takes.
pub const LANGUAGES: [(&str, &str); 3] = [
    ("english", include_str!("../resources/english.txt")),
    ("french", include_str!("../resources/french.txt")),
    ("russian", include_str!("../resources/russian.txt")),
];

pub fn language(name: &str) -> Option<Vec<String>> {
    LANGUAGES.iter().find(|(n, _)| *n == name).map(|(_, text)| split(text))
}

/// Bundled passages for `--mode classics` and `text`, separated by blank
/// lines: excerpts of public-domain classics, each cited.
pub const CLASSICS: [(&str, &str); 3] = [
    ("english", include_str!("../resources/classics/english.txt")),
    ("french", include_str!("../resources/classics/french.txt")),
    ("russian", include_str!("../resources/classics/russian.txt")),
];

pub fn split(text: &str) -> Vec<String> {
    normalise(text).split_whitespace().map(str::to_owned).collect()
}

/// Typographic characters folded into what a keyboard types directly.
pub fn normalise(text: &str) -> String {
    let fold = |c| match c {
        '‘' | '’' | '‚' | '′' => '\'',
        '“' | '”' | '„' | '«' | '»' | '″' => '"',
        '–' | '—' | '―' | '−' => '-',
        '\u{a0}' | '\u{202f}' | '\u{2009}' => ' ',
        c => c,
    };
    text.chars().map(fold).collect::<String>().replace('…', "...")
}

/// A passage of prose, and where it is from when its text says.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Passage {
    pub words: Vec<String>,
    pub cite: Option<String>,
}

/// The passages of `text`: its blocks between blank lines, as words. A
/// `[[Author, Work (year)]]` line cites the passages after it, up to the
/// next such line; `[[]]` stops citing.
pub fn passages(text: &str) -> Vec<Passage> {
    let mut out = Vec::new();
    let mut block = String::new();
    let mut cite = None;
    for line in text.lines().chain([""]) {
        let citation = citation(line);
        if !line.trim().is_empty() && citation.is_none() {
            block.push_str(line);
            block.push('\n');
            continue;
        }
        let words = split(&block);
        if !words.is_empty() {
            out.push(Passage { words, cite: cite.clone() });
        }
        block.clear();
        if let Some(name) = citation {
            cite = (!name.is_empty()).then(|| name.to_string());
        }
    }
    out
}

/// The source a line naming one as `[[...]]` cites.
fn citation(line: &str) -> Option<&str> {
    let inner = line.trim().strip_prefix("[[")?.strip_suffix("]]")?;
    Some(inner.trim())
}

/// The bundled classics in `language`, if any.
pub fn classics(language: &str) -> Vec<Passage> {
    let bundled = CLASSICS.iter().find(|(n, _)| *n == language);
    bundled.map_or_else(Vec::new, |(_, text)| passages(text))
}

/// Passages for `language`: the bundled ones, then those of every `.txt`
/// file in `dir/<language>/`, in name order.
pub fn load_passages(language: &str, dir: &Path) -> Result<Vec<Passage>> {
    let mut out = classics(language);
    let dir = dir.join(language);
    let entries = match fs::read_dir(&dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(out),
        Err(e) => {
            return Err(e).with_context(|| format!("reading {}", dir.display()))
        }
    };
    let mut files: Vec<_> = entries
        .filter_map(|e| Some(e.ok()?.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "txt"))
        .collect();
    files.sort();
    for file in files {
        let text = fs::read_to_string(&file)
            .with_context(|| format!("reading {}", file.display()))?;
        out.extend(passages(&text));
    }
    Ok(out)
}

/// The passages `oui external` wrote for `language` into `dir`, if any.
pub fn external(language: &str, dir: &Path) -> Result<Vec<Passage>> {
    let path = dir.join(format!("{language}.txt"));
    match fs::read_to_string(&path) {
        Ok(text) => Ok(passages(&text)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(e) => Err(e).with_context(|| format!("reading {}", path.display())),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Source {
    /// Words drawn at random from a list.
    Random(Vec<String>),
    /// A text typed in order, starting over once exhausted.
    Text(Vec<String>),
    /// Passages of prose, flattened; `starts` indexes each one's first word
    /// and `cites` holds each one's source.
    Passages {
        words: Vec<String>,
        starts: Vec<usize>,
        cites: Vec<Option<String>>,
    },
    /// Walks over which word follows which in prose.
    Funny(Chain),
    /// Made-up words strung from the letter n-grams of prose.
    Ngrams(Ngrams),
}

impl Source {
    pub fn passages(passages: Vec<Passage>) -> Self {
        let mut starts = Vec::with_capacity(passages.len());
        let mut cites = Vec::with_capacity(passages.len());
        let mut words = Vec::new();
        for passage in passages {
            starts.push(words.len());
            cites.push(passage.cite);
            words.extend(passage.words);
        }
        Source::Passages { words, starts, cites }
    }

    pub fn words(&self) -> &[String] {
        match self {
            Source::Random(words)
            | Source::Text(words)
            | Source::Passages { words, .. } => words,
            Source::Funny(chain) => chain.words(),
            Source::Ngrams(ngrams) => ngrams.words(),
        }
    }

    /// The word a new test starts on: a random passage's first, else 0.
    pub fn start(&self, rng: &mut Rng) -> usize {
        match self {
            Source::Passages { starts, .. } if !starts.is_empty() => {
                starts[rng.usize(..starts.len())]
            }
            _ => 0,
        }
    }

    /// Words in the passage starting at word `start`.
    pub fn passage_len(&self, start: usize) -> Option<usize> {
        let Source::Passages { words, starts, .. } = self else {
            return None;
        };
        let i = starts.iter().position(|&s| s == start)?;
        Some(starts.get(i + 1).unwrap_or(&words.len()) - start)
    }

    /// Where the passage starting at word `start` is from, if cited.
    pub fn cite(&self, start: usize) -> Option<&str> {
        let Source::Passages { starts, cites, .. } = self else {
            return None;
        };
        let i = starts.iter().position(|&s| s == start)?;
        cites[i].as_deref()
    }

    /// `n` words; texts and passages continue from word `from`, wrapping.
    pub fn take(&self, from: usize, n: usize, rng: &mut Rng) -> Vec<String> {
        match self {
            Source::Random(pool) => sample(pool, n, rng),
            Source::Funny(chain) => chain.walk(n, rng),
            Source::Ngrams(ngrams) => ngrams.take(n, rng),
            Source::Text(text) | Source::Passages { words: text, .. } => {
                if text.is_empty() {
                    return Vec::new();
                }
                let cycle = text.iter().cycle().skip(from % text.len());
                cycle.take(n).cloned().collect()
            }
        }
    }
}

pub fn sample(pool: &[String], n: usize, rng: &mut Rng) -> Vec<String> {
    if pool.is_empty() {
        return Vec::new();
    }
    (0..n).map(|_| pool[rng.usize(..pool.len())].clone()).collect()
}

/// `n` words drawn in proportion to how many `targets` bigrams each holds,
/// or plain samples when no word holds any.
pub fn practice(
    pool: &[String],
    targets: &[(char, char)],
    n: usize,
    rng: &mut Rng,
) -> Vec<String> {
    let scored: Vec<(&String, usize)> = pool
        .iter()
        .map(|w| (w, score(w, targets)))
        .filter(|&(_, s)| s > 0)
        .collect();
    let total: usize = scored.iter().map(|&(_, s)| s).sum();
    if total == 0 {
        return sample(pool, n, rng);
    }
    let pick = |mut x: usize| {
        for &(word, s) in &scored {
            if x < s {
                return word.clone();
            }
            x -= s;
        }
        unreachable!("x is below the total score")
    };
    (0..n).map(|_| pick(rng.usize(..total))).collect()
}

/// Occurrences of `targets` in `word`, its boundaries included as `' '`.
fn score(word: &str, targets: &[(char, char)]) -> usize {
    let padded: Vec<char> =
        std::iter::once(' ').chain(word.chars()).chain([' ']).collect();
    let pairs = padded.windows(2);
    pairs.filter(|p| targets.contains(&(p[0], p[1]))).count()
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;
    use crate::keyboard::Layout;

    #[test]
    fn typography_folds_to_typeable_ascii() {
        let text = "«Je m’endors» — dit-il… “ok”";
        assert_eq!(normalise(text), "\"Je m'endors\" - dit-il... \"ok\"");
        assert_eq!(split("a\u{a0}b"), ["a", "b"]);
    }

    fn uncited(words: &[&str]) -> Passage {
        Passage { words: owned(words), cite: None }
    }

    fn cited(words: &[&str], cite: &str) -> Passage {
        Passage { words: owned(words), cite: Some(cite.to_string()) }
    }

    #[test]
    fn modes_are_named_as_the_cli_takes_them() {
        let three = Mode::Ngrams(NonZeroUsize::new(3).unwrap());
        for &mode in Mode::value_variants().iter().chain([&three]) {
            let name = mode.to_string();
            assert_eq!(Mode::from_str(&name, false), Ok(mode), "{name}");
        }
        assert_eq!(three.to_string(), "ngrams:3");
    }

    #[test]
    fn ngrams_take_their_order_and_default_to_letters() {
        let n = |n| Ok(Mode::Ngrams(NonZeroUsize::new(n).unwrap()));
        assert_eq!(Mode::from_str("ngrams", true), n(1));
        assert_eq!(Mode::from_str("ngrams:2", true), n(2));
        assert_eq!(Mode::from_str("N3", true), n(3));
        assert!(Mode::from_str("N3", false).is_err());
        for bad in ["n0", "ngrams:", "ngrams:x", "ngrams2", "nope"] {
            assert!(Mode::from_str(bad, true).is_err(), "{bad}");
        }
    }

    #[test]
    fn modes_answer_to_their_first_letter() {
        for &mode in Mode::value_variants() {
            let letter = &mode.to_string()[..1];
            assert_eq!(Mode::from_str(letter, true), Ok(mode), "{letter}");
        }
    }

    #[test]
    fn clap_parses_ngrams_with_their_order() {
        let arg = Arg::new("mode").long("mode");
        let cmd = Command::new("t").arg(arg.value_parser(ModeParser));
        let parse = |v: &str| {
            let m = cmd.clone().try_get_matches_from(["t", "--mode", v]);
            m.map(|m| *m.get_one::<Mode>("mode").unwrap())
        };
        let two = Mode::Ngrams(NonZeroUsize::new(2).unwrap());
        assert_eq!(parse("ngrams:2").unwrap(), two);
        assert_eq!(parse("n2").unwrap(), two);
        assert_eq!(parse("f").unwrap(), Mode::Funny);
        assert!(parse("n0").is_err());
    }

    #[test]
    fn passages_split_on_blank_lines() {
        let text = "one two\nthree\n\n\n  \nfour\r\n\r\nfive six\n";
        assert_eq!(
            passages(text),
            [
                uncited(&["one", "two", "three"]),
                uncited(&["four"]),
                uncited(&["five", "six"])
            ]
        );
    }

    #[test]
    fn citations_carry_to_the_next_one() {
        let text = "zero\n[[A, B (1900)]]\none\n\ntwo\n [[ C ]] \n\n\
                    three\n[[]]\nfour\n[[not]] a citation";
        assert_eq!(
            passages(text),
            [
                uncited(&["zero"]),
                cited(&["one"], "A, B (1900)"),
                cited(&["two"], "A, B (1900)"),
                cited(&["three"], "C"),
                uncited(&["four", "[[not]]", "a", "citation"]),
            ]
        );
    }

    #[test]
    fn every_bundled_passage_is_cited() {
        for (name, text) in CLASSICS {
            let passages = passages(text);
            assert!(passages.len() >= 100, "{name} has {}", passages.len());
            let uncited = passages.iter().find(|p| p.cite.is_none());
            assert_eq!(uncited, None, "{name}");
            assert!(LANGUAGES.iter().any(|(l, _)| *l == name));
        }
    }

    #[test]
    fn every_bundled_character_has_a_key() {
        let lists = LANGUAGES.iter().map(|&(name, text)| (name, split(text)));
        let texts = CLASSICS.iter().map(|&(name, text)| {
            (name, passages(text).into_iter().flat_map(|p| p.words).collect())
        });
        // Russian prose quotes the odd Latin name or apostrophe, typed by
        // switching layout, so any bundled layout will do.
        let typeable = |c: char| {
            [Layout::Qwerty, Layout::Jcuken]
                .iter()
                .any(|l| l.locate(c).is_some())
        };
        for (name, words) in lists.chain(texts) {
            let chars: BTreeSet<char> =
                words.iter().flat_map(|w: &String| w.chars()).collect();
            let keyless: String =
                chars.into_iter().filter(|&c| !typeable(c)).collect();
            assert!(keyless.is_empty(), "{name}: {keyless:?}");
        }
    }

    #[test]
    fn user_texts_extend_the_bundled_ones() {
        let dir = std::env::temp_dir()
            .join(format!("oui-texts-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("french")).unwrap();
        let b = "[[Moi]]\ndeux\n\ntrois";
        fs::write(dir.join("french/b.txt"), b).unwrap();
        fs::write(dir.join("french/a.txt"), "un").unwrap();
        fs::write(dir.join("french/notes.md"), "ignored").unwrap();
        let bundled = classics("french").len();
        let all = load_passages("french", &dir).unwrap();
        assert_eq!(all.len(), bundled + 3);
        assert_eq!(
            all[bundled..],
            [
                uncited(&["un"]),
                cited(&["deux"], "Moi"),
                cited(&["trois"], "Moi")
            ]
        );
        assert_eq!(load_passages("german", &dir).unwrap().len(), 0);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn passages_start_on_a_passage_and_know_its_length() {
        let source = Source::passages(vec![
            uncited(&["a", "b"]),
            cited(&["c", "d", "e"], "Z"),
        ]);
        let mut rng = Rng::with_seed(5);
        for _ in 0..20 {
            let start = source.start(&mut rng);
            assert!(start == 0 || start == 2);
        }
        assert_eq!(
            (source.passage_len(0), source.passage_len(2)),
            (Some(2), Some(3))
        );
        assert_eq!(source.passage_len(1), None);
        assert_eq!((source.cite(0), source.cite(2)), (None, Some("Z")));
        assert_eq!(source.cite(1), None);
        assert_eq!(source.take(2, 4, &mut rng), owned(&["c", "d", "e", "a"]));
    }

    #[test]
    fn score_counts_inner_and_boundary_bigrams() {
        assert_eq!(score("aqua", &[('q', 'u')]), 1);
        assert_eq!(score("tête", &[(' ', 't'), ('e', ' ')]), 2);
        assert_eq!(score("abc", &[('x', 'y')]), 0);
    }

    #[test]
    fn practice_draws_only_words_holding_targets() {
        let pool = owned(&["quand", "que", "le", "de", "jardin"]);
        let mut rng = Rng::with_seed(3);
        let drill = practice(&pool, &[('q', 'u')], 40, &mut rng);
        assert_eq!(drill.len(), 40);
        assert!(drill.iter().all(|w| w.starts_with("qu")));
        assert!(drill.contains(&"quand".to_string()));
    }

    #[test]
    fn practice_falls_back_to_sampling() {
        let pool = owned(&["le", "de"]);
        let mut rng = Rng::with_seed(3);
        let drill = practice(&pool, &[('q', 'u')], 10, &mut rng);
        assert!(drill.iter().all(|w| pool.contains(w)));
        assert!(practice(&[], &[('q', 'u')], 3, &mut rng).is_empty());
    }

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
            assert!(words.len() >= 1000, "{name} has {} words", words.len());
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
