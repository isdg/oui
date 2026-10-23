//! Everything the results screen shows, computed from a session's keystroke
//! log: speed, accuracy, a timeline, and per-key and per-bigram tallies.

use std::collections::BTreeMap;
use std::ops::AddAssign;
use std::time::Duration;

use crate::session::{self, Keystroke, Session};

/// Longer gaps are pauses, not typing, and would swamp the latency means.
pub const PAUSE: Duration = Duration::from_secs(2);
/// Width of the trailing window the WPM timeline averages over.
pub const WINDOW: Duration = Duration::from_secs(5);
/// Fewer timed samples than this make a bigram's mean latency noise.
pub const MIN_TIMED: u32 = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Tally {
    pub hits: u32,
    pub misses: u32,
    /// Hits with a clean latency, and the sum of those latencies.
    pub timed: u32,
    pub latency: Duration,
}

impl Tally {
    pub fn attempts(&self) -> u32 {
        self.hits + self.misses
    }

    pub fn miss_rate(&self) -> f64 {
        match self.attempts() {
            0 => 0.0,
            n => f64::from(self.misses) / f64::from(n),
        }
    }

    pub fn mean_latency(&self) -> Option<Duration> {
        (self.timed > 0).then(|| self.latency / self.timed)
    }

    fn record(&mut self, hit: bool, latency: Option<Duration>) {
        if !hit {
            self.misses += 1;
            return;
        }
        self.hits += 1;
        if let Some(latency) = latency {
            self.timed += 1;
            self.latency += latency;
        }
    }
}

impl AddAssign for Tally {
    fn add_assign(&mut self, other: Self) {
        self.hits += other.hits;
        self.misses += other.misses;
        self.timed += other.timed;
        self.latency += other.latency;
    }
}

#[derive(Debug, Clone, Default)]
pub struct Report {
    pub elapsed: Duration,
    pub wpm: f64,
    pub raw_wpm: f64,
    pub accuracy: f64,
    pub hits: usize,
    pub keystrokes: usize,
    pub words: usize,
    pub correct_words: usize,
    /// WPM over the trailing [`WINDOW`], sampled once per second.
    pub timeline: Vec<f64>,
    /// Keyed by the target character.
    pub keys: BTreeMap<char, Tally>,
    /// Keyed by (previous, target) character; `' '` marks a word boundary.
    pub bigrams: BTreeMap<(char, char), Tally>,
    /// Miss counts keyed by (target, typed) character.
    pub typos: BTreeMap<(char, char), u32>,
}

impl Report {
    pub fn new(session: &Session, elapsed: Duration) -> Self {
        let (words, typed) = (session.words(), session.typed());
        Report::from_parts(session.log(), words, typed, elapsed)
    }

    /// A report over a session's parts, as a history record stores them.
    pub fn from_parts(
        log: &[Keystroke],
        words: &[String],
        typed: &[String],
        elapsed: Duration,
    ) -> Self {
        let hits = log.iter().filter(|k| k.hit()).count();
        let pairs = typed.iter().zip(words);
        let mut report = Report {
            elapsed,
            wpm: wpm(session::correct_chars(words, typed), elapsed),
            raw_wpm: wpm(log.len(), elapsed),
            accuracy: accuracy(log),
            hits,
            keystrokes: log.len(),
            words: typed.len(),
            correct_words: pairs.filter(|(typed, word)| typed == word).count(),
            timeline: timeline(log, elapsed),
            ..Report::default()
        };
        for (i, k) in log.iter().enumerate() {
            let latency = latency(log, i);
            report.keys.entry(k.expected).or_default().record(k.hit(), latency);
            if let Some(prev) = k.prev {
                let bigram = report.bigrams.entry((prev, k.expected));
                bigram.or_default().record(k.hit(), latency);
            }
            if !k.hit() {
                *report.typos.entry((k.expected, k.typed)).or_default() += 1;
            }
        }
        report
    }

    /// Keys with at least one miss, by miss rate then miss count.
    pub fn worst_keys(&self, n: usize) -> Vec<(char, Tally)> {
        let mut keys: Vec<_> =
            self.keys.iter().map(|(&c, &t)| (c, t)).collect();
        keys.retain(|(_, t)| t.misses > 0);
        keys.sort_by(|(_, a), (_, b)| {
            b.miss_rate()
                .total_cmp(&a.miss_rate())
                .then(b.misses.cmp(&a.misses))
        });
        keys.truncate(n);
        keys
    }

    /// Bigrams with at least one miss, by miss count then miss rate.
    pub fn error_bigrams(&self, n: usize) -> Vec<((char, char), Tally)> {
        let mut bigrams = self.bigram_list();
        bigrams.retain(|(_, t)| t.misses > 0);
        bigrams.sort_by(|(_, a), (_, b)| {
            b.misses
                .cmp(&a.misses)
                .then(b.miss_rate().total_cmp(&a.miss_rate()))
        });
        bigrams.truncate(n);
        bigrams
    }

    /// Bigrams with at least [`MIN_TIMED`] samples, slowest mean first.
    pub fn slow_bigrams(&self, n: usize) -> Vec<((char, char), Duration)> {
        let bigrams = self.bigram_list().into_iter();
        let mut slow: Vec<_> = bigrams
            .filter(|(_, t)| t.timed >= MIN_TIMED)
            .filter_map(|(b, t)| Some((b, t.mean_latency()?)))
            .collect();
        slow.sort_by(|(_, a), (_, b)| b.cmp(a));
        slow.truncate(n);
        slow
    }

    /// Most frequent (target, typed) confusions.
    pub fn top_typos(&self, n: usize) -> Vec<((char, char), u32)> {
        let mut typos: Vec<_> =
            self.typos.iter().map(|(&p, &c)| (p, c)).collect();
        typos.sort_by(|(_, a), (_, b)| b.cmp(a));
        typos.truncate(n);
        typos
    }

    /// The bigrams worth drilling: the `n` most missed, then the `n`
    /// slowest not already listed.
    pub fn weak_bigrams(&self, n: usize) -> Vec<(char, char)> {
        let mut weak: Vec<_> =
            self.error_bigrams(n).into_iter().map(|(b, _)| b).collect();
        for (b, _) in self.slow_bigrams(n) {
            if !weak.contains(&b) {
                weak.push(b);
            }
        }
        weak
    }

    fn bigram_list(&self) -> Vec<((char, char), Tally)> {
        self.bigrams.iter().map(|(&b, &t)| (b, t)).collect()
    }
}

/// Several reports as one: counts and tallies summed, speeds averaged per
/// report, accuracy over all keystrokes. The timeline is left empty.
pub fn combine<'a>(reports: impl IntoIterator<Item = &'a Report>) -> Report {
    let mut total = Report::default();
    let mut n = 0.0;
    for r in reports {
        n += 1.0;
        total.elapsed += r.elapsed;
        total.wpm += r.wpm;
        total.raw_wpm += r.raw_wpm;
        total.hits += r.hits;
        total.keystrokes += r.keystrokes;
        total.words += r.words;
        total.correct_words += r.correct_words;
        for (&c, &t) in &r.keys {
            *total.keys.entry(c).or_default() += t;
        }
        for (&b, &t) in &r.bigrams {
            *total.bigrams.entry(b).or_default() += t;
        }
        for (&p, &count) in &r.typos {
            *total.typos.entry(p).or_default() += count;
        }
    }
    if n > 0.0 {
        total.wpm /= n;
        total.raw_wpm /= n;
    }
    total.accuracy = match total.keystrokes {
        0 => 1.0,
        k => total.hits as f64 / k as f64,
    };
    total
}

pub fn wpm(chars: usize, elapsed: Duration) -> f64 {
    let minutes = elapsed.as_secs_f64() / 60.0;
    if minutes == 0.0 {
        return 0.0;
    }
    chars as f64 / 5.0 / minutes
}

pub fn accuracy(log: &[Keystroke]) -> f64 {
    if log.is_empty() {
        return 1.0;
    }
    log.iter().filter(|k| k.hit()).count() as f64 / log.len() as f64
}

/// The clean latency of keystroke `i`, if it has one.
fn latency(log: &[Keystroke], i: usize) -> Option<Duration> {
    let k = log[i];
    let prev = log.get(i.checked_sub(1)?)?;
    let gap = k.at.saturating_sub(prev.at);
    (k.chained && k.hit() && gap <= PAUSE).then_some(gap)
}

fn timeline(log: &[Keystroke], elapsed: Duration) -> Vec<f64> {
    let secs = elapsed.as_secs_f64().ceil() as u64;
    let sample = |s: u64| {
        let end = Duration::from_secs(s).min(elapsed);
        let start = end.saturating_sub(WINDOW);
        let inside =
            |k: &&Keystroke| k.at >= start && (k.at < end || end == elapsed);
        let hits = log.iter().filter(|k| k.hit()).filter(inside).count();
        wpm(hits, end - start)
    };
    (1..=secs).map(sample).collect()
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use super::*;
    use crate::session::Input;

    const MS: Duration = Duration::from_millis(1);

    /// Plays `(key, ms since start)` pairs; `<` is backspace.
    fn play(words: &[&str], keys: &[(char, u32)]) -> Report {
        let words = words.iter().map(|w| w.to_string()).collect();
        let mut s = Session::new(words, None);
        let t0 = Instant::now();
        for &(c, ms) in keys {
            let input =
                if c == '<' { Input::Backspace } else { Input::Char(c) };
            s.apply(input, t0 + MS * ms);
        }
        s.finish(t0 + MS * keys.last().map_or(0, |&(_, ms)| ms));
        Report::new(&s, s.elapsed(t0))
    }

    fn steady(text: &str, gap: u32) -> Vec<(char, u32)> {
        text.chars().zip((0..).map(|i| i * gap)).collect()
    }

    #[test]
    fn wpm_is_five_chars_per_word_per_minute() {
        assert_eq!(wpm(50, Duration::from_secs(60)), 10.0);
        assert_eq!(wpm(10, Duration::ZERO), 0.0);
    }

    #[test]
    fn clean_run_scores_every_word() {
        let r = play(&["ab", "cd"], &steady("ab cd", 150));
        assert_eq!((r.words, r.correct_words), (2, 2));
        assert_eq!((r.hits, r.keystrokes), (5, 5));
        assert_eq!(r.accuracy, 1.0);
        assert_eq!(r.elapsed, MS * 600);
        assert_eq!(r.wpm, r.raw_wpm);
        assert!(r.worst_keys(5).is_empty() && r.top_typos(5).is_empty());
    }

    #[test]
    fn corrected_errors_still_cost_accuracy() {
        let keys = [('a', 0), ('x', 100), ('<', 200), ('b', 300)];
        let r = play(&["ab"], &keys);
        assert_eq!((r.hits, r.keystrokes), (2, 3));
        assert_eq!(r.correct_words, 1);
        assert_eq!(
            r.keys[&'b'],
            Tally { hits: 1, misses: 1, ..Tally::default() }
        );
        assert_eq!(r.bigrams[&('a', 'b')].misses, 1);
        assert_eq!(r.top_typos(5), [(('b', 'x'), 1)]);
    }

    #[test]
    fn latency_counts_only_chained_hits_under_the_pause() {
        let keys = [('a', 0), ('b', 200), ('c', 3000), ('d', 3100)];
        let r = play(&["abcd"], &keys);
        assert_eq!(r.keys[&'a'].timed, 0, "first key has no predecessor");
        assert_eq!(r.keys[&'b'].mean_latency(), Some(MS * 200));
        assert_eq!(r.keys[&'c'].timed, 0, "2.8 s is a pause");
        assert_eq!(r.bigrams[&('c', 'd')].mean_latency(), Some(MS * 100));
    }

    #[test]
    fn word_boundaries_form_bigrams() {
        let r = play(&["ab", "cd"], &steady("ab cd", 100));
        assert!(r.bigrams.contains_key(&('b', ' ')));
        assert!(r.bigrams.contains_key(&(' ', 'c')));
        assert!(!r.bigrams.keys().any(|&(p, _)| p == 'd'));
    }

    #[test]
    fn rankings_order_and_truncate() {
        // e is missed 1 of 1 times, o 1 of 2, so e ranks first.
        let r = play(&["e", "oo", "z"], &steady("x ox z", 100));
        let worst: Vec<char> =
            r.worst_keys(5).iter().map(|(c, _)| *c).collect();
        assert_eq!(worst, ['e', 'o']);
        assert_eq!(r.worst_keys(1).len(), 1);
    }

    #[test]
    fn slow_bigrams_need_two_samples() {
        let keys = [('a', 0), ('b', 100), ('a', 400), ('b', 500), ('c', 1500)];
        let r = play(&["ababc"], &keys);
        assert_eq!(r.slow_bigrams(5), [(('a', 'b'), MS * 100)]);
    }

    #[test]
    fn combine_sums_tallies_and_averages_speed() {
        let a = play(&["ab"], &steady("ab", 100));
        let b = play(&["ab"], &[('a', 0), ('x', 100), ('<', 150), ('b', 200)]);
        let all = combine([&a, &b]);
        assert_eq!(all.wpm, (a.wpm + b.wpm) / 2.0);
        assert_eq!((all.hits, all.keystrokes), (4, 5));
        assert_eq!(all.accuracy, 0.8);
        assert_eq!((all.keys[&'b'].hits, all.keys[&'b'].misses), (2, 1));
        assert_eq!(all.typos[&('b', 'x')], 1);
        assert_eq!(combine([]).accuracy, 1.0);
    }

    #[test]
    fn weak_bigrams_merge_misses_and_slowness_without_repeats() {
        let slow =
            |misses| Tally { hits: 2, misses, timed: 2, latency: MS * 600 };
        let bigrams =
            BTreeMap::from([(('q', 'u'), slow(1)), (('r', 'é'), slow(0))]);
        let r = Report { bigrams, ..Report::default() };
        assert_eq!(r.weak_bigrams(5), [('q', 'u'), ('r', 'é')]);
    }

    #[test]
    fn timeline_samples_each_second_over_a_trailing_window() {
        let r = play(&["abcdefghij"], &steady("abcdefghij", 250));
        assert_eq!(r.timeline.len(), 3);
        assert_eq!(r.timeline[0], wpm(4, Duration::from_secs(1)));
        assert_eq!(r.timeline[2], wpm(10, MS * 2250));
    }
}
