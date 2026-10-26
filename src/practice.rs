//! Practice: the words densest in the bigrams that cost a language's
//! recorded tests the most time.

use std::cmp::Reverse;
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::time::Duration;

use anyhow::Result;
use clap::ValueEnum;

use crate::fingering::{self, Fingering, Mark};
use crate::history::Record;
use crate::keyboard::Layout;
use crate::session::Keystroke;
use crate::stats::{self, Report, Tally};
use crate::words;

/// Costliest bigrams practised, beside any behind a fingering mark.
const TARGETS: usize = 16;
/// Keystrokes a miss costs: the wrong key, a backspace and the retype.
const MISS: u32 = 3;

/// A bigram worth practising and the time it costs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Target {
    pub bigram: (char, char),
    pub tally: Tally,
    pub cost: Duration,
}

/// The words practice draws on: the bundled list, else the lower-case
/// words of the language's passages, in order of first use.
pub fn vocabulary(language: &str, texts: &Path) -> Result<Vec<String>> {
    if let Some(list) = words::language(language) {
        return Ok(list);
    }
    let passages = words::load_passages(language, texts)?;
    let mut seen = BTreeSet::new();
    let bare = |w: &String| {
        let w = w.trim_matches(|c: char| !c.is_alphabetic());
        let lower = w.chars().all(|c| c.is_lowercase() || "'-".contains(c));
        (!w.is_empty() && lower).then(|| w.to_string())
    };
    let all = passages.iter().flat_map(|p| &p.words).filter_map(bare);
    Ok(all.filter(|w| seen.insert(w.clone())).collect())
}

/// The bigrams words of `vocabulary` hold, costliest first: the [`TARGETS`]
/// costliest and any behind fingering marks, each costing
/// Σ(gap − baseline) over its clean hits + misses × [`MISS`] × baseline.
pub fn targets(records: &[Record], vocabulary: &[String]) -> Vec<Target> {
    let reports: Vec<Report> = records.iter().map(Record::report).collect();
    let logs: Vec<Vec<Keystroke>> = records.iter().map(Record::log).collect();
    let (start, rest) = baselines(&logs);
    let held: BTreeSet<(char, char)> =
        vocabulary.iter().flat_map(|w| pairs(w)).collect();
    let cost = |(a, _): (char, char), t: &Tally| {
        let base = if a == ' ' { start } else { rest };
        t.latency.saturating_sub(base * t.timed) + base * MISS * t.misses
    };
    let mut ranked: Vec<Target> = stats::combine(&reports)
        .bigrams
        .into_iter()
        .filter(|(b, _)| held.contains(b))
        .map(|(bigram, tally)| Target {
            bigram,
            tally,
            cost: cost(bigram, &tally),
        })
        .filter(|t| !t.cost.is_zero())
        .collect();
    ranked.sort_by_key(|t| Reverse(t.cost));
    pick(ranked, &marked(records, &logs))
}

/// The `n` words of `vocabulary` with the most target cost per keystroke,
/// the space after them included; ties keep the vocabulary's order.
pub fn words(
    vocabulary: &[String],
    targets: &[Target],
    n: usize,
) -> Vec<String> {
    let costs: BTreeMap<(char, char), Duration> =
        targets.iter().map(|t| (t.bigram, t.cost)).collect();
    let mut scored: Vec<(Duration, &String)> = vocabulary
        .iter()
        .filter_map(|w| {
            let pairs = pairs(w);
            let cost: Duration =
                pairs.iter().filter_map(|p| costs.get(p)).sum();
            (!cost.is_zero()).then(|| (cost / pairs.len() as u32, w))
        })
        .collect();
    scored.sort_by_key(|&(cost, _)| Reverse(cost));
    scored.into_iter().take(n).map(|(_, w)| w.clone()).collect()
}

fn pick(ranked: Vec<Target>, marked: &BTreeSet<(char, char)>) -> Vec<Target> {
    let keep =
        |&(i, t): &(usize, Target)| i < TARGETS || marked.contains(&t.bigram);
    ranked.into_iter().enumerate().filter(keep).map(|(_, t)| t).collect()
}

/// The baselines: the median clean gap starting a word, and that of any
/// other bigram.
fn baselines(logs: &[Vec<Keystroke>]) -> (Duration, Duration) {
    let (mut start, mut rest) = (Vec::new(), Vec::new());
    for log in logs {
        for (i, k) in log.iter().enumerate() {
            let Some(gap) = stats::latency(log, i) else { continue };
            match k.prev {
                Some(' ') => start.push(gap),
                _ => rest.push(gap),
            }
        }
    }
    (fingering::median(start), fingering::median(rest))
}

/// Both orders of the pairs behind each key's moved or as-one mark, per
/// layout the records were typed on.
fn marked(
    records: &[Record],
    logs: &[Vec<Keystroke>],
) -> BTreeSet<(char, char)> {
    let mut out = BTreeSet::new();
    for &layout in Layout::value_variants() {
        let on = records.iter().zip(logs);
        let logs = on
            .filter(|(r, _)| r.meta.layout == layout.name())
            .map(|(_, log)| log.as_slice());
        let Some(fingering) = Fingering::infer(layout, logs) else {
            continue;
        };
        let marks = fingering.marks.into_values();
        let evidence =
            marks.filter(|m| m.mark != Mark::Unsure).flat_map(|m| m.evidence);
        for pair in evidence {
            let a = layout.char_at(pair.keys.0);
            let (Some(a), Some(b)) = (a, layout.char_at(pair.keys.1)) else {
                continue;
            };
            out.extend([(a, b), (b, a)]);
        }
    }
    out
}

/// The bigrams typing `word` takes, from the space before it to the one
/// after.
fn pairs(word: &str) -> Vec<(char, char)> {
    let padded: Vec<char> =
        std::iter::once(' ').chain(word.chars()).chain([' ']).collect();
    padded.windows(2).map(|p| (p[0], p[1])).collect()
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;
    use crate::history::{Meta, Stroke, VERSION};

    const MS: Duration = Duration::from_millis(1);

    /// Each `(bigram, ms, n)` typed `n` times, a clean gap of `ms` between
    /// its two characters and a pause before the next; `ms` 0 misses it.
    fn log(bigrams: &[(&str, u32, usize)]) -> Vec<Keystroke> {
        let mut log = Vec::new();
        let mut at = Duration::ZERO;
        for &(bigram, ms, n) in bigrams {
            let mut chars = bigram.chars();
            let (a, b) = (chars.next().unwrap(), chars.next().unwrap());
            for _ in 0..n {
                at += stats::PAUSE * 2;
                let stroke = |at, c, typed, prev, chained| Keystroke {
                    at,
                    expected: c,
                    typed,
                    prev,
                    chained,
                };
                let typed = if ms == 0 { '#' } else { b };
                log.push(stroke(at, a, a, None, false));
                log.push(stroke(at + MS * ms.max(1), b, typed, Some(a), true));
            }
        }
        log
    }

    fn record(layout: Layout, log: &[Keystroke]) -> Record {
        let meta = Meta {
            source: "english".into(),
            mode: "words".into(),
            layout: layout.name().into(),
            ..Meta::default()
        };
        Record {
            v: VERSION,
            at: "2026-10-26T10:00:00+01:00".into(),
            meta,
            completed: true,
            elapsed_ms: log.last().map_or(0, |k| k.at.as_millis() as u64),
            wpm: 0.0,
            raw_wpm: 0.0,
            accuracy: 1.0,
            words: Vec::new(),
            typed: Vec::new(),
            keys: log.iter().map(Stroke::from).collect(),
        }
    }

    fn owned(words: &[&str]) -> Vec<String> {
        words.iter().map(|w| w.to_string()).collect()
    }

    fn costs(targets: &[Target]) -> Vec<(String, u128)> {
        let name = |t: &Target| format!("{}{}", t.bigram.0, t.bigram.1);
        targets.iter().map(|t| (name(t), t.cost.as_millis())).collect()
    }

    #[test]
    fn cost_is_time_over_the_baseline_and_misses() {
        let log = log(&[
            ("ab", 100, 10),
            ("cd", 100, 10),
            ("qu", 400, 10),
            ("th", 0, 1),
            (" t", 200, 10),
            (" q", 400, 10),
        ]);
        let vocabulary = owned(&["ab", "cd", "the", "quiz", "q"]);
        let targets = targets(&[record(Layout::Qwerty, &log)], &vocabulary);
        assert_eq!(
            costs(&targets),
            [("qu".into(), 3000), (" q".into(), 1000), ("th".into(), 300)],
            "a word start counts against the usual word start"
        );
        assert_eq!(targets[0].tally.timed, 10);
    }

    #[test]
    fn bigrams_no_word_holds_are_no_target() {
        let log = log(&[("ab", 100, 10), ("x,", 900, 10)]);
        let targets = targets(&[record(Layout::Qwerty, &log)], &owned(&["x"]));
        assert!(targets.is_empty(), "{:?}", costs(&targets));
    }

    #[test]
    fn words_rank_by_cost_per_keystroke_then_list_order() {
        let target = |bigram, ms| Target {
            bigram,
            tally: Tally::default(),
            cost: MS * ms,
        };
        let targets = [target(('q', 'u'), 3000), target(('t', 'h'), 300)];
        let vocabulary =
            owned(&["the", "and", "quick", "queue", "aqua", "then"]);
        let drill = words(&vocabulary, &targets, 9);
        assert_eq!(drill, ["aqua", "quick", "queue", "the", "then"]);
        assert_eq!(words(&vocabulary, &targets, 2), ["aqua", "quick"]);
        assert_eq!(words(&vocabulary, &targets, 9), drill, "deterministic");
    }

    #[test]
    fn fingering_marks_add_their_pairs_past_the_costliest() {
        let steady = [
            ("13", 120, 300),
            ("25", 120, 300),
            ("47", 120, 300),
            ("80", 120, 300),
        ];
        let moved =
            [("mb", 234, 17), ("ub", 209, 17), ("bu", 200, 3), ("be", 100, 52)];
        let log = log(&[&steady[..], &moved].concat());
        let records = [record(Layout::Qwerty, &log)];
        let logs = [log];
        let marked = marked(&records, &logs);
        let want = [('b', 'm'), ('b', 'u'), ('m', 'b'), ('u', 'b')];
        assert_eq!(marked.into_iter().collect::<Vec<_>>(), want);

        let target = |i: u32| Target {
            bigram: (char::from_u32(0x61 + i).unwrap(), 'z'),
            tally: Tally::default(),
            cost: MS * (100 - i),
        };
        let ranked: Vec<Target> = (0..20).map(target).collect();
        let marked = BTreeSet::from([('s', 'z'), ('a', 'z')]);
        let picked = pick(ranked.clone(), &marked);
        assert_eq!(picked.len(), TARGETS + 1);
        assert_eq!(picked[..TARGETS], ranked[..TARGETS]);
        assert_eq!(picked[TARGETS].bigram, ('s', 'z'));
    }

    #[test]
    fn vocabulary_falls_back_to_the_passages() {
        let dir = std::env::temp_dir()
            .join(format!("oui-practice-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("german")).unwrap();
        let text = "Der Hund, der «bellt»: aujourd'hui der 42 Hund.";
        fs::write(dir.join("german/a.txt"), text).unwrap();
        let german = vocabulary("german", &dir).unwrap();
        assert_eq!(german, ["der", "bellt", "aujourd'hui"]);
        assert_eq!(vocabulary("english", &dir).unwrap().len(), 5000);
        assert!(vocabulary("klingon", &dir).unwrap().is_empty());
        fs::remove_dir_all(&dir).unwrap();
    }
}
