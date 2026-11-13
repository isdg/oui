//! The reading history, `reads.jsonl` beside the tests' `history.jsonl`:
//! one JSON record per reading test, with the time each stretch of text
//! was shown and each probe's answer, so any figure can be recomputed.

use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::history::{Flags, INCOGNITO};

/// Bumped on any change old readers would misread; newer records are skipped.
pub const VERSION: u32 = 1;
/// The share of probes answered right from which a test's speed counts;
/// guessing reaches it on ten probes one time in eighteen.
pub const PASS: f64 = 0.8;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReadRecord {
    pub v: u32,
    /// When the test ended, RFC 3339 in local time.
    pub at: String,
    /// Language of a bundled work, else the file's name.
    pub source: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cite: Option<String>,
    /// The file read, by its canonical path, which a later test goes on in.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file: Option<String>,
    /// False for excerpts, which do not read as running prose does.
    pub continuous: bool,
    /// The first word read, among the `total` words of the text.
    pub start: usize,
    pub total: usize,
    pub words: usize,
    /// Characters read, a space after each word included.
    pub chars: usize,
    pub elapsed_ms: u64,
    pub wpm: f64,
    /// The speed the text slid by at, when it set the pace.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pace: Option<f64>,
    /// How a paced text was shown; sliding, if a paced record lacks it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub view: Option<View>,
    pub turns: Vec<Turn>,
    pub probes: Vec<Asked>,
    /// The terminal's size, as line length changes how fast one reads.
    pub cols: u16,
    pub rows: u16,
    #[serde(default, skip_serializing_if = "Flags::is_empty")]
    pub flags: Flags,
}

impl ReadRecord {
    pub fn incognito(&self) -> bool {
        self.flags.has(INCOGNITO)
    }

    pub fn elapsed(&self) -> Duration {
        Duration::from_millis(self.elapsed_ms)
    }

    /// Words as written a minute, as reading rates are published; `wpm`
    /// counts five characters a word, as typing does.
    pub fn words_a_minute(&self) -> f64 {
        match self.elapsed().as_secs_f64() / 60.0 {
            0.0 => 0.0,
            minutes => self.words as f64 / minutes,
        }
    }

    /// Sentences read that were known again, among those asked.
    pub fn hits(&self) -> (usize, usize) {
        self.told(true)
    }

    /// Sentences never read that were taken for read, among those asked.
    pub fn false_alarms(&self) -> (usize, usize) {
        self.told(false)
    }

    /// The probes answered "read" among those that were `old`.
    fn told(&self, old: bool) -> (usize, usize) {
        let asked = self.probes.iter().filter(|p| p.1 == old);
        (asked.clone().filter(|p| p.2).count(), asked.count())
    }

    /// Probes answered right, among them all.
    pub fn right(&self) -> (usize, usize) {
        let right = self.probes.iter().filter(|p| p.1 == p.2).count();
        (right, self.probes.len())
    }

    /// Whether the text was taken in well enough for its speed to count.
    pub fn counts(&self) -> bool {
        let (right, asked) = self.right();
        asked > 0 && right as f64 >= PASS * asked as f64
    }
}

/// How a paced text is shown.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize,
)]
#[serde(rename_all = "lowercase")]
pub enum View {
    /// Through a stripe, a character at a time.
    #[default]
    Slide,
    /// A word at a time in one place, the eyes at rest.
    Blink,
}

/// A stretch of text shown, as `[first word, word after its last, ms]`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Turn(pub usize, pub usize, pub u64);

/// A probe as `[sentence, was read, answered read, ms to answer]`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Asked(pub String, pub bool, pub bool, pub u64);

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) fn record(at: &str) -> ReadRecord {
        ReadRecord {
            v: VERSION,
            at: at.to_string(),
            source: "english".into(),
            cite: None,
            file: None,
            continuous: false,
            start: 0,
            total: 900,
            words: 3,
            chars: 14,
            elapsed_ms: 800,
            wpm: 210.0,
            pace: None,
            view: None,
            turns: vec![Turn(0, 2, 500), Turn(2, 3, 300)],
            probes: vec![Asked("It rained.".into(), true, false, 1200)],
            cols: 80,
            rows: 24,
            flags: Flags::default(),
        }
    }

    #[test]
    fn turns_and_probes_serialise_as_compact_arrays() {
        let json = serde_json::to_string(&record("t")).unwrap();
        assert!(json.contains(r#""turns":[[0,2,500],[2,3,300]]"#), "{json}");
        assert!(
            json.contains(r#""probes":[["It rained.",true,false,1200]]"#),
            "{json}"
        );
    }

    #[test]
    fn answers_are_scored_against_what_was_read() {
        let asked = |old, answer| Asked(String::new(), old, answer, 0);
        let probes = vec![
            asked(true, true),
            asked(true, false),
            asked(true, true),
            asked(false, false),
            asked(false, true),
        ];
        let r = ReadRecord { probes, ..record("t") };
        assert_eq!((r.hits(), r.false_alarms()), ((2, 3), (1, 2)));
        assert_eq!(r.right(), (3, 5));
        assert!(!r.counts(), "three in five is under the pass");
        let four = vec![asked(true, true); 4];
        let passed = [four, vec![asked(false, true)]].concat();
        assert!(ReadRecord { probes: passed, ..record("t") }.counts());
        assert!(!ReadRecord { probes: vec![], ..record("t") }.counts());
    }

    #[test]
    fn real_words_a_minute_differ_from_five_character_ones() {
        let r = ReadRecord { words: 4, elapsed_ms: 1000, ..record("t") };
        assert_eq!(r.words_a_minute(), 240.0);
        let unread = ReadRecord { elapsed_ms: 0, ..record("t") };
        assert_eq!(unread.words_a_minute(), 0.0);
    }

    #[test]
    fn optional_fields_are_stored_only_when_present() {
        let bare = record("t");
        let json = serde_json::to_string(&bare).unwrap();
        for field in ["cite", "file", "pace", "view"] {
            assert!(!json.contains(field), "{field} in {json}");
        }
        assert_eq!(serde_json::from_str::<ReadRecord>(&json).unwrap(), bare);
        let full = ReadRecord {
            cite: Some("Austen, Emma (1815)".into()),
            file: Some("/books/emma.txt".into()),
            pace: Some(300.0),
            continuous: true,
            ..bare
        };
        let json = serde_json::to_string(&full).unwrap();
        assert_eq!(serde_json::from_str::<ReadRecord>(&json).unwrap(), full);
    }
}
