//! oui — the binary: CLI, terminal and the event loop over the two screens.

use std::fs;
use std::io::IsTerminal;
use std::num::{NonZeroU64, NonZeroUsize};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};
use clap::{Args, Parser, Subcommand};
use fastrand::Rng;
use ratatui::crossterm::event::{
    self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers,
};
use ratatui::{DefaultTerminal, Frame};

use oui::history::{Meta, Record, Store};
use oui::keyboard::{Layout, Measure};
use oui::session::{Input, Session};
use oui::stats::{self, Report};
use oui::summary;
use oui::ui;
use oui::words::{self, Source};

/// Words in a test when neither `--words` nor a file says otherwise.
const DEFAULT_WORDS: usize = 50;
/// Words a timed test starts with, and tops up by when running low.
const BATCH: usize = 100;
/// Redraw interval, which keeps the clock of a timed test moving.
const FRAME: Duration = Duration::from_millis(100);
/// Bigrams a drill takes from each of the error and slowness rankings.
const DRILL: usize = 3;
/// Tests of the same source a drill looks back over, this one included.
const RECENT: usize = 20;

/// Typing test that shows which keys and bigrams slow you down.
#[derive(Parser, Debug)]
#[command(name = "oui", version, about, args_conflicts_with_subcommands = true)]
struct Cli {
    #[command(subcommand)]
    cmd: Option<Cmd>,
    #[command(flatten)]
    test: TestArgs,
}

#[derive(Subcommand, Debug, PartialEq, Eq)]
enum Cmd {
    /// Summarise the recorded tests: speed over time, heatmaps, rankings.
    Stats {
        /// Only the most recent N tests.
        #[arg(short = 'n', long)]
        last: Option<NonZeroUsize>,
        /// Only tests of this language or file name.
        #[arg(short, long)]
        source: Option<String>,
        /// Heatmap layout; detected from the recorded words by default.
        #[arg(short = 'k', long, value_enum)]
        layout: Option<Layout>,
    },
}

#[derive(Args, Debug)]
struct TestArgs {
    /// Text file to type in order, instead of random words.
    file: Option<PathBuf>,
    /// Bundled word list to draw from: english, french or russian.
    #[arg(short, long, default_value = "english", conflicts_with = "file")]
    language: String,
    /// Number of words in the test (default 50, or the whole file).
    #[arg(short, long, conflicts_with = "time")]
    words: Option<NonZeroUsize>,
    /// Make it a timed test of this many seconds.
    #[arg(short, long)]
    time: Option<NonZeroU64>,
    /// Keyboard layout the heatmap is drawn on; detected from the words by
    /// default, so Russian text gets ЙЦУКЕН.
    #[arg(short = 'k', long, value_enum)]
    layout: Option<Layout>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Limit {
    Words(usize),
    Time(Duration),
}

enum Screen {
    Typing(Session),
    Results { report: Report, measure: Measure },
}

/// Where a session's words come from: the source, or a drill over it.
struct Feed {
    source: Source,
    /// Bigrams being drilled; empty for a normal test.
    drill: Vec<(char, char)>,
    /// Words taken from the source for the current session.
    drawn: usize,
    rng: Rng,
}

impl Feed {
    fn take(&mut self, n: usize) -> Vec<String> {
        if !self.drill.is_empty() {
            let pool = self.source.words();
            return words::practice(pool, &self.drill, n, &mut self.rng);
        }
        let words = self.source.take(self.drawn, n, &mut self.rng);
        self.drawn += words.len();
        words
    }
}

/// A run's settings, resolved from the command line.
struct Setup {
    source: Source,
    limit: Limit,
    layout: Layout,
    title: String,
    meta: Meta,
}

struct App {
    feed: Feed,
    limit: Limit,
    layout: Layout,
    title: String,
    meta: Meta,
    /// Where finished tests are recorded; `None` records nothing.
    store: Option<Store>,
    /// Shown on the results screen, e.g. when a record could not be saved.
    notice: Option<String>,
    /// The last [`RECENT`] tests of this source, which drills draw on.
    recent: Vec<Report>,
    screen: Screen,
}

impl App {
    fn new(setup: Setup, store: Option<Store>) -> Self {
        let Setup { source, limit, layout, title, meta } = setup;
        let (recent, notice) = match store.as_ref().map(|s| recent(s, &meta)) {
            Some(Err(e)) => (Vec::new(), Some(format!("history: {e:#}"))),
            Some(Ok(recent)) => (recent, None),
            None => (Vec::new(), None),
        };
        let mut app = App {
            feed: Feed { source, drill: Vec::new(), drawn: 0, rng: Rng::new() },
            limit,
            layout,
            title,
            meta,
            store,
            notice,
            recent,
            screen: Screen::Typing(Session::new(Vec::new(), None)),
        };
        app.restart();
        app
    }

    fn restart(&mut self) {
        self.feed.drawn = 0;
        let session = match self.limit {
            Limit::Words(n) => Session::new(self.feed.take(n), None),
            Limit::Time(limit) => {
                Session::new(self.feed.take(BATCH), Some(limit))
            }
        };
        self.screen = Screen::Typing(session);
    }

    fn title(&self) -> String {
        if self.feed.drill.is_empty() {
            return self.title.clone();
        }
        let show =
            |&(a, b): &(char, char)| format!("{}{}", ui::show(a), ui::show(b));
        let drill: Vec<String> = self.feed.drill.iter().map(show).collect();
        format!("{} · drill {}", self.title, drill.join(" "))
    }

    /// Advances the clock; returns to results once the session is over.
    fn tick(&mut self, now: Instant) {
        let Screen::Typing(session) = &mut self.screen else { return };
        session.tick(now);
        if session.limit().is_some() && session.remaining() < BATCH / 2 {
            session.extend(self.feed.take(BATCH));
        }
        if session.finished() {
            let saved = save(self.store.as_ref(), session, now, &self.meta);
            self.notice = saved.err().map(|e| format!("not saved: {e:#}"));
            let report = Report::new(session, session.elapsed(now));
            if !session.log().is_empty() {
                self.recent.push(report.clone());
                let excess = self.recent.len().saturating_sub(RECENT);
                self.recent.drain(..excess);
            }
            self.screen =
                Screen::Results { report, measure: Measure::default() };
        }
    }

    /// Handles a key press; returns false to quit.
    fn key(&mut self, key: KeyEvent, now: Instant) -> bool {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let alt = key.modifiers.contains(KeyModifiers::ALT);
        if ctrl && key.code == KeyCode::Char('c') {
            return false;
        }
        match &mut self.screen {
            Screen::Typing(session) => {
                let input = match key.code {
                    KeyCode::Esc if !session.started() => return false,
                    KeyCode::Esc => {
                        session.finish(now);
                        None
                    }
                    KeyCode::Tab => {
                        self.restart();
                        return true;
                    }
                    KeyCode::Backspace if ctrl || alt => {
                        Some(Input::DeleteWord)
                    }
                    KeyCode::Char('w') if ctrl => Some(Input::DeleteWord),
                    KeyCode::Backspace => Some(Input::Backspace),
                    KeyCode::Char(c) if !ctrl => Some(Input::Char(c)),
                    _ => None,
                };
                if let Some(input) = input {
                    session.apply(input, now);
                }
                self.tick(now);
            }
            Screen::Results { measure, .. } => match key.code {
                KeyCode::Char('q') | KeyCode::Esc => return false,
                KeyCode::Char('r') => {
                    self.feed.drill.clear();
                    self.restart();
                }
                KeyCode::Char('p') => {
                    let recent = stats::combine(&self.recent);
                    self.feed.drill = recent.weak_bigrams(DRILL);
                    self.restart();
                }
                KeyCode::Tab => *measure = measure.toggle(),
                _ => {}
            },
        }
        true
    }

    fn draw(&self, frame: &mut Frame, now: Instant) {
        let title = self.title();
        match &self.screen {
            Screen::Typing(session) => {
                ui::typing::draw(frame, session, now, &title)
            }
            Screen::Results { report, measure } => {
                let notice = self.notice.as_deref();
                let (layout, measure) = (self.layout, *measure);
                ui::results::draw(
                    frame, report, layout, measure, &title, notice,
                )
            }
        }
    }
}

/// Reports of the last [`RECENT`] recorded tests of the same source.
fn recent(store: &Store, meta: &Meta) -> Result<Vec<Report>> {
    let records = store.load()?.records;
    let same = records.iter().filter(|r| r.meta.source == meta.source);
    let mut reports: Vec<Report> = same.map(Record::report).collect();
    reports.drain(..reports.len().saturating_sub(RECENT - 1));
    Ok(reports)
}

/// Records a finished session, unless nothing was typed.
fn save(
    store: Option<&Store>,
    session: &Session,
    now: Instant,
    meta: &Meta,
) -> Result<()> {
    let Some(store) = store else { return Ok(()) };
    if session.log().is_empty() {
        return Ok(());
    }
    let at = chrono::Local::now()
        .to_rfc3339_opts(chrono::SecondsFormat::Secs, false);
    store.append(&Record::new(session, now, at, meta.clone()))
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let store = Store::open()?;
    match cli.cmd {
        Some(Cmd::Stats { last, source, layout }) => {
            stats(&store, last, source.as_deref(), layout)
        }
        None => {
            let app = App::new(setup(&cli.test)?, Some(store));
            ratatui::run(|terminal| run(terminal, app))
        }
    }
}

fn stats(
    store: &Store,
    last: Option<NonZeroUsize>,
    source: Option<&str>,
    layout: Option<Layout>,
) -> Result<()> {
    let history = store.load()?;
    if history.skipped > 0 {
        let path = store.history();
        eprintln!(
            "oui: skipped {} unreadable lines in {}",
            history.skipped,
            path.display()
        );
    }
    let records = select(history.records, last, source);
    if records.is_empty() {
        println!("no tests recorded in {}", store.history().display());
        return Ok(());
    }
    let layout = layout.unwrap_or_else(|| {
        let words: Vec<String> =
            records.iter().flat_map(|r| r.words.iter().cloned()).collect();
        Layout::detect(&words)
    });
    let colour = std::io::stdout().is_terminal();
    print!("{}", summary::render(&records, layout, colour));
    Ok(())
}

/// The records of `source` (all if `None`), the last `last` of them.
fn select(
    records: Vec<Record>,
    last: Option<NonZeroUsize>,
    source: Option<&str>,
) -> Vec<Record> {
    let mut records: Vec<Record> = records
        .into_iter()
        .filter(|r| source.is_none_or(|s| r.meta.source == s))
        .collect();
    if let Some(n) = last {
        records.drain(..records.len().saturating_sub(n.get()));
    }
    records
}

fn setup(cli: &TestArgs) -> Result<Setup> {
    let (source, name) = source(cli)?;
    let limit = limit(cli, &source);
    let title = match limit {
        Limit::Words(n) => format!("{name} · {n} words"),
        Limit::Time(d) => format!("{name} · {}s", d.as_secs()),
    };
    let layout = cli.layout.unwrap_or_else(|| Layout::detect(source.words()));
    let mode = match source {
        Source::Random(_) => "words",
        Source::Text(_) => "file",
    };
    let meta = Meta {
        source: name,
        mode: mode.to_string(),
        with: Vec::new(),
        layout: layout.name().to_string(),
    };
    Ok(Setup { source, limit, layout, title, meta })
}

fn run(terminal: &mut DefaultTerminal, mut app: App) -> Result<()> {
    loop {
        let now = Instant::now();
        app.tick(now);
        terminal.draw(|frame| app.draw(frame, now))?;
        if !event::poll(FRAME)? {
            continue;
        }
        let Event::Key(key) = event::read()? else { continue };
        if key.kind == KeyEventKind::Press && !app.key(key, Instant::now()) {
            return Ok(());
        }
    }
}

/// The words to type and a name for them.
fn source(cli: &TestArgs) -> Result<(Source, String)> {
    if let Some(path) = &cli.file {
        let text = fs::read_to_string(path)
            .with_context(|| format!("reading {}", path.display()))?;
        let words = words::split(&text);
        if words.is_empty() {
            bail!("{} has no words to type", path.display());
        }
        let name = path.file_name().unwrap_or(path.as_os_str());
        return Ok((Source::Text(words), name.to_string_lossy().into_owned()));
    }
    let Some(words) = words::language(&cli.language) else {
        let known: Vec<&str> =
            words::LANGUAGES.iter().map(|(n, _)| *n).collect();
        bail!(
            "unknown language '{}' (have {})",
            cli.language,
            known.join(", ")
        );
    };
    Ok((Source::Random(words), cli.language.clone()))
}

fn limit(cli: &TestArgs, source: &Source) -> Limit {
    if let Some(secs) = cli.time {
        return Limit::Time(Duration::from_secs(secs.get()));
    }
    match (cli.words, source) {
        (Some(n), _) => Limit::Words(n.get()),
        (None, Source::Text(text)) => Limit::Words(text.len()),
        (None, Source::Random(_)) => Limit::Words(DEFAULT_WORDS),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cli(args: &[&str]) -> TestArgs {
        Cli::try_parse_from([&["oui"][..], args].concat()).unwrap().test
    }

    fn press(app: &mut App, code: KeyCode) -> bool {
        app.key(KeyEvent::new(code, KeyModifiers::NONE), Instant::now())
    }

    fn words(text: &str) -> Source {
        Source::Text(words::split(text))
    }

    fn app(text: &str, limit: Limit) -> App {
        App::new(test_setup(words(text), limit), None)
    }

    fn test_setup(source: Source, limit: Limit) -> Setup {
        let layout = Layout::Qwerty;
        Setup {
            source,
            limit,
            layout,
            title: "t".into(),
            meta: Meta::default(),
        }
    }

    /// A scratch directory removed on drop.
    struct TempDir(PathBuf);

    impl TempDir {
        fn new(name: &str) -> Self {
            let dir = std::env::temp_dir()
                .join(format!("oui-main-{name}-{}", std::process::id()));
            let _ = fs::remove_dir_all(&dir);
            TempDir(dir)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn defaults_to_fifty_english_words() {
        let cli = cli(&[]);
        let (source, name) = source(&cli).unwrap();
        assert_eq!(name, "english");
        assert_eq!(limit(&cli, &source), Limit::Words(50));
        assert_eq!(cli.layout, None);
        assert_eq!(Layout::detect(source.words()), Layout::Qwerty);
    }

    #[test]
    fn a_text_runs_whole_unless_limited() {
        let text = words("one two three");
        assert_eq!(limit(&cli(&["notes.txt"]), &text), Limit::Words(3));
        assert_eq!(
            limit(&cli(&["notes.txt", "-w", "2"]), &text),
            Limit::Words(2)
        );
        let timed = limit(&cli(&["-t", "30"]), &text);
        assert_eq!(timed, Limit::Time(Duration::from_secs(30)));
    }

    #[test]
    fn rejects_conflicting_or_zero_limits() {
        let parse =
            |args: &[&str]| Cli::try_parse_from([&["oui"][..], args].concat());
        assert!(parse(&["-w", "10", "-t", "30"]).is_err());
        assert!(parse(&["-w", "0"]).is_err());
        assert!(parse(&["notes.txt", "-l", "french"]).is_err());
        assert!(parse(&["-k", "qwerty", "-l", "french"]).is_ok());
        assert!(parse(&["-k", "йцукен"])
            .is_ok_and(|c| c.test.layout == Some(Layout::Jcuken)));
    }

    #[test]
    fn stats_is_a_subcommand_that_takes_no_test_flags() {
        let parse =
            |args: &[&str]| Cli::try_parse_from([&["oui"][..], args].concat());
        let cmd = parse(&["stats", "-n", "5", "-s", "russian"]).unwrap().cmd;
        assert_eq!(
            cmd,
            Some(Cmd::Stats {
                last: NonZeroUsize::new(5),
                source: Some("russian".into()),
                layout: None,
            })
        );
        assert!(parse(&["stats", "-w", "10"]).is_err());
        assert!(parse(&["-w", "10"]).is_ok_and(|c| c.cmd.is_none()));
    }

    #[test]
    fn select_filters_by_source_then_keeps_the_last() {
        let record = |source: &str, at: &str| {
            let s = Session::new(vec!["a".into()], None);
            let meta = Meta { source: source.into(), ..Meta::default() };
            Record::new(&s, Instant::now(), at.into(), meta)
        };
        let all = vec![
            record("english", "1"),
            record("russian", "2"),
            record("russian", "3"),
            record("russian", "4"),
        ];
        let at =
            |rs: Vec<Record>| rs.into_iter().map(|r| r.at).collect::<Vec<_>>();
        assert_eq!(
            at(select(all.clone(), NonZeroUsize::new(2), Some("russian"))),
            ["3", "4"]
        );
        assert_eq!(at(select(all.clone(), None, Some("english"))), ["1"]);
        assert_eq!(select(all, NonZeroUsize::new(9), None).len(), 4);
    }

    #[test]
    fn unknown_language_lists_the_bundled_ones() {
        let err = source(&cli(&["-l", "klingon"])).unwrap_err().to_string();
        assert_eq!(
            err,
            "unknown language 'klingon' (have english, french, russian)"
        );
    }

    #[test]
    fn finishing_the_text_shows_results() {
        let mut app = app("ab cd", Limit::Words(2));
        for c in "ab cd".chars() {
            assert!(press(&mut app, KeyCode::Char(c)));
        }
        let Screen::Results { report, .. } = &app.screen else {
            panic!("still typing");
        };
        assert_eq!(report.correct_words, 2);
        press(&mut app, KeyCode::Char('r'));
        assert!(matches!(app.screen, Screen::Typing(_)));
    }

    #[test]
    fn escape_quits_before_typing_and_ends_the_test_after() {
        let new = || app("ab", Limit::Words(1));
        assert!(!press(&mut new(), KeyCode::Esc));
        let mut app = new();
        press(&mut app, KeyCode::Char('a'));
        assert!(press(&mut app, KeyCode::Esc));
        assert!(matches!(app.screen, Screen::Results { .. }));
        assert!(!press(&mut app, KeyCode::Char('q')));
    }

    #[test]
    fn timed_tests_top_up_words_and_texts_continue() {
        let mut app = app("a b c", Limit::Time(Duration::from_secs(60)));
        let Screen::Typing(session) = &app.screen else { unreachable!() };
        assert_eq!(session.words().len(), BATCH);
        assert_eq!(session.words()[..4], ["a", "b", "c", "a"]);
        for _ in 0..BATCH / 2 + 1 {
            press(&mut app, KeyCode::Char('a'));
            press(&mut app, KeyCode::Char(' '));
        }
        let Screen::Typing(session) = &app.screen else { unreachable!() };
        assert_eq!(session.words().len(), 2 * BATCH);
        assert_eq!(
            session.words()[BATCH],
            "b",
            "text resumes where it left off"
        );
    }

    #[test]
    fn drill_feeds_back_the_weak_bigrams_until_a_new_test() {
        let mut app = app("ab cd", Limit::Words(2));
        for code in [
            KeyCode::Char('a'),
            KeyCode::Char('x'),
            KeyCode::Backspace,
            KeyCode::Char('b'),
            KeyCode::Char(' '),
            KeyCode::Char('c'),
            KeyCode::Char('d'),
        ] {
            press(&mut app, code);
        }
        press(&mut app, KeyCode::Char('p'));
        assert_eq!(app.feed.drill, [('a', 'b')]);
        assert_eq!(app.title(), "t · drill ab");
        let Screen::Typing(session) = &app.screen else { unreachable!() };
        assert_eq!(session.words(), ["ab", "ab"]);
        for c in "ab ab".chars() {
            press(&mut app, KeyCode::Char(c));
        }
        press(&mut app, KeyCode::Char('r'));
        assert!(app.feed.drill.is_empty());
        assert_eq!(app.title(), "t");
    }

    #[test]
    fn each_test_with_keystrokes_is_recorded() {
        let tmp = TempDir::new("record");
        let store = Store::at(&tmp.0);
        let setup = test_setup(words("ab"), Limit::Words(1));
        let mut app = App::new(setup, Some(store.clone()));
        press(&mut app, KeyCode::Char('a'));
        press(&mut app, KeyCode::Char('b'));
        press(&mut app, KeyCode::Char('r'));
        press(&mut app, KeyCode::Char('a'));
        press(&mut app, KeyCode::Esc);
        press(&mut app, KeyCode::Char('r'));
        press(&mut app, KeyCode::Tab);
        let records = store.load().unwrap().records;
        let completed: Vec<bool> =
            records.iter().map(|r| r.completed).collect();
        assert_eq!(completed, [true, false]);
        assert!(app.notice.is_none());
    }

    #[test]
    fn a_failed_save_is_shown_not_fatal() {
        let tmp = TempDir::new("blocked");
        fs::write(&tmp.0, "a file, not a dir").unwrap();
        let setup = test_setup(words("a"), Limit::Words(1));
        let mut app = App::new(setup, Some(Store::at(&tmp.0)));
        press(&mut app, KeyCode::Char('a'));
        assert!(matches!(app.screen, Screen::Results { .. }));
        assert!(app
            .notice
            .as_deref()
            .is_some_and(|n| n.starts_with("not saved")));
        let _ = fs::remove_file(&tmp.0);
    }

    #[test]
    fn drills_draw_on_earlier_tests_of_the_same_source() {
        let tmp = TempDir::new("recent");
        let store = Store::at(&tmp.0);
        let meta =
            |source: &str| Meta { source: source.into(), ..Meta::default() };
        // An earlier test that missed the d of "cd".
        let mut past = Session::new(vec!["cd".into()], None);
        let t0 = Instant::now();
        let ms = |n| t0 + Duration::from_millis(n);
        past.apply(Input::Char('c'), ms(0));
        past.apply(Input::Char('x'), ms(100));
        past.apply(Input::Backspace, ms(200));
        past.apply(Input::Char('d'), ms(300));
        for source in ["t", "other"] {
            store
                .append(&Record::new(&past, t0, String::new(), meta(source)))
                .unwrap();
        }
        let mut setup = test_setup(words("ab cd"), Limit::Words(2));
        setup.meta = meta("t");
        let mut app = App::new(setup, Some(store));
        assert_eq!(app.recent.len(), 1, "only the same source");
        for c in "ab cd".chars() {
            press(&mut app, KeyCode::Char(c));
        }
        assert_eq!(app.recent.len(), 2);
        press(&mut app, KeyCode::Char('p'));
        assert_eq!(app.feed.drill, [('c', 'd')]);
    }

    #[test]
    fn tab_toggles_the_heatmap_measure() {
        let mut app = app("a", Limit::Words(1));
        press(&mut app, KeyCode::Char('a'));
        press(&mut app, KeyCode::Tab);
        let Screen::Results { measure, .. } = app.screen else {
            unreachable!()
        };
        assert_eq!(measure, Measure::Speed);
    }
}
