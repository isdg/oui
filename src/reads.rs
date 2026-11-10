//! The reading history, `reads.jsonl` beside the tests' `history.jsonl`:
//! one JSON record per reading test, with the time each stretch of text
//! was shown and each probe's answer, so any figure can be recomputed.

use serde::{Deserialize, Serialize};

/// Bumped on any change old readers would misread; newer records are skipped.
pub const VERSION: u32 = 1;

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
    pub turns: Vec<Turn>,
    pub probes: Vec<Asked>,
    /// The terminal's size, as line length changes how fast one reads.
    pub cols: u16,
    pub rows: u16,
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
            turns: vec![Turn(0, 2, 500), Turn(2, 3, 300)],
            probes: vec![Asked("It rained.".into(), true, false, 1200)],
            cols: 80,
            rows: 24,
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
    fn optional_fields_are_stored_only_when_present() {
        let bare = record("t");
        let json = serde_json::to_string(&bare).unwrap();
        for field in ["cite", "file", "pace"] {
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
