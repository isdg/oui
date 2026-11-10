//! `oui read` on the terminal: which text a test reads, its keys and its
//! event loop.

use std::collections::HashSet;
use std::fs;
use std::num::{NonZeroU32, NonZeroUsize};
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::{anyhow, bail, Context, Result};
use clap::Args;
use fastrand::Rng;
use ratatui::crossterm::event::{
    self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers,
};
use ratatui::{DefaultTerminal, Frame};

use oui::history::Store;
use oui::probes::{self, Probe};
use oui::read::{Input, Origin, ReadTest, Stage, Text, PROBES, WORDS};
use oui::reads::ReadRecord;
use oui::ui;
use oui::words;

const DEFAULT_LANGUAGE: &str = "english";
const FRAME: Duration = Duration::from_millis(100);
/// Between frames of a sliding text, which moves a character at a time.
const SLIDE_FRAME: Duration = Duration::from_millis(30);

#[derive(Args, Debug, Clone, Default, PartialEq, Eq)]
pub struct ReadArgs {
    /// Text to read on from where its last test stopped: running prose in
    /// a text file, or a markdown article. Without one, the excerpts of a
    /// bundled work neither typed nor read before.
    file: Option<PathBuf>,
    /// Language of the bundled work: english (default), french or
    /// russian, or any with texts in ~/.oui/texts/<language>/.
    #[arg(short, long, conflicts_with = "file")]
    language: Option<String>,
    /// Words to read, up to the end of a paragraph (default 400).
    #[arg(short, long)]
    words: Option<NonZeroUsize>,
    /// Slide the text past at this many WPM, five characters a word,
    /// instead of paging through it at your own speed: the test of a
    /// pace, which counts if the text was still taken in.
    #[arg(short, long, value_name = "WPM")]
    pace: Option<NonZeroU32>,
    /// Read FILE from its start again.
    #[arg(short, long, requires = "file")]
    restart: bool,
    /// Record nothing: the test stays out of reads.jsonl and `oui stats`.
    #[arg(short, long)]
    incognito: bool,
}

/// What was typed or read before, which a new test builds on.
#[derive(Debug, Clone, Default)]
struct Seen {
    /// Citations of the tests typed.
    typed: HashSet<String>,
    /// The reading tests, oldest first.
    reads: Vec<ReadRecord>,
}

impl Seen {
    fn load(store: &Store) -> Result<Self> {
        let typed = store.load()?.records.into_iter();
        let typed = typed.filter_map(|r| r.meta.cite).collect();
        Ok(Seen { typed, reads: store.load_reads()?.records })
    }
}

/// A test to run: what to read, what to ask and where it is from.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Plan {
    paragraphs: Vec<Vec<String>>,
    probes: Vec<Probe>,
    origin: Origin,
    notice: Option<String>,
}

impl Plan {
    /// The test of it, sliding past at `pace` if one is set.
    fn test(&self, pace: Option<NonZeroU32>) -> ReadTest {
        let test = ReadTest::new(self.paragraphs.clone(), self.probes.clone());
        match pace {
            Some(pace) => test.paced(pace),
            None => test,
        }
    }

    fn title(&self) -> String {
        let words: usize = self.paragraphs.iter().map(Vec::len).sum();
        let excerpts = if self.origin.continuous { "" } else { " excerpts" };
        format!("read · {}{excerpts} · {words} words", self.origin.source)
    }
}

/// The next test of `args`, its language defaulting to `language`: of
/// the file from its place, `restart` aside, else of a work unseen.
fn plan(
    args: &ReadArgs,
    language: &str,
    restart: bool,
    texts: &Path,
    seen: &Seen,
    rng: &mut Rng,
) -> Result<Plan> {
    let words = args.words.map_or(WORDS, NonZeroUsize::get);
    let (text, mut origin, notice, from) = match &args.file {
        Some(path) => file(path, restart, seen)?,
        None => {
            let (text, origin) = work(language, words, texts, seen, rng)?;
            (text, origin, None, 0)
        }
    };
    let Some(stretch) = text.stretch(from, words, PROBES) else {
        bail!(
            "too little left of {} to test: it takes {PROBES} sentences \
             to ask about in what is read and as many in what is not",
            origin.source
        );
    };
    let paragraphs = |range: Range<usize>| -> Vec<&[String]> {
        text.paragraphs[range].iter().map(Vec::as_slice).collect()
    };
    let read = paragraphs(stretch.clone());
    let unread = paragraphs(stretch.end..text.paragraphs.len());
    let probes = probes::draw(&read, &unread, PROBES, rng);
    origin.start = text.paragraphs[..from].iter().map(Vec::len).sum();
    let paragraphs = text.paragraphs[stretch].to_vec();
    Ok(Plan { paragraphs, probes, origin, notice })
}

/// The text of the file at `path`, and the paragraph its next test
/// starts on: where the last one stopped, unless the file has changed.
fn file(
    path: &Path,
    restart: bool,
    seen: &Seen,
) -> Result<(Text, Origin, Option<String>, usize)> {
    let read = fs::read_to_string(path)
        .with_context(|| format!("reading {}", path.display()))?;
    let markdown = path
        .extension()
        .is_some_and(|x| x.eq_ignore_ascii_case("md") || x == "markdown");
    let text = Text::file(&read, markdown);
    let whole = fs::canonicalize(path)
        .with_context(|| format!("resolving {}", path.display()))?;
    let whole = whole.to_string_lossy().into_owned();
    let name = path.file_name().unwrap_or(path.as_os_str());
    let name = name.to_string_lossy().into_owned();
    let total = text.words();
    let of_file = |r: &&ReadRecord| r.file.as_deref() == Some(&whole);
    let last = seen.reads.iter().rev().find(of_file).filter(|_| !restart);
    let (from, notice) = match last {
        None => (0, None),
        Some(last) if last.total != total => {
            (0, Some(format!("{name} has changed: from its start")))
        }
        Some(last) => match text.paragraph_at(last.start + last.words) {
            Some(from) => (from, None),
            None => bail!(
                "{name} is read through: --restart reads it from its start"
            ),
        },
    };
    let origin = Origin {
        source: name,
        cite: text.cite.clone(),
        file: Some(whole),
        continuous: true,
        start: 0,
        total,
    };
    Ok((text, origin, notice, from))
}

/// A work of `language` neither typed nor read, long enough to test
/// `words` of, picked at random.
fn work(
    language: &str,
    words: usize,
    texts: &Path,
    seen: &Seen,
    rng: &mut Rng,
) -> Result<(Text, Origin)> {
    let works = Text::works(words::load_passages(language, texts)?);
    if works.is_empty() {
        let known: Vec<&str> =
            words::LANGUAGES.iter().map(|(n, _)| *n).collect();
        let known = known.join(", ");
        bail!("no works to read in '{language}' (have {known})");
    }
    let read: HashSet<&str> =
        seen.reads.iter().filter_map(|r| r.cite.as_deref()).collect();
    let fresh = |text: &Text| {
        let cite = text.cite.as_deref().unwrap_or_default();
        !seen.typed.contains(cite)
            && !read.contains(cite)
            && text.stretch(0, words, PROBES).is_some()
    };
    let mut fresh: Vec<Text> = works.into_iter().filter(fresh).collect();
    if fresh.is_empty() {
        return Err(anyhow!(
            "every {language} work long enough was typed or read already: \
             give `oui read` a FILE"
        ));
    }
    let text = fresh.swap_remove(rng.usize(..fresh.len()));
    let origin = Origin {
        source: language.to_string(),
        cite: text.cite.clone(),
        file: None,
        continuous: false,
        start: 0,
        total: text.words(),
    };
    Ok((text, origin))
}

struct App {
    args: ReadArgs,
    language: String,
    /// `None` records nothing and builds on nothing.
    store: Option<Store>,
    seen: Seen,
    plan: Plan,
    test: ReadTest,
    /// The finished test, and whether the history holds it.
    record: Option<(ReadRecord, bool)>,
    notice: Option<String>,
    /// The terminal's columns and rows, as last drawn.
    size: (u16, u16),
    rng: Rng,
}

impl App {
    fn new(
        args: ReadArgs,
        language: Option<String>,
        store: Option<Store>,
    ) -> Result<Self> {
        let language = args.language.clone().or(language);
        let language = language.unwrap_or_else(|| DEFAULT_LANGUAGE.into());
        let seen = store.as_ref().map(Seen::load).transpose()?;
        let seen = seen.unwrap_or_default();
        let mut rng = Rng::new();
        let texts = texts(store.as_ref());
        let plan =
            plan(&args, &language, args.restart, &texts, &seen, &mut rng)?;
        let test = plan.test(args.pace);
        let notice = plan.notice.clone();
        Ok(App {
            args,
            language,
            store,
            seen,
            plan,
            test,
            record: None,
            notice,
            size: (0, 0),
            rng,
        })
    }

    /// The next test: the file from where this one stopped, or another
    /// work. A text with no test left says so and stays on the results.
    fn next(&mut self) {
        let texts = texts(self.store.as_ref());
        let (args, language) = (&self.args, &self.language);
        match plan(args, language, false, &texts, &self.seen, &mut self.rng) {
            Ok(plan) => {
                self.test = plan.test(self.args.pace);
                self.notice = plan.notice.clone();
                self.plan = plan;
                self.record = None;
            }
            Err(e) => self.notice = Some(format!("{e:#}")),
        }
    }

    /// Records the test just done; what was read counts as seen even if
    /// the history is not to hold it.
    fn finish(&mut self) {
        let at = chrono::Local::now()
            .to_rfc3339_opts(chrono::SecondsFormat::Secs, false);
        let record = self.test.record(at, self.plan.origin.clone(), self.size);
        self.seen.reads.push(record.clone());
        let store = self.store.as_ref().filter(|_| !self.args.incognito);
        let kept = match store.map(|s| s.append_read(&record)) {
            Some(Ok(())) => true,
            Some(Err(e)) => {
                self.notice = Some(format!("not saved: {e:#}"));
                false
            }
            None => false,
        };
        self.record = Some((record, kept));
    }

    /// Takes the test on the results out of the history, or puts it back.
    fn toggle_recorded(&mut self) {
        let (Some(store), Some((record, kept))) =
            (&self.store, &mut self.record)
        else {
            return;
        };
        let done = match *kept {
            true => store.remove_read(record).map(drop),
            false => store.append_read(record),
        };
        match done {
            Ok(()) => (*kept, self.notice) = (!*kept, None),
            Err(e) => self.notice = Some(format!("history: {e:#}")),
        }
    }

    /// Handles a key press; returns false to quit.
    fn key(&mut self, key: KeyEvent, now: Instant) -> bool {
        use KeyCode::{Char, Down, Enter, Esc, Left, PageDown, PageUp};
        use KeyCode::{Right, Up};

        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        if ctrl && key.code == Char('c') {
            return false;
        }
        let input = match (self.test.stage(), key.code) {
            (Stage::Done, Char('r')) => {
                self.next();
                None
            }
            (Stage::Done, Char('i')) => {
                self.toggle_recorded();
                None
            }
            (Stage::Done | Stage::Ready, Char('q')) | (_, Esc) => return false,
            (Stage::Ready, Char(' ') | Enter) => Some(Input::Next),
            (
                Stage::Reading,
                Char(' ' | 'j' | 'l') | Enter | Right | Down | PageDown,
            ) => Some(Input::Next),
            (Stage::Reading, Char('k' | 'h' | 'b') | Left | Up | PageUp) => {
                Some(Input::Back)
            }
            // The keys of `y` and `n` on ЙЦУКЕН too.
            (Stage::Probing, Char('y' | 'н') | Right) => Some(Input::Yes),
            (Stage::Probing, Char('n' | 'т') | Left) => Some(Input::No),
            _ => None,
        };
        if let Some(input) = input {
            self.test.apply(input, now);
            self.settle();
        }
        true
    }

    /// Advances the clock of a sliding text.
    fn tick(&mut self, now: Instant) {
        self.test.tick(now);
        self.settle();
    }

    /// Records the test once it is done.
    fn settle(&mut self) {
        if self.test.stage() == Stage::Done && self.record.is_none() {
            self.finish();
        }
    }

    fn title(&self) -> String {
        match self.args.pace {
            Some(pace) => format!("{} · {pace} wpm", self.plan.title()),
            None => self.plan.title(),
        }
    }

    /// How long to wait for a key before drawing again.
    fn frame(&self) -> Duration {
        let slides = self.test.pace().is_some();
        match slides && self.test.stage() == Stage::Reading {
            true => SLIDE_FRAME,
            false => FRAME,
        }
    }

    /// Draws the screen, the pages laid out for the terminal as it is.
    fn draw(&mut self, frame: &mut Frame, now: Instant) {
        let area = frame.area();
        self.size = (area.width, area.height);
        let (width, height) = ui::read::text_size(area.width, area.height);
        let pages = ui::read::pages(self.test.paragraphs(), width, height);
        self.test.lay(pages, now);
        let title = self.title();
        let cite = self.plan.origin.cite.as_deref();
        match (self.test.stage(), &self.record) {
            (Stage::Ready, _) => ui::read::ready(frame, &self.test, &title),
            (Stage::Reading, _) if self.test.pace().is_some() => {
                ui::read::stripe(frame, &self.test, &title, now)
            }
            (Stage::Reading, _) => {
                ui::read::page(frame, &self.test, &title, cite)
            }
            (Stage::Probing, _) => ui::read::probe(frame, &self.test, &title),
            (Stage::Done, Some((record, kept))) => {
                let title = match cite {
                    Some(cite) => format!("{title} · {cite}"),
                    None => title,
                };
                let notice = self.notice.as_deref();
                let kept = self.store.as_ref().map(|_| *kept);
                ui::read::results(frame, record, &title, notice, kept)
            }
            (Stage::Done, None) => {}
        }
    }
}

/// Where the user's own texts are kept; nowhere without a store.
fn texts(store: Option<&Store>) -> PathBuf {
    store.map_or_else(PathBuf::new, |s| s.dir().join("texts"))
}

/// Runs `oui read`, `language` being the config's.
pub fn run(
    args: ReadArgs,
    language: Option<String>,
    store: Store,
) -> Result<()> {
    let app = App::new(args, language, Some(store))?;
    ratatui::run(|terminal| event_loop(terminal, app))
}

fn event_loop(terminal: &mut DefaultTerminal, mut app: App) -> Result<()> {
    loop {
        let now = Instant::now();
        app.tick(now);
        terminal.draw(|frame| app.draw(frame, now))?;
        if !event::poll(app.frame())? {
            continue;
        }
        let Event::Key(key) = event::read()? else { continue };
        if key.kind == KeyEventKind::Press && !app.key(key, Instant::now()) {
            return Ok(());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A scratch directory removed on drop.
    struct TempDir(PathBuf);

    impl TempDir {
        fn new(name: &str) -> Self {
            let dir = std::env::temp_dir()
                .join(format!("oui-read-{name}-{}", std::process::id()));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(&dir).unwrap();
            TempDir(dir)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    /// A book of `n` paragraphs, each two sentences to ask about, the
    /// i-th naming itself.
    fn book(dir: &Path, n: usize) -> PathBuf {
        let paragraph = |i| {
            format!(
                "Paragraph {i} opens with eight words in it. \
                 Then {i} closes with another eight words here.\n"
            )
        };
        let text: Vec<String> = (0..n).map(paragraph).collect();
        let path = dir.join("book.txt");
        fs::write(&path, text.join("\n")).unwrap();
        path
    }

    fn args(file: &Path, words: usize) -> ReadArgs {
        ReadArgs {
            file: Some(file.to_path_buf()),
            words: NonZeroUsize::new(words),
            ..ReadArgs::default()
        }
    }

    /// The record of a test read through, of a bundled work.
    fn app_record(texts: &Path) -> ReadRecord {
        let mut app = App::new(ReadArgs::default(), None, None).unwrap();
        assert_eq!(super::texts(None), texts);
        read_through(&mut app);
        app.record.unwrap().0
    }

    fn press(app: &mut App, code: KeyCode) -> bool {
        app.key(KeyEvent::new(code, KeyModifiers::NONE), Instant::now())
    }

    /// Reads the text through and answers every probe "read".
    fn read_through(app: &mut App) {
        while app.test.stage() != Stage::Probing {
            assert!(press(app, KeyCode::Char(' ')));
        }
        while app.test.stage() != Stage::Done {
            assert!(press(app, KeyCode::Char('y')));
        }
    }

    #[test]
    fn a_file_is_read_on_from_where_its_last_test_stopped() {
        let tmp = TempDir::new("place");
        let file = book(&tmp.0, 30);
        let store = || Some(Store::at(tmp.0.join("home")));
        // Sixteen words a paragraph: 80 words are five of them.
        let mut app = App::new(args(&file, 80), None, store()).unwrap();
        assert_eq!(app.plan.title(), "read · book.txt · 80 words");
        assert_eq!((app.plan.origin.start, app.plan.origin.total), (0, 480));
        assert_eq!(app.plan.probes.len(), PROBES);
        read_through(&mut app);
        let (record, kept) = app.record.clone().unwrap();
        assert!(kept && record.continuous);
        assert_eq!((record.start, record.words), (0, 80));
        assert_eq!(record.probes.len(), PROBES);

        press(&mut app, KeyCode::Char('r'));
        assert_eq!(app.test.stage(), Stage::Ready);
        assert_eq!(app.plan.origin.start, 80, "r reads on");
        assert!(app.plan.paragraphs[0][1] == "5", "the sixth paragraph");

        let again = App::new(args(&file, 80), None, store()).unwrap();
        assert_eq!(again.plan.origin.start, 80, "so does a later run");
        let restart = ReadArgs { restart: true, ..args(&file, 80) };
        let restarted = App::new(restart, None, store()).unwrap();
        assert_eq!(restarted.plan.origin.start, 0);
    }

    #[test]
    fn a_changed_file_starts_over_and_a_finished_one_says_so() {
        let tmp = TempDir::new("changed");
        let file = book(&tmp.0, 30);
        let store = || Some(Store::at(tmp.0.join("home")));
        let mut app = App::new(args(&file, 400), None, store()).unwrap();
        assert_eq!(app.plan.origin.start, 0);
        let read: usize = app.plan.paragraphs.iter().map(Vec::len).sum();
        assert_eq!(read, 400, "all but the foils");
        read_through(&mut app);
        press(&mut app, KeyCode::Char('r'));
        let notice = app.notice.clone().unwrap();
        assert!(notice.starts_with("too little left of book.txt"), "{notice}");
        assert_eq!(app.test.stage(), Stage::Done, "the results stay");

        book(&tmp.0, 31);
        let app = App::new(args(&file, 80), None, store()).unwrap();
        assert_eq!(app.plan.origin.start, 0);
        assert_eq!(
            app.notice.as_deref(),
            Some("book.txt has changed: from its start")
        );
    }

    #[test]
    fn a_file_too_short_to_test_is_refused() {
        let tmp = TempDir::new("short");
        let file = book(&tmp.0, 6);
        let err = App::new(args(&file, 80), None, None).err().unwrap();
        assert!(err.to_string().starts_with("too little left"), "{err}");
        let missing = args(&tmp.0.join("none.txt"), 80);
        let err = App::new(missing, None, None).err().unwrap();
        assert!(err.to_string().starts_with("reading "), "{err}");
    }

    #[test]
    fn without_a_file_a_work_never_seen_is_read_as_excerpts() {
        let tmp = TempDir::new("works");
        let store = || Some(Store::at(&tmp.0));
        let mut app = App::new(ReadArgs::default(), None, store()).unwrap();
        let cite = app.plan.origin.cite.clone().unwrap();
        assert!(!app.plan.origin.continuous);
        assert!(app.plan.title().starts_with("read · english excerpts · "));
        read_through(&mut app);
        for _ in 0..30 {
            press(&mut app, KeyCode::Char('r'));
            assert_ne!(app.plan.origin.cite.as_ref(), Some(&cite));
            assert_eq!(app.test.stage(), Stage::Ready);
        }
        let french = Some("french".to_string());
        let app = App::new(ReadArgs::default(), french, None).unwrap();
        assert_eq!(app.plan.origin.source, "french", "the config's");
        let args = ReadArgs {
            language: Some("klingon".into()),
            ..ReadArgs::default()
        };
        let err = App::new(args, None, None).err().unwrap();
        assert!(err.to_string().starts_with("no works to read"), "{err}");
    }

    #[test]
    fn works_typed_or_read_before_are_passed_over() {
        let texts = Path::new("");
        let all = Text::works(words::load_passages("english", texts).unwrap());
        let mut seen = Seen::default();
        let cite = |t: &Text| t.cite.clone().unwrap();
        let long = |t: &&Text| t.stretch(0, 400, PROBES).is_some();
        let left = cite(all.iter().find(long).unwrap());
        seen.typed = all.iter().map(cite).filter(|c| *c != left).collect();
        let mut rng = Rng::with_seed(1);
        let (text, origin) = work("english", 400, texts, &seen, &mut rng)
            .expect("one work is left");
        assert_eq!(text.cite.as_ref(), Some(&left));
        assert_eq!(origin.total, text.words());
        seen.reads = vec![ReadRecord { cite: Some(left), ..app_record(texts) }];
        let err = work("english", 400, texts, &seen, &mut rng).err().unwrap();
        assert!(err.to_string().contains("typed or read already"), "{err}");
    }

    #[test]
    fn incognito_keeps_the_test_out_until_i_records_it() {
        let tmp = TempDir::new("incognito");
        let file = book(&tmp.0, 30);
        let store = Store::at(tmp.0.join("home"));
        let args = ReadArgs { incognito: true, ..args(&file, 80) };
        let mut app = App::new(args, None, Some(store.clone())).unwrap();
        read_through(&mut app);
        assert!(store.load_reads().unwrap().records.is_empty());
        press(&mut app, KeyCode::Char('i'));
        assert_eq!(store.load_reads().unwrap().records.len(), 1);
        press(&mut app, KeyCode::Char('i'));
        assert!(store.load_reads().unwrap().records.is_empty());
        press(&mut app, KeyCode::Char('r'));
        assert_eq!(app.plan.origin.start, 80, "read all the same");
    }

    #[test]
    fn a_paced_test_runs_on_the_clock_and_records_its_pace() {
        let tmp = TempDir::new("pace");
        let file = book(&tmp.0, 30);
        let store = Store::at(tmp.0.join("home"));
        let pace = NonZeroU32::new(300);
        let args = ReadArgs { pace, ..args(&file, 80) };
        let mut app = App::new(args, None, Some(store.clone())).unwrap();
        assert_eq!(app.title(), "read · book.txt · 80 words · 300 wpm");
        assert_eq!(app.frame(), FRAME);
        let t0 = Instant::now();
        app.key(KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE), t0);
        assert_eq!(app.frame(), SLIDE_FRAME);
        press(&mut app, KeyCode::Char(' '));
        let slide = app.test.slide().unwrap();
        app.tick(t0 + slide - Duration::from_millis(1));
        assert_eq!(app.test.stage(), Stage::Reading, "no key hurries it");
        app.tick(t0 + slide);
        assert_eq!((app.test.stage(), app.frame()), (Stage::Probing, FRAME));
        while app.test.stage() != Stage::Done {
            press(&mut app, KeyCode::Char('n'));
        }
        let record = &store.load_reads().unwrap().records[0];
        assert_eq!(record.pace, Some(300.0));
        assert!((record.wpm - 300.0).abs() < 0.5, "{}", record.wpm);
    }

    #[test]
    fn keys_turn_pages_answer_probes_and_quit() {
        let tmp = TempDir::new("keys");
        let file = book(&tmp.0, 30);
        let mut app = App::new(args(&file, 80), None, None).unwrap();
        assert!(press(&mut app, KeyCode::Char('y')), "not yet a probe");
        assert_eq!(app.test.stage(), Stage::Ready);
        press(&mut app, KeyCode::Enter);
        assert_eq!(app.test.stage(), Stage::Reading);
        assert!(press(&mut app, KeyCode::Char('q')), "q is no key here");
        press(&mut app, KeyCode::Right);
        assert_eq!(app.test.stage(), Stage::Probing, "one page, unlaid");
        press(&mut app, KeyCode::Char('н'));
        press(&mut app, KeyCode::Left);
        let answered = |app: &App| app.test.probe().map(|(at, _)| at);
        assert_eq!(answered(&app), Some(2));
        assert!(!press(&mut app, KeyCode::Esc), "esc abandons");
        let ctrl_c = KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL);
        assert!(!app.key(ctrl_c, Instant::now()));
        assert!(app.record.is_none(), "nothing recorded");
    }
}
