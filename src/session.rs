//! The typing state machine: target words, what was typed, and the keystroke
//! log every statistic is computed from.

use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Input {
    Char(char),
    Backspace,
    DeleteWord,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Keystroke {
    /// Time since the session's first keystroke.
    pub at: Duration,
    /// Target character at the cursor; `' '` once past the end of the word.
    pub expected: char,
    pub typed: char,
    /// Target character before the cursor; `None` at the very start.
    pub prev: Option<char>,
    /// The previous keystroke was a hit with no edit since, so the gap
    /// between the two is a clean inter-key latency.
    pub chained: bool,
}

impl Keystroke {
    pub fn hit(&self) -> bool {
        self.expected == self.typed
    }
}

#[derive(Debug, Clone)]
pub struct Session {
    words: Vec<String>,
    typed: Vec<String>,
    input: String,
    log: Vec<Keystroke>,
    limit: Option<Duration>,
    start: Option<Instant>,
    end: Option<Duration>,
    chain: bool,
}

impl Session {
    /// A session over `words`, ending at `limit` if given, else after the
    /// last word.
    pub fn new(words: Vec<String>, limit: Option<Duration>) -> Self {
        Self {
            words,
            typed: Vec::new(),
            input: String::new(),
            log: Vec::new(),
            limit,
            start: None,
            end: None,
            chain: false,
        }
    }

    pub fn words(&self) -> &[String] {
        &self.words
    }

    /// The submitted inputs, one per word before [`Session::current`].
    pub fn typed(&self) -> &[String] {
        &self.typed
    }

    pub fn input(&self) -> &str {
        &self.input
    }

    pub fn current(&self) -> usize {
        self.typed.len()
    }

    pub fn log(&self) -> &[Keystroke] {
        &self.log
    }

    pub fn limit(&self) -> Option<Duration> {
        self.limit
    }

    pub fn started(&self) -> bool {
        self.start.is_some()
    }

    pub fn finished(&self) -> bool {
        self.end.is_some()
    }

    /// Words not yet submitted, the current one included.
    pub fn remaining(&self) -> usize {
        self.words.len() - self.typed.len()
    }

    pub fn extend(&mut self, words: impl IntoIterator<Item = String>) {
        self.words.extend(words);
    }

    pub fn elapsed(&self, now: Instant) -> Duration {
        match (self.end, self.start) {
            (Some(end), _) => end,
            (None, Some(start)) => now.saturating_duration_since(start),
            (None, None) => Duration::ZERO,
        }
    }

    /// Characters of correctly typed words, counting the space that
    /// submitted each one.
    pub fn correct_chars(&self) -> usize {
        let last = self.words.len().saturating_sub(1);
        let correct = self.typed.iter().zip(&self.words).enumerate();
        correct
            .filter(|(_, (typed, word))| typed == word)
            .map(|(i, (_, word))| word.chars().count() + usize::from(i < last))
            .sum()
    }

    pub fn tick(&mut self, now: Instant) {
        let Some(limit) = self.limit else { return };
        if self.started() && !self.finished() && self.elapsed(now) >= limit {
            self.end = Some(limit);
        }
    }

    pub fn finish(&mut self, now: Instant) {
        if self.finished() {
            return;
        }
        let elapsed = self.elapsed(now);
        self.end = Some(self.limit.map_or(elapsed, |limit| elapsed.min(limit)));
    }

    pub fn apply(&mut self, input: Input, now: Instant) {
        self.tick(now);
        if self.finished() {
            return;
        }
        match input {
            Input::Char(' ') if self.input.is_empty() => {}
            Input::Char(c) => self.press(c, now),
            Input::Backspace => {
                self.chain = false;
                if self.input.pop().is_none() {
                    self.reopen();
                }
            }
            Input::DeleteWord => {
                self.chain = false;
                if self.input.is_empty() {
                    self.reopen();
                }
                self.input.clear();
            }
        }
    }

    fn press(&mut self, c: char, now: Instant) {
        let i = self.current();
        let Some(word) = self.words.get(i) else { return };
        let target: Vec<char> = word.chars().collect();
        let pos = self.input.chars().count().min(target.len());
        let prev = match pos {
            0 if i == 0 => None,
            0 => Some(' '),
            p => Some(target[p - 1]),
        };
        let start = *self.start.get_or_insert(now);
        let stroke = Keystroke {
            at: now.saturating_duration_since(start),
            expected: target.get(pos).copied().unwrap_or(' '),
            typed: c,
            prev,
            chained: self.chain,
        };
        self.chain = stroke.hit();
        self.log.push(stroke);

        if c == ' ' {
            return self.submit(now);
        }
        self.input.push(c);
        if i + 1 == self.words.len() && self.input == self.words[i] {
            self.submit(now);
        }
    }

    fn submit(&mut self, now: Instant) {
        self.typed.push(std::mem::take(&mut self.input));
        if self.remaining() == 0 {
            self.finish(now);
        }
    }

    /// Step back into the previous word, but only if it was typed wrong.
    fn reopen(&mut self) {
        let Some(i) = self.current().checked_sub(1) else { return };
        if self.typed[i] != self.words[i] {
            self.input = self.typed.pop().unwrap_or_default();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MS: Duration = Duration::from_millis(1);

    fn session(words: &[&str]) -> Session {
        Session::new(words.iter().map(|w| w.to_string()).collect(), None)
    }

    /// Types `keys` one every 100 ms; `<` is backspace, `^` delete-word.
    fn type_keys(s: &mut Session, t0: Instant, keys: &str) {
        for (i, c) in keys.chars().enumerate() {
            let input = match c {
                '<' => Input::Backspace,
                '^' => Input::DeleteWord,
                c => Input::Char(c),
            };
            s.apply(input, t0 + MS * 100 * i as u32);
        }
    }

    #[test]
    fn clean_run_finishes_on_the_last_character() {
        let mut s = session(&["ab", "cd"]);
        let t0 = Instant::now();
        type_keys(&mut s, t0, "ab cd");
        assert!(s.finished());
        assert_eq!(s.typed(), ["ab", "cd"]);
        assert_eq!(s.elapsed(t0 + Duration::from_secs(9)), MS * 400);
        assert_eq!(s.correct_chars(), 5);
        assert!(s.log().iter().all(Keystroke::hit));
    }

    #[test]
    fn keystrokes_carry_target_bigrams_and_chain() {
        let mut s = session(&["ab", "cd"]);
        type_keys(&mut s, Instant::now(), "ab c");
        let log = s.log();
        let pairs: Vec<_> = log.iter().map(|k| (k.prev, k.expected)).collect();
        assert_eq!(
            pairs,
            [(None, 'a'), (Some('a'), 'b'), (Some('b'), ' '), (Some(' '), 'c')]
        );
        assert_eq!(
            log.iter().map(|k| k.chained).collect::<Vec<_>>(),
            [false, true, true, true]
        );
    }

    #[test]
    fn a_miss_and_a_backspace_break_the_chain() {
        let mut s = session(&["abc"]);
        type_keys(&mut s, Instant::now(), "ax<bc");
        let log = s.log();
        assert_eq!((log[1].typed, log[1].hit()), ('x', false));
        assert!(!log[2].chained, "retyped b follows a backspace");
        assert!(log[3].chained);
        assert!(s.finished());
    }

    #[test]
    fn overflow_expects_a_space() {
        let mut s = session(&["ab", "cd"]);
        type_keys(&mut s, Instant::now(), "abx");
        let k = s.log()[2];
        assert_eq!((k.prev, k.expected, k.typed), (Some('b'), ' ', 'x'));
        assert_eq!(s.input(), "abx");
    }

    #[test]
    fn early_space_submits_a_wrong_word() {
        let mut s = session(&["abc", "d"]);
        type_keys(&mut s, Instant::now(), "ab ");
        assert_eq!(s.typed(), ["ab"]);
        assert_eq!(s.log()[2].expected, 'c');
        assert_eq!(s.correct_chars(), 0);
    }

    #[test]
    fn leading_space_is_ignored() {
        let mut s = session(&["a", "b"]);
        type_keys(&mut s, Instant::now(), "  a");
        assert_eq!(s.log().len(), 1);
        assert_eq!(s.input(), "a");
    }

    #[test]
    fn backspace_reopens_only_a_wrong_word() {
        let mut s = session(&["ab", "cd", "ef"]);
        type_keys(&mut s, Instant::now(), "ab cx <");
        assert_eq!((s.current(), s.input()), (1, "cx"));
        type_keys(&mut s, Instant::now(), "<<<<");
        assert_eq!((s.current(), s.input()), (1, ""), "ab was right");
    }

    #[test]
    fn delete_word_clears_input_and_can_reopen() {
        let mut s = session(&["ab", "cd"]);
        type_keys(&mut s, Instant::now(), "ax c^");
        assert_eq!((s.current(), s.input()), (1, ""));
        type_keys(&mut s, Instant::now(), "^");
        assert_eq!((s.current(), s.input()), (0, ""));
    }

    #[test]
    fn time_limit_ends_the_session_and_ignores_later_keys() {
        let words = vec!["a".to_string(); 10];
        let mut s = Session::new(words, Some(Duration::from_secs(1)));
        let t0 = Instant::now();
        s.apply(Input::Char('a'), t0);
        s.tick(t0 + MS * 999);
        assert!(!s.finished());
        s.apply(Input::Char(' '), t0 + MS * 1500);
        assert!(s.finished());
        assert_eq!(s.log().len(), 1);
        assert_eq!(s.elapsed(t0 + Duration::from_secs(5)), MS * 1000);
    }

    #[test]
    fn early_finish_is_capped_at_the_limit() {
        let mut s = Session::new(vec!["a".into()], Some(MS * 500));
        let t0 = Instant::now();
        s.apply(Input::Char('x'), t0);
        s.finish(t0 + MS * 800);
        assert_eq!(s.elapsed(t0), MS * 500);
    }

    #[test]
    fn clock_starts_on_the_first_keystroke() {
        let mut s = session(&["ab"]);
        let t0 = Instant::now();
        assert_eq!(s.elapsed(t0 + MS * 300), Duration::ZERO);
        s.apply(Input::Char('a'), t0 + MS * 300);
        assert_eq!(s.elapsed(t0 + MS * 450), MS * 150);
    }

    #[test]
    fn empty_session_ignores_input() {
        let mut s = session(&[]);
        s.apply(Input::Char('a'), Instant::now());
        assert!(s.log().is_empty());
    }
}
