//! The test history, `history.jsonl` under `~/.oui` (or `$OUI_HOME`): one
//! JSON record per test, appended as it ends, keeping the raw keystrokes so
//! any statistic can be recomputed over old tests.

use std::collections::BTreeSet;
use std::env;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::reads::{self, ReadRecord};
use crate::session::{Keystroke, Session};
use crate::stats::Report;

/// Bumped on any change old readers would misread; newer records are skipped.
pub const VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Record {
    pub v: u32,
    /// When the test ended, RFC 3339 in local time.
    pub at: String,
    #[serde(flatten)]
    pub meta: Meta,
    /// False when the test was cut short with esc.
    pub completed: bool,
    pub elapsed_ms: u64,
    pub wpm: f64,
    pub raw_wpm: f64,
    pub accuracy: f64,
    /// Target words up to the one the test ended on.
    pub words: Vec<String>,
    pub typed: Vec<String>,
    pub keys: Vec<Stroke>,
    #[serde(default, skip_serializing_if = "Flags::is_empty")]
    pub flags: Flags,
}

/// Marks a record carries beyond what the test was, such as [`INCOGNITO`];
/// plain strings, so one this build does not know survives a rewrite.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Flags(BTreeSet<String>);

/// Kept out of `oui stats` unless asked for, yet a test like any other.
pub const INCOGNITO: &str = "incognito";

impl Flags {
    pub fn has(&self, flag: &str) -> bool {
        self.0.contains(flag)
    }

    pub fn set(&mut self, flag: &str, on: bool) {
        match on {
            true => self.0.insert(flag.to_owned()),
            false => self.0.remove(flag),
        };
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

/// What a test was, beyond its keystrokes; plain strings so the file
/// outlives any change to the CLI's enums.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Meta {
    /// Language or file name, as in the screen title.
    pub source: String,
    pub mode: String,
    #[serde(default)]
    pub with: Vec<String>,
    /// Prose typed `--plain`: lowercase, bare of punctuation and numbers.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub plain: bool,
    /// A drill on weak bigrams, as `p` ran before it started practice;
    /// read from the records of then, never written.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub drill: bool,
    pub layout: String,
    /// Where a text-mode passage is from, when its text cites it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cite: Option<String>,
}

/// A keystroke as `[ms, expected, typed, prev, chained]`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Stroke(pub u64, pub char, pub char, pub Option<char>, pub bool);

impl From<&Keystroke> for Stroke {
    fn from(k: &Keystroke) -> Self {
        let ms = u64::try_from(k.at.as_millis()).unwrap_or(u64::MAX);
        Stroke(ms, k.expected, k.typed, k.prev, k.chained)
    }
}

impl From<&Stroke> for Keystroke {
    fn from(&Stroke(ms, expected, typed, prev, chained): &Stroke) -> Self {
        let at = Duration::from_millis(ms);
        Keystroke { at, expected, typed, prev, chained }
    }
}

impl Record {
    pub fn new(
        session: &Session,
        now: Instant,
        at: String,
        meta: Meta,
    ) -> Self {
        let elapsed = session.elapsed(now);
        let report = Report::new(session, elapsed);
        let reached = (session.current() + 1).min(session.words().len());
        Record {
            v: VERSION,
            at,
            meta,
            completed: session.completed(),
            elapsed_ms: u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX),
            wpm: report.wpm,
            raw_wpm: report.raw_wpm,
            accuracy: report.accuracy,
            words: session.words()[..reached].to_vec(),
            typed: session.typed().to_vec(),
            keys: session.log().iter().map(Stroke::from).collect(),
            flags: Flags::default(),
        }
    }

    pub fn incognito(&self) -> bool {
        self.flags.has(INCOGNITO)
    }

    pub fn log(&self) -> Vec<Keystroke> {
        self.keys.iter().map(Keystroke::from).collect()
    }

    /// The full report, recomputed from the stored keystrokes.
    pub fn report(&self) -> Report {
        let log = self.log();
        let elapsed = Duration::from_millis(self.elapsed_ms);
        Report::from_parts(&log, &self.words, &self.typed, elapsed)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Store {
    dir: PathBuf,
}

/// Records read back, and how many lines were unreadable or too new.
#[derive(Debug, Clone)]
pub struct History<T = Record> {
    pub records: Vec<T>,
    pub skipped: usize,
}

impl<T> Default for History<T> {
    fn default() -> Self {
        History { records: Vec::new(), skipped: 0 }
    }
}

impl Store {
    /// `$OUI_HOME`, else `~/.oui`.
    pub fn open() -> Result<Self> {
        if let Some(dir) = env::var_os("OUI_HOME") {
            return Ok(Store::at(dir));
        }
        let home =
            env::var_os("HOME").context("neither OUI_HOME nor HOME set")?;
        Ok(Store::at(Path::new(&home).join(".oui")))
    }

    pub fn at(dir: impl Into<PathBuf>) -> Self {
        Store { dir: dir.into() }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    pub fn history(&self) -> PathBuf {
        self.dir.join("history.jsonl")
    }

    /// The reading tests, beside the history.
    pub fn reads(&self) -> PathBuf {
        self.dir.join("reads.jsonl")
    }

    pub fn append(&self, record: &Record) -> Result<()> {
        append(&self.history(), record)
    }

    /// Rewrites the latest line recording `old` as `new`; false if none
    /// records it.
    pub fn replace(&self, old: &Record, new: &Record) -> Result<bool> {
        replace(&self.history(), old, new)
    }

    /// Every readable record, oldest first; a missing file is no history.
    pub fn load(&self) -> Result<History> {
        load(&self.history(), |r: &Record| r.v <= VERSION)
    }

    pub fn append_read(&self, record: &ReadRecord) -> Result<()> {
        append(&self.reads(), record)
    }

    /// As [`Store::replace`], of a reading test.
    pub fn replace_read(
        &self,
        old: &ReadRecord,
        new: &ReadRecord,
    ) -> Result<bool> {
        replace(&self.reads(), old, new)
    }

    /// As [`Store::load`], of the reading tests.
    pub fn load_reads(&self) -> Result<History<ReadRecord>> {
        load(&self.reads(), |r: &ReadRecord| r.v <= reads::VERSION)
    }
}

fn append<T: Serialize>(path: &Path, record: &T) -> Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)
            .with_context(|| format!("creating {}", dir.display()))?;
    }
    let mut line = serde_json::to_string(record)?;
    line.push('\n');
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .with_context(|| format!("opening {}", path.display()))?;
    file.write_all(line.as_bytes())
        .with_context(|| format!("writing {}", path.display()))
}

fn replace<T: Serialize>(path: &Path, old: &T, new: &T) -> Result<bool> {
    let (line, with) =
        (serde_json::to_string(old)?, serde_json::to_string(new)?);
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(e) => {
            return Err(e)
                .with_context(|| format!("reading {}", path.display()))
        }
    };
    let mut lines: Vec<&str> = text.lines().collect();
    let Some(i) = lines.iter().rposition(|l| *l == line) else {
        return Ok(false);
    };
    lines[i] = &with;
    let mut kept = lines.join("\n");
    kept.push('\n');
    // Written beside the file, then renamed over it, so a failure midway
    // leaves it whole.
    let tmp = path.with_extension("jsonl.tmp");
    fs::write(&tmp, kept)
        .with_context(|| format!("writing {}", tmp.display()))?;
    fs::rename(&tmp, path)
        .with_context(|| format!("replacing {}", path.display()))?;
    Ok(true)
}

/// The lines of `path` that parse into a record `readable` by this build.
fn load<T: DeserializeOwned>(
    path: &Path,
    readable: fn(&T) -> bool,
) -> Result<History<T>> {
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Ok(History::default())
        }
        Err(e) => {
            return Err(e)
                .with_context(|| format!("reading {}", path.display()))
        }
    };
    let mut history = History::default();
    for line in text.lines().filter(|l| !l.trim().is_empty()) {
        match serde_json::from_str::<T>(line) {
            Ok(r) if readable(&r) => history.records.push(r),
            _ => history.skipped += 1,
        }
    }
    Ok(history)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::Input;

    const MS: Duration = Duration::from_millis(1);

    /// A scratch directory removed on drop.
    struct TempDir(PathBuf);

    impl TempDir {
        fn new(name: &str) -> Self {
            let dir = env::temp_dir()
                .join(format!("oui-{name}-{}", std::process::id()));
            let _ = fs::remove_dir_all(&dir);
            TempDir(dir)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn played(
        words: &[&str],
        keys: &str,
        limit: Option<Duration>,
    ) -> (Session, Instant) {
        let words = words.iter().map(|w| w.to_string()).collect();
        let mut s = Session::new(words, limit);
        let t0 = Instant::now();
        for (i, c) in keys.chars().enumerate() {
            let input =
                if c == '<' { Input::Backspace } else { Input::Char(c) };
            s.apply(input, t0 + MS * 120 * i as u32);
        }
        (s, t0 + MS * 120 * keys.len() as u32)
    }

    fn meta() -> Meta {
        Meta {
            source: "english".into(),
            mode: "words".into(),
            with: vec![],
            plain: false,
            drill: false,
            layout: "qwerty".into(),
            cite: None,
        }
    }

    #[test]
    fn record_rebuilds_the_same_report() {
        let (s, now) = played(&["ab", "cd"], "ax<b cd", None);
        let record =
            Record::new(&s, now, "2026-10-24T10:00:00+02:00".into(), meta());
        let live = Report::new(&s, s.elapsed(now));
        let back = record.report();
        assert!(record.completed);
        assert_eq!((back.wpm, back.accuracy), (live.wpm, live.accuracy));
        assert_eq!(back.keys, live.keys);
        assert_eq!(back.bigrams, live.bigrams);
        assert_eq!(back.typos, live.typos);
    }

    #[test]
    fn strokes_serialise_as_compact_arrays() {
        let stroke = Stroke(120, 'b', 'x', Some('a'), true);
        assert_eq!(
            serde_json::to_string(&stroke).unwrap(),
            r#"[120,"b","x","a",true]"#
        );
    }

    #[test]
    fn cites_are_stored_only_when_present() {
        let (s, now) = played(&["a"], "a", None);
        let plain = Record::new(&s, now, String::new(), meta());
        let json = serde_json::to_string(&plain).unwrap();
        assert!(!json.contains("cite"), "older readers see no change");
        let back: Record = serde_json::from_str(&json).unwrap();
        assert_eq!(back.meta.cite, None);
        let cite = Some("Austen, Emma (1815)".to_string());
        let cited = Record { meta: Meta { cite, ..meta() }, ..plain };
        let json = serde_json::to_string(&cited).unwrap();
        assert_eq!(serde_json::from_str::<Record>(&json).unwrap(), cited);
    }

    #[test]
    fn only_reached_words_are_kept_and_early_ends_are_marked() {
        let (mut s, now) = played(&["a", "b", "c", "d"], "a b", None);
        s.finish(now);
        let record = Record::new(&s, now, String::new(), meta());
        assert_eq!(record.words, ["a", "b"]);
        assert_eq!(record.typed, ["a"]);
        assert!(!record.completed);
    }

    #[test]
    fn appended_records_load_back_in_order() {
        let tmp = TempDir::new("append");
        let store = Store::at(tmp.0.join("nested"));
        assert!(store.load().unwrap().records.is_empty(), "no file yet");
        for (i, keys) in ["a", "b"].iter().enumerate() {
            let (s, now) = played(&["a"], keys, None);
            store
                .append(&Record::new(&s, now, format!("t{i}"), meta()))
                .unwrap();
        }
        let history = store.load().unwrap();
        let at: Vec<_> =
            history.records.iter().map(|r| r.at.as_str()).collect();
        assert_eq!(at, ["t0", "t1"]);
        assert_eq!(history.skipped, 0);
    }

    #[test]
    fn replace_rewrites_the_latest_line_of_a_record() {
        let tmp = TempDir::new("replace");
        let store = Store::at(&tmp.0);
        let record = |at: &str| {
            let (s, now) = played(&["a"], "a", None);
            Record::new(&s, now, at.to_string(), meta())
        };
        let (a, b) = (record("t0"), record("t1"));
        let mut c = a.clone();
        c.flags.set(INCOGNITO, true);
        assert!(!store.replace(&a, &c).unwrap(), "no file yet");
        for r in [&a, &b, &a] {
            store.append(r).unwrap();
        }
        assert!(store.replace(&a, &c).unwrap());
        let loaded = store.load().unwrap().records;
        assert_eq!(loaded, [a.clone(), b.clone(), c.clone()]);
        assert!(store.replace(&a, &c).unwrap());
        assert!(!store.replace(&a, &c).unwrap(), "no line left of it");
        assert!(store.replace(&c, &a).unwrap());
        let loaded = store.load().unwrap().records;
        assert_eq!(loaded, [c.clone(), b.clone(), a.clone()]);
        let left: Vec<_> = fs::read_dir(&tmp.0).unwrap().collect();
        assert_eq!(left.len(), 1, "no temporary file stays");
    }

    #[test]
    fn flags_are_kept_whether_known_or_not() {
        let mut flags = Flags::default();
        assert!(!flags.has(INCOGNITO));
        flags.set(INCOGNITO, true);
        assert!(flags.has(INCOGNITO));
        let json = r#"["later","incognito"]"#;
        let read: Flags = serde_json::from_str(json).unwrap();
        assert!(read.has(INCOGNITO) && read.has("later"));
        assert_eq!(
            serde_json::to_string(&read).unwrap(),
            r#"["incognito","later"]"#
        );
        flags.set(INCOGNITO, false);
        assert!(flags.is_empty());
        let (s, now) = played(&["a"], "a", None);
        let plain = Record::new(&s, now, "t".into(), meta());
        let line = serde_json::to_string(&plain).unwrap();
        assert!(!line.contains("flags"), "none written when none set");
    }

    #[test]
    fn reading_tests_are_kept_in_a_file_of_their_own() {
        let tmp = TempDir::new("reads");
        let store = Store::at(&tmp.0);
        let record = reads::tests::record;
        let (a, b) = (record("t0"), record("t1"));
        assert!(store.load_reads().unwrap().records.is_empty());
        assert!(!store.replace_read(&a, &b).unwrap(), "no file yet");
        store.append_read(&a).unwrap();
        store.append_read(&b).unwrap();
        let newer = ReadRecord { v: reads::VERSION + 1, ..record("t2") };
        store.append_read(&newer).unwrap();
        let history = store.load_reads().unwrap();
        assert_eq!((&history.records, history.skipped), (&vec![a, b], 1));
        let mut flagged = history.records[0].clone();
        flagged.flags.set(INCOGNITO, true);
        assert!(store.replace_read(&history.records[0], &flagged).unwrap());
        let records = store.load_reads().unwrap().records;
        assert!(records[0].incognito() && !records[1].incognito());
        assert!(store.load().unwrap().records.is_empty(), "tests apart");
        assert!(!store.history().exists());
    }

    #[test]
    fn unreadable_and_newer_lines_are_skipped() {
        let tmp = TempDir::new("skip");
        let store = Store::at(&tmp.0);
        let (s, now) = played(&["a"], "a", None);
        let good = Record::new(&s, now, String::new(), meta());
        let newer = Record { v: VERSION + 1, ..good.clone() };
        let lines = [
            serde_json::to_string(&good).unwrap(),
            "{not json".to_string(),
            serde_json::to_string(&newer).unwrap(),
            String::new(),
        ];
        fs::create_dir_all(&tmp.0).unwrap();
        fs::write(store.history(), lines.join("\n")).unwrap();
        let history = store.load().unwrap();
        assert_eq!((history.records.len(), history.skipped), (1, 2));
    }

    #[test]
    fn default_dir_is_dot_oui_in_home() {
        // Read-only: setting OUI_HOME here would race the parallel tests.
        if env::var_os("OUI_HOME").is_none() {
            let home = PathBuf::from(env::var_os("HOME").unwrap());
            assert_eq!(Store::open().unwrap().dir(), home.join(".oui"));
        }
    }
}
