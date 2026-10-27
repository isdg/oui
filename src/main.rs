//! oui — the binary: CLI, terminal and the event loop over the two screens.

use std::fs;
use std::io::IsTerminal;
use std::num::{NonZeroU64, NonZeroUsize};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use anyhow::{anyhow, bail, Context, Result};
use clap::{Args, Parser, Subcommand};
use fastrand::Rng;
use ratatui::crossterm::event::{
    self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers,
};
use ratatui::{DefaultTerminal, Frame};

use oui::chain::Chain;
use oui::config::Config;
use oui::extras::{Decorator, Extra};
use oui::history::{Meta, Record, Store};
use oui::keyboard::{Layout, Measure};
use oui::practice::{self, Target};
use oui::session::{Input, Session};
use oui::stats::{self, Report};
use oui::summary::{self, Period, View};
use oui::ui;
use oui::words::{self, Mode, Source};

const DEFAULT_LANGUAGE: &str = "english";
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
/// Words a practice test draws from, the richest in costly bigrams.
const PRACTICE: usize = 150;
/// Targets a practice test names in its title, costliest first.
const FOCUS: usize = 3;

const FILES_HELP: &str = "\
Files (under $OUI_HOME, default ~/.oui):
  config                 defaults for any flag above, one `key = value` a
                         line; flags override them:
                           language = russian
                           mode     = words        # or text, classics, funny
                           with     = punctuation, numbers
                           words    = 50           # or time = 60
                           layout   = auto         # or qwerty, jcuken
  history.jsonl          every test, keystrokes included; read by `oui stats`
                         and --practice
  texts/<language>/*.txt your own passages for --mode text, blank-line
                         separated; a [[Author, Work]] line cites those
                         after it";

/// Typing test that shows which keys and bigrams slow you down.
#[derive(Parser, Debug)]
#[command(
    name = "oui",
    version,
    about,
    args_conflicts_with_subcommands = true,
    after_help = FILES_HELP
)]
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
        /// Group tests by day, week or month: a table of periods, and WPM
        /// plotted per period instead of per test.
        #[arg(short, long, value_enum)]
        by: Option<Period>,
        /// Add the heatmaps and the key and bigram rankings.
        #[arg(short, long)]
        detailed: bool,
        /// Add fingers, rows and hand moves -- same-finger bigrams among
        /// them -- and the fingering your timing implies.
        #[arg(short, long)]
        geometry: bool,
        /// Add what --practice targets per language, and the words it
        /// draws.
        #[arg(short, long)]
        practice: bool,
    },
}

/// What a test is; each flag can be defaulted in ~/.oui/config.
#[derive(Args, Debug, Clone, Default)]
struct TestArgs {
    /// Text file to type in order, instead of random words.
    file: Option<PathBuf>,
    /// Language to type: english (default), french or russian, or any with
    /// texts in ~/.oui/texts/<language>/ for --mode text.
    #[arg(short, long, conflicts_with = "file")]
    language: Option<String>,
    /// Random words (default), or passages of real prose.
    #[arg(short, long, value_enum, conflicts_with = "file")]
    mode: Option<Mode>,
    /// Random words richest in the bigrams that cost your tests of the
    /// language the most time, misses included.
    #[arg(short, long, conflicts_with_all = ["file", "mode"])]
    practice: bool,
    /// Extras mixed into random words, comma-separated: punctuation,
    /// numbers, capitals, symbols (or p, n, c, s).
    #[arg(
        short = 'W',
        long,
        value_enum,
        value_delimiter = ',',
        conflicts_with = "file"
    )]
    with: Vec<Extra>,
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
    /// Record nothing: tests are left out of history.jsonl and `oui stats`.
    #[arg(short, long, alias = "inkognito")]
    incognito: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Limit {
    Words(usize),
    Time(Duration),
    /// One passage of a text-mode source, however long.
    Passage,
}

enum Screen {
    Typing(Session),
    Results {
        report: Box<Report>,
        measure: Measure,
        /// The finished test, for its text.
        session: Session,
        /// How far the text is scrolled while shown instead of the results.
        review: Option<usize>,
    },
}

/// Where a session's words come from: the source, or a drill over it.
struct Feed {
    source: Source,
    /// Bigrams being drilled; empty for a normal test.
    drill: Vec<(char, char)>,
    /// The `--with` extras; drills are left plain so they hit their bigrams.
    decorator: Decorator,
    /// Where the current session starts in the source, and how many words
    /// it has taken from there.
    start: usize,
    drawn: usize,
    rng: Rng,
}

impl Feed {
    fn take(&mut self, n: usize) -> Vec<String> {
        if !self.drill.is_empty() {
            let pool = self.source.words();
            return words::practice(pool, &self.drill, n, &mut self.rng);
        }
        let from = self.start + self.drawn;
        let words = self.source.take(from, n, &mut self.rng);
        self.drawn += words.len();
        self.decorator.apply(words, &mut self.rng)
    }

    /// Where the passage a test started on is from; drills cite nothing.
    fn cite(&self) -> Option<&str> {
        if !self.drill.is_empty() {
            return None;
        }
        self.source.cite(self.start)
    }
}

/// A run's settings, resolved from the command line.
struct Setup {
    source: Source,
    limit: Limit,
    layout: Layout,
    title: String,
    meta: Meta,
    extras: Vec<Extra>,
    incognito: bool,
}

struct App {
    feed: Feed,
    limit: Limit,
    layout: Layout,
    title: String,
    meta: Meta,
    /// The history drills draw on; `None` reads nothing.
    store: Option<Store>,
    /// Finished tests are not recorded in the store.
    incognito: bool,
    /// Shown on the results screen, e.g. when a record could not be saved.
    notice: Option<String>,
    /// The last [`RECENT`] tests of this source, which drills draw on.
    recent: Vec<Report>,
    screen: Screen,
}

impl App {
    fn new(setup: Setup, store: Option<Store>) -> Self {
        let Setup { source, limit, layout, title, meta, extras, incognito } =
            setup;
        let (recent, notice) = match store.as_ref().map(|s| recent(s, &meta)) {
            Some(Err(e)) => (Vec::new(), Some(format!("history: {e:#}"))),
            Some(Ok(recent)) => (recent, None),
            None => (Vec::new(), None),
        };
        let mut app = App {
            feed: Feed {
                source,
                drill: Vec::new(),
                decorator: Decorator::new(&extras),
                start: 0,
                drawn: 0,
                rng: Rng::new(),
            },
            limit,
            layout,
            title,
            meta,
            store,
            incognito,
            notice,
            recent,
            screen: Screen::Typing(Session::new(Vec::new(), None)),
        };
        app.restart();
        app
    }

    fn restart(&mut self) {
        self.feed.drawn = 0;
        self.feed.decorator.reset();
        self.feed.start = self.feed.source.start(&mut self.feed.rng);
        let session = match self.limit {
            Limit::Words(n) => Session::new(self.feed.take(n), None),
            Limit::Passage => {
                let n = self.feed.source.passage_len(self.feed.start);
                Session::new(self.feed.take(n.unwrap_or(DEFAULT_WORDS)), None)
            }
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
            let cite = self.feed.cite().map(str::to_owned);
            let meta = Meta { cite, ..self.meta.clone() };
            let store = self.store.as_ref().filter(|_| !self.incognito);
            let saved = save(store, session, now, &meta);
            self.notice = saved.err().map(|e| format!("not saved: {e:#}"));
            let report = Report::new(session, session.elapsed(now));
            if !session.log().is_empty() {
                self.recent.push(report.clone());
                let excess = self.recent.len().saturating_sub(RECENT);
                self.recent.drain(..excess);
            }
            self.screen = Screen::Results {
                report: Box::new(report),
                measure: Measure::default(),
                session: session.clone(),
                review: None,
            };
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
            Screen::Results { measure, session, review, .. } => {
                match key.code {
                    KeyCode::Char('q') | KeyCode::Esc => return false,
                    KeyCode::Char('o') => {
                        *review = if review.is_some() { None } else { Some(0) };
                    }
                    KeyCode::Down | KeyCode::Char('j') => {
                        if let Some(row) = review {
                            *row = (*row + 1).min(session.words().len());
                        }
                    }
                    KeyCode::Up | KeyCode::Char('k') => {
                        if let Some(row) = review {
                            *row = row.saturating_sub(1);
                        }
                    }
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
                }
            }
        }
        true
    }

    /// Draws the screen; a scrolled text keeps the scroll it could show.
    fn draw(&mut self, frame: &mut Frame, now: Instant) {
        let title = self.title();
        let cite = self.feed.cite();
        match &mut self.screen {
            Screen::Typing(session) => {
                ui::typing::draw(frame, session, now, &title, cite)
            }
            Screen::Results { session, review: Some(row), .. } => {
                *row = ui::review::draw(frame, session, &title, cite, *row);
            }
            Screen::Results { report, measure, .. } => {
                let notice = self.notice.as_deref();
                let (layout, measure) = (self.layout, *measure);
                let title = match cite {
                    Some(cite) => format!("{title} · {cite}"),
                    None => title,
                };
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
        Some(Cmd::Stats {
            last,
            source,
            layout,
            by,
            detailed,
            geometry,
            practice,
        }) => {
            let view = View { by, detailed, geometry, ..View::default() };
            stats(&store, last, source.as_deref(), layout, view)?;
            if practice {
                practice_summary(&store, last, source.as_deref())?;
            }
            Ok(())
        }
        None => {
            let config = Config::load(&store.dir().join("config"))?;
            let args = merge(&cli.test, &config);
            let app = App::new(setup(&args, &store)?, Some(store));
            ratatui::run(|terminal| run(terminal, app))
        }
    }
}

fn stats(
    store: &Store,
    last: Option<NonZeroUsize>,
    source: Option<&str>,
    layout: Option<Layout>,
    mut view: View,
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
    view.layout = layout.unwrap_or_else(|| {
        let words: Vec<String> =
            records.iter().flat_map(|r| r.words.iter().cloned()).collect();
        Layout::detect(&words)
    });
    view.colour = std::io::stdout().is_terminal();
    print!("{}", summary::render(&records, &view));
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

/// The flags given, with the config filling in the rest. A FILE takes no
/// language, mode or extras from the config, nor does text mode extras,
/// nor practice a mode.
fn merge(cli: &TestArgs, config: &Config) -> TestArgs {
    fn pick<T>(
        flag: Option<T>,
        set: Option<T>,
        from_config: bool,
    ) -> Option<T> {
        flag.or(set.filter(|_| from_config))
    }
    let from_config = cli.file.is_none();
    let mode = pick(cli.mode, config.mode, from_config && !cli.practice);
    let with = match cli.with.is_empty() {
        false => cli.with.clone(),
        true if from_config && !mode.is_some_and(Mode::passages) => {
            config.with.clone()
        }
        true => Vec::new(),
    };
    let (words, time) = match cli.words.is_some() || cli.time.is_some() {
        true => (cli.words, cli.time),
        false => (config.words, config.time),
    };
    TestArgs {
        file: cli.file.clone(),
        language: pick(
            cli.language.clone(),
            config.language.clone(),
            from_config,
        ),
        mode,
        practice: cli.practice,
        with,
        words,
        time,
        layout: cli.layout.or(config.layout),
        incognito: cli.incognito,
    }
}

/// The run's settings, drawing on the passages and history in `store`.
fn setup(cli: &TestArgs, store: &Store) -> Result<Setup> {
    let (source, name, focus) = source(cli, store)?;
    let limit = limit(cli, &source);
    if matches!(source, Source::Passages { .. }) && !cli.with.is_empty() {
        bail!("--with mixes extras into random words, not passages");
    }
    let mut extras = cli.with.clone();
    extras.sort();
    extras.dedup();
    let with: Vec<String> =
        extras.iter().map(|e| e.name().to_string()).collect();
    let mode = match source {
        _ if cli.practice => "practice",
        Source::Random(_) => "words",
        Source::Text(_) => "file",
        Source::Passages { .. } => cli.mode.unwrap_or(Mode::Text).name(),
        Source::Funny(_) => "funny",
    };
    let mut what = match mode {
        "words" | "file" => name.clone(),
        _ => format!("{name} {mode}"),
    };
    if !with.is_empty() {
        what = format!("{what} with {}", with.join(", "));
    }
    if !focus.is_empty() {
        let show =
            |&(a, b): &(char, char)| format!("{}{}", ui::show(a), ui::show(b));
        let focus: Vec<String> = focus.iter().map(show).collect();
        what = format!("{what} · {}", focus.join(" "));
    }
    let mut title = match limit {
        Limit::Words(n) => format!("{what} · {n} words"),
        Limit::Time(d) => format!("{what} · {}s", d.as_secs()),
        Limit::Passage => format!("{what} · one passage"),
    };
    if cli.incognito {
        title.push_str(" · incognito");
    }
    let layout = cli.layout.unwrap_or_else(|| Layout::detect(source.words()));
    let meta = Meta {
        source: name,
        mode: mode.to_string(),
        with,
        layout: layout.name().to_string(),
        cite: None,
    };
    let incognito = cli.incognito;
    Ok(Setup { source, limit, layout, title, meta, extras, incognito })
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

/// The words to type, a name for them and the bigrams they practise.
type Words = (Source, String, Vec<(char, char)>);

fn source(cli: &TestArgs, store: &Store) -> Result<Words> {
    if let Some(path) = &cli.file {
        let text = fs::read_to_string(path)
            .with_context(|| format!("reading {}", path.display()))?;
        let words = words::split(&text);
        if words.is_empty() {
            bail!("{} has no words to type", path.display());
        }
        let name = path.file_name().unwrap_or(path.as_os_str());
        let name = name.to_string_lossy().into_owned();
        return Ok((Source::Text(words), name, Vec::new()));
    }
    let language =
        cli.language.clone().unwrap_or_else(|| DEFAULT_LANGUAGE.to_string());
    if cli.practice {
        let records = select(store.load()?.records, None, Some(&language));
        let (targets, words) = practice_pool(&language, &records, store)?;
        let focus = targets.iter().take(FOCUS).map(|t| t.bigram).collect();
        return Ok((Source::Random(words), language, focus));
    }
    if cli.mode == Some(Mode::Classics) {
        let passages = words::classics(&language);
        if passages.is_empty() {
            bail!("no classics for '{language}'");
        }
        return Ok((Source::passages(passages), language, Vec::new()));
    }
    if let Some(mode @ (Mode::Text | Mode::Funny)) = cli.mode {
        let texts = store.dir().join("texts");
        let passages = words::load_passages(&language, &texts)?;
        if passages.is_empty() {
            let dir = texts.join(&language);
            bail!(
                "no texts for '{language}' (add .txt files to {})",
                dir.display()
            );
        }
        let source = match mode {
            Mode::Funny => Source::Funny(Chain::new(&passages)),
            _ => Source::passages(passages),
        };
        return Ok((source, language, Vec::new()));
    }
    let Some(words) = words::language(&language) else {
        return Err(unknown(&language));
    };
    Ok((Source::Random(words), language, Vec::new()))
}

fn unknown(language: &str) -> anyhow::Error {
    let known: Vec<&str> = words::LANGUAGES.iter().map(|(n, _)| *n).collect();
    anyhow!("unknown language '{language}' (have {})", known.join(", "))
}

/// The bigrams costing `records` of `language` the most, and the
/// [`PRACTICE`] words richest in them.
fn practice_pool(
    language: &str,
    records: &[Record],
    store: &Store,
) -> Result<(Vec<Target>, Vec<String>)> {
    let texts = store.dir().join("texts");
    let vocabulary = practice::vocabulary(language, &texts)?;
    if vocabulary.is_empty() {
        return Err(unknown(language));
    }
    if records.is_empty() {
        let history = store.history();
        bail!("no {language} tests to practise on in {}", history.display());
    }
    let targets = practice::targets(records, &vocabulary);
    let words = practice::words(&vocabulary, &targets, PRACTICE);
    if words.is_empty() {
        bail!("nothing to practise yet in the {language} tests");
    }
    Ok((targets, words))
}

/// What practice targets in each language of the selected tests.
fn practice_summary(
    store: &Store,
    last: Option<NonZeroUsize>,
    source: Option<&str>,
) -> Result<()> {
    let records = select(store.load()?.records, last, source);
    let mut languages: Vec<&str> =
        records.iter().map(|r| r.meta.source.as_str()).collect();
    languages.sort_unstable();
    languages.dedup();
    let texts = store.dir().join("texts");
    for language in languages {
        if practice::vocabulary(language, &texts)?.is_empty() {
            continue;
        }
        let tests = select(records.clone(), None, Some(language));
        let (targets, words) = practice_pool(language, &tests, store)?;
        print!("{}", summary::practice(language, &targets, &words));
    }
    Ok(())
}

fn limit(cli: &TestArgs, source: &Source) -> Limit {
    if let Some(secs) = cli.time {
        return Limit::Time(Duration::from_secs(secs.get()));
    }
    match (cli.words, source) {
        (Some(n), _) => Limit::Words(n.get()),
        (None, Source::Text(text)) => Limit::Words(text.len()),
        (None, Source::Random(_) | Source::Funny(_)) => {
            Limit::Words(DEFAULT_WORDS)
        }
        (None, Source::Passages { .. }) => Limit::Passage,
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

    fn no_home() -> Store {
        Store::at("/nonexistent/oui")
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
            extras: Vec::new(),
            incognito: false,
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
        let (source, name, focus) = source(&cli, &no_home()).unwrap();
        assert!(focus.is_empty());
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
    fn extras_parse_as_one_list_and_name_the_test() {
        let args = cli(&["-W", "n,p", "--with", "p,symbols", "-w", "9"]);
        let setup = setup(&args, &no_home()).unwrap();
        assert_eq!(setup.meta.with, ["punctuation", "numbers", "symbols"]);
        assert_eq!(
            setup.title,
            "english with punctuation, numbers, symbols · 9 words"
        );
        assert!(Cli::try_parse_from(["oui", "-W", "x"]).is_err());
        assert!(Cli::try_parse_from(["oui", "notes.txt", "-W", "p"]).is_err());
    }

    #[test]
    fn funny_mode_walks_the_prose_and_takes_extras() {
        let args = cli(&["-m", "funnywords", "-l", "russian", "-W", "p"]);
        let setup = setup(&args, &no_home()).unwrap();
        assert_eq!(setup.title, "russian funny with punctuation · 50 words");
        assert_eq!(setup.meta.mode, "funny");
        assert_eq!(setup.layout, Layout::Jcuken);
        let Source::Funny(chain) = &setup.source else { unreachable!() };
        let vocab = chain.words().to_vec();
        let app = App::new(setup, None);
        let Screen::Typing(session) = &app.screen else { unreachable!() };
        assert_eq!(session.words().len(), DEFAULT_WORDS);
        let known = |w: &String| {
            let bare = w.trim_matches(|c: char| !c.is_alphabetic());
            vocab.iter().any(|v| v.to_lowercase() == bare.to_lowercase())
        };
        assert!(session.words().iter().all(known));
    }

    #[test]
    fn text_mode_types_one_whole_passage() {
        let args = cli(&["-m", "text", "-l", "french"]);
        let setup = setup(&args, &no_home()).unwrap();
        assert_eq!(setup.title, "french text · one passage");
        assert_eq!(setup.meta.mode, "text");
        let app = App::new(setup, None);
        let Screen::Typing(session) = &app.screen else { unreachable!() };
        let len = app.feed.source.passage_len(app.feed.start);
        assert_eq!(Some(session.words().len()), len);
        assert!(session.words()[0].starts_with(char::is_uppercase));
        assert!(app.feed.cite().is_some_and(|c| c.contains(", ")), "bundled");
    }

    #[test]
    fn classics_mode_types_only_the_bundled_passages() {
        let tmp = TempDir::new("classics");
        let store = Store::at(&tmp.0);
        let mine = tmp.0.join("texts/french");
        fs::create_dir_all(&mine).unwrap();
        fs::write(mine.join("a.txt"), "[[Moi]]\nun deux").unwrap();
        let args = cli(&["-m", "classics", "-l", "french"]);
        let classics = setup(&args, &store).unwrap();
        assert_eq!(classics.title, "french classics · one passage");
        assert_eq!(classics.meta.mode, "classics");
        let Source::Passages { cites, .. } = &classics.source else {
            unreachable!()
        };
        assert_eq!(cites.len(), words::classics("french").len());
        assert!(cites.iter().all(|c| c.as_deref() != Some("Moi")));
        let err = |args: &[&str]| {
            setup(&cli(args), &store).err().unwrap().to_string()
        };
        assert!(err(&["-m", "classics", "-W", "p"]).starts_with("--with"));
        assert!(err(&["-m", "classics", "-l", "german"])
            .starts_with("no classics for 'german'"));
    }

    #[test]
    fn text_mode_refuses_extras_and_unknown_texts() {
        let err = |args: &[&str]| {
            setup(&cli(args), &no_home()).err().unwrap().to_string()
        };
        assert!(err(&["-m", "text", "-W", "p"]).starts_with("--with mixes"));
        assert!(err(&["-m", "text", "-l", "german"])
            .starts_with("no texts for 'german'"));
        assert!(
            Cli::try_parse_from(["oui", "notes.txt", "-m", "text"]).is_err()
        );
    }

    #[test]
    fn config_fills_in_what_flags_leave_out() {
        let config = oui::config::parse(
            "language = russian\nmode = words\nwith = p\ntime = 60\nlayout = jcuken",
        )
        .unwrap();
        let args = merge(&cli(&[]), &config);
        assert_eq!(args.language.as_deref(), Some("russian"));
        assert_eq!(args.with, [Extra::Punctuation]);
        assert_eq!((args.words, args.time), (None, NonZeroU64::new(60)));
        assert_eq!(args.layout, Some(Layout::Jcuken));

        let args =
            merge(&cli(&["-l", "french", "-w", "10", "-W", "n"]), &config);
        assert_eq!(args.language.as_deref(), Some("french"), "flags win");
        assert_eq!((args.words, args.time), (NonZeroUsize::new(10), None));
        assert_eq!(args.with, [Extra::Numbers]);
    }

    #[test]
    fn a_file_or_text_mode_takes_no_extras_from_config() {
        let config =
            oui::config::parse("language = russian\nwith = p").unwrap();
        let file = merge(&cli(&["notes.txt"]), &config);
        assert_eq!((file.language, file.with), (None, vec![]));
        let text = merge(&cli(&["-m", "text"]), &config);
        assert!(text.with.is_empty());
        assert!(merge(&cli(&["-m", "classics"]), &config).with.is_empty());
        assert!(setup(&text, &no_home()).is_ok(), "config extras are no error");
        let config = oui::config::parse("mode = text").unwrap();
        assert!(
            merge(&cli(&["-W", "p"]), &config).with == [Extra::Punctuation]
        );
    }

    #[test]
    fn help_documents_the_config_and_files() {
        use clap::CommandFactory;
        let help = Cli::command().render_long_help().to_string();
        for needle in
            ["config", "history.jsonl", "texts/<language>/", "with     ="]
        {
            assert!(help.contains(needle), "help lacks {needle:?}\n{help}");
        }
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
                by: None,
                detailed: false,
                geometry: false,
                practice: false,
            })
        );
        assert!(parse(&["stats", "-w", "10"]).is_err());
        let by = parse(&["stats", "--by", "week"]).unwrap().cmd;
        assert!(matches!(by, Some(Cmd::Stats { by: Some(Period::Week), .. })));
        let g = parse(&["stats", "-dg", "--by", "day"]).unwrap().cmd;
        assert!(matches!(
            g,
            Some(Cmd::Stats { detailed: true, geometry: true, .. })
        ));
        let p = parse(&["stats", "-p", "-s", "english"]).unwrap().cmd;
        assert!(matches!(p, Some(Cmd::Stats { practice: true, .. })));
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

    /// A store holding one english test of "aqua", slow from q to u.
    fn slow_qu(tmp: &TempDir) -> Store {
        let store = Store::at(&tmp.0);
        let mut past = Session::new(vec!["aqua".into()], None);
        let t0 = Instant::now();
        for (c, ms) in [('a', 0), ('q', 100), ('u', 600), ('a', 700)] {
            past.apply(Input::Char(c), t0 + Duration::from_millis(ms));
        }
        let meta = Meta { source: "english".into(), ..Meta::default() };
        store.append(&Record::new(&past, t0, String::new(), meta)).unwrap();
        store
    }

    #[test]
    fn practice_draws_words_holding_the_costliest_bigrams() {
        let tmp = TempDir::new("practice");
        let store = slow_qu(&tmp);
        let config = oui::config::parse("mode = text\nwith = p").unwrap();
        let args = merge(&cli(&["--practice", "-w", "9"]), &config);
        assert_eq!(args.mode, None, "the config's mode is ignored");
        let practice = setup(&args, &store).unwrap();
        assert_eq!(
            practice.title,
            "english practice with punctuation · qu · 9 words"
        );
        assert_eq!(
            (practice.meta.source.as_str(), practice.meta.mode.as_str()),
            ("english", "practice")
        );
        let Source::Random(pool) = &practice.source else { unreachable!() };
        assert!(pool.len() <= PRACTICE && pool.len() > 9);
        assert!(pool.iter().all(|w| w.contains("qu")), "{pool:?}");
        let again = setup(&args, &store).unwrap().source;
        assert_eq!(&again, &practice.source, "the same history, the same pool");
        let mut app = App::new(practice, Some(store));
        let Screen::Typing(session) = &app.screen else { unreachable!() };
        assert_eq!(session.words().len(), 9);
        for c in "quq".chars() {
            press(&mut app, KeyCode::Char(c));
        }
        press(&mut app, KeyCode::Esc);
        press(&mut app, KeyCode::Char('p'));
        assert!(!app.feed.drill.is_empty(), "drills still work");
    }

    #[test]
    fn practice_needs_history_and_takes_no_file_or_mode() {
        let parse =
            |args: &[&str]| Cli::try_parse_from([&["oui"][..], args].concat());
        assert!(parse(&["-p", "notes.txt"]).is_err());
        assert!(parse(&["-p", "-m", "text"]).is_err());
        assert!(parse(&["-p", "-l", "russian", "-t", "30"]).is_ok());
        let err = |args: &[&str]| {
            setup(&cli(args), &no_home()).err().unwrap().to_string()
        };
        assert_eq!(
            err(&["-p"]),
            "no english tests to practise on in /nonexistent/oui/history.jsonl"
        );
        assert!(err(&["-p", "-l", "klingon"]).starts_with("unknown language"));
    }

    #[test]
    fn unknown_language_lists_the_bundled_ones() {
        let err = source(&cli(&["-l", "klingon"]), &no_home())
            .unwrap_err()
            .to_string();
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
    fn passages_are_cited_except_in_drills() {
        let passage = words::Passage {
            words: words::split("ab cd"),
            cite: Some("X, Y (1900)".into()),
        };
        let source = Source::passages(vec![passage]);
        let mut app = App::new(test_setup(source, Limit::Passage), None);
        assert_eq!(app.feed.cite(), Some("X, Y (1900)"));
        app.feed.drill = vec![('a', 'b')];
        assert_eq!(app.feed.cite(), None);
        assert_eq!(self::app("ab", Limit::Words(1)).feed.cite(), None);
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
        assert!(records.iter().all(|r| r.meta.cite.is_none()));
        assert!(app.notice.is_none());
    }

    #[test]
    fn incognito_records_nothing_but_still_drills_on_history() {
        let tmp = TempDir::new("incognito");
        let store = Store::at(&tmp.0);
        let mut past = Session::new(vec!["cd".into()], None);
        let t0 = Instant::now();
        let ms = |n| t0 + Duration::from_millis(n);
        past.apply(Input::Char('c'), ms(0));
        past.apply(Input::Char('x'), ms(100));
        past.apply(Input::Backspace, ms(200));
        past.apply(Input::Char('d'), ms(300));
        let meta = Meta { source: "t".into(), ..Meta::default() };
        store.append(&Record::new(&past, t0, String::new(), meta)).unwrap();

        let args = cli(&["-i", "-w", "9"]);
        assert!(args.incognito);
        assert!(setup(&args, &no_home()).unwrap().title.ends_with("incognito"));
        assert!(cli(&["--inkognito"]).incognito);
        let mut setup = test_setup(words("ab cd"), Limit::Words(2));
        setup.meta.source = "t".into();
        setup.incognito = true;
        let mut app = App::new(setup, Some(store.clone()));
        for c in "ab cd".chars() {
            press(&mut app, KeyCode::Char(c));
        }
        assert!(matches!(app.screen, Screen::Results { .. }));
        assert!(app.notice.is_none());
        assert_eq!(store.load().unwrap().records.len(), 1, "not recorded");
        press(&mut app, KeyCode::Char('p'));
        assert_eq!(app.feed.drill, [('c', 'd')]);
    }

    #[test]
    fn a_passage_test_records_its_citation() {
        let tmp = TempDir::new("cite");
        let store = Store::at(&tmp.0);
        let passage = words::Passage {
            words: words::split("ab"),
            cite: Some("X, Y (1900)".into()),
        };
        let source = Source::passages(vec![passage]);
        let setup = test_setup(source, Limit::Passage);
        let mut app = App::new(setup, Some(store.clone()));
        press(&mut app, KeyCode::Char('a'));
        press(&mut app, KeyCode::Char('b'));
        let records = store.load().unwrap().records;
        assert_eq!(records[0].meta.cite.as_deref(), Some("X, Y (1900)"));
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

    #[test]
    fn o_shows_the_typed_text_and_arrows_scroll_it() {
        let mut app = app("ab cd", Limit::Words(2));
        for c in "ab cd".chars() {
            press(&mut app, KeyCode::Char(c));
        }
        let review = |app: &App| match &app.screen {
            Screen::Results { review, session, .. } => {
                assert_eq!(session.words(), ["ab", "cd"]);
                *review
            }
            Screen::Typing(_) => unreachable!(),
        };
        assert_eq!(review(&app), None);
        press(&mut app, KeyCode::Char('o'));
        assert_eq!(review(&app), Some(0));
        press(&mut app, KeyCode::Down);
        press(&mut app, KeyCode::Down);
        press(&mut app, KeyCode::Down);
        assert_eq!(review(&app), Some(2), "no further than the words");
        press(&mut app, KeyCode::Up);
        assert_eq!(review(&app), Some(1));
        press(&mut app, KeyCode::Char('o'));
        assert_eq!(review(&app), None);
        press(&mut app, KeyCode::Down);
        assert_eq!(review(&app), None, "arrows do nothing on the results");
    }
}
