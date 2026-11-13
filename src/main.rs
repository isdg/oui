//! oui — the binary: CLI, terminal and the event loop over the two screens.

use std::fs;
use std::io::IsTerminal;
use std::num::{NonZeroU64, NonZeroUsize};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::{anyhow, bail, Context, Result};
use clap::builder::ArgPredicate;
use clap::{Args, Parser, Subcommand, ValueEnum};
use fastrand::Rng;
use ratatui::crossterm::event::{
    self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers,
};
use ratatui::{DefaultTerminal, Frame};

use oui::chain::Chain;
use oui::config::Config;
use oui::external;
use oui::extras::{Decorator, Extra};
use oui::history::{Meta, Record, Store, INCOGNITO};
use oui::keyboard::{Layout, Measure};
use oui::ngrams::Ngrams;
use oui::practice::{self, Target};
use oui::reads::ReadRecord;
use oui::session::{Input, Session};
use oui::stats::Report;
use oui::summary::{self, Period, Plot, View};
use oui::ui;
use oui::ui::reader::{Reading, Scroll};
use oui::ui::results::Keys;
use oui::words::{self, Mode, ModeParser, Source, Texts};

mod read_app;

use read_app::ReadArgs;

const DEFAULT_LANGUAGE: &str = "english";
/// Words in a test when neither `--words` nor a file says otherwise.
const DEFAULT_WORDS: usize = 50;
/// Words a timed test starts with, and tops up by when running low.
const BATCH: usize = 100;
/// Redraw interval, which keeps the clock of a timed test moving.
const FRAME: Duration = Duration::from_millis(100);
/// Words a practice test draws from, the richest in costly bigrams.
const PRACTICE: usize = 150;
/// Targets a practice test names in its title, costliest first.
const FOCUS: usize = 3;

const FILES_HELP: &str = "\
Files:
  ~/.ouirc               defaults for any flag above, one `key = value` a
                         line; flags override them:
                           language = russian
                           mode     = words        # text, text:classics,
                                                   # funny, practice, ngrams:2
                           with     = punctuation, numbers
                           plain    = true         # as --plain
                           words    = 50           # or time = 60
                           layout   = auto         # or qwerty, jcuken
Under $OUI_HOME, default ~/.oui:
  history.jsonl          every test, keystrokes included; read by `oui stats`
                         and --practice
  reads.jsonl            every `oui read` test: the time on each page and
                         each sentence's answer
  texts/<language>/*.txt your own passages for --mode text, blank-line
                         separated; a [[Author, Work]] line cites those
                         after it
  external/<language>.txt
                         passages `oui external` extracts from an archive,
                         for --mode text and text:external;
                         external/readme.txt
                         lists the articles used";

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
        /// How WPM over time is drawn.
        #[arg(long, value_enum, default_value_t)]
        plot: Plot,
        /// Count the tests flagged incognito too.
        #[arg(long)]
        incognito: bool,
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
    /// Time yourself reading a text silently, then tell sentences of it
    /// from ones it never held: the speed counts if you can.
    Read(ReadArgs),
    /// Extract passages of prose from an archive of markdown articles into
    /// ~/.oui/external/, adding only articles its readme.txt does not list.
    #[command(alias = "ext")]
    External {
        /// Directories searched for `.md` articles with front matter.
        #[arg(required = true)]
        dirs: Vec<PathBuf>,
    },
}

/// What a test is; each flag can be defaulted in ~/.ouirc.
#[derive(Args, Debug, Clone, Default)]
struct TestArgs {
    /// Text file to type in order, instead of random words.
    file: Option<PathBuf>,
    /// Language to type: english (default), french or russian, or any with
    /// texts in ~/.oui/texts/<language>/ or external ones for --mode text.
    #[arg(short, long, conflicts_with = "file")]
    language: Option<String>,
    /// Random words (default), passages of real prose, practice, or letter
    /// n-grams; each mode also takes its first letter.
    #[arg(
        short,
        long,
        value_parser = ModeParser,
        conflicts_with = "file",
        default_value_if("practice", ArgPredicate::IsPresent, "practice")
    )]
    mode: Option<Mode>,
    /// Short for --mode practice.
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
    /// Type prose plain -- lowercase, bare of punctuation and numbers --
    /// in --mode text, funny and a FILE; --plain=false overrides ~/.ouirc.
    #[arg(
        short = 'P',
        long,
        num_args = 0..=1,
        require_equals = true,
        default_missing_value = "true"
    )]
    plain: Option<bool>,
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
    /// Go on from where the last test of FILE, else of text mode, stopped:
    /// in its mode, language and --plain, unless given.
    #[arg(short = 'c', long = "continue", conflicts_with = "practice")]
    resume: bool,
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
        /// Where the work is scrolled to while read instead of the results.
        reader: Option<Scroll>,
    },
}

/// Where a session's words come from.
struct Feed {
    source: Source,
    /// The `--with` extras.
    decorator: Decorator,
    /// Where the current session starts in the source, and how many words
    /// it has taken from there.
    start: usize,
    drawn: usize,
    /// The word the last test of an ordered source stopped before, where
    /// `c` goes on from.
    resume: Option<usize>,
    rng: Rng,
}

impl Feed {
    fn take(&mut self, n: usize) -> Vec<String> {
        let from = self.start + self.drawn;
        let words = self.source.take(from, n, &mut self.rng);
        self.drawn += words.len();
        self.decorator.apply(words, &mut self.rng)
    }

    /// The work the last test of the source was in, and the reader's
    /// place in it: all of it is behind once typing went on elsewhere.
    fn reading(&self) -> Reading<'_> {
        let words = self.source.words();
        let work = self.source.work(self.start);
        let next = self.resume.map(|at| at % words.len().max(1));
        let within = |at: &usize| work.iter().any(|p| p.contains(at));
        let place = match next.filter(within) {
            Some(at) => work.iter().take_while(|p| p.end <= at).count(),
            None => work.len(),
        };
        let passages = work.into_iter().map(|p| &words[p]).collect();
        Reading { passages, place }
    }

    /// Where the reader opens: on the place, else on the passage typed.
    fn opening(&self) -> Scroll {
        let reading = self.reading();
        let work = self.source.work(self.start);
        let typed = work.iter().position(|p| p.contains(&self.start));
        let passage = match reading.place < reading.passages.len() {
            true => reading.place,
            false => typed.unwrap_or(0),
        };
        Scroll { passage, row: 0 }
    }

    /// Where the passage a test started on is from.
    fn cite(&self) -> Option<&str> {
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
    /// What it was resolved from, which `p` sets practice up from.
    args: TestArgs,
}

/// What the tests of a run type, and how they are named and recorded.
struct Run {
    feed: Feed,
    limit: Limit,
    layout: Layout,
    title: String,
    meta: Meta,
}

impl Run {
    fn new(setup: Setup) -> Self {
        let Setup { source, limit, layout, title, meta, extras, .. } = setup;
        let feed = Feed {
            source,
            decorator: Decorator::new(&extras),
            start: 0,
            drawn: 0,
            resume: None,
            rng: Rng::new(),
        };
        Run { feed, limit, layout, title, meta }
    }
}

struct App {
    run: Run,
    /// The run `p` set aside to practise; `r`, `c` and the reader go back
    /// to it, and its place in the text is kept meanwhile.
    parked: Option<Run>,
    /// What the app was launched with, which `p` sets practice up from.
    args: TestArgs,
    /// The history tests are recorded in and practice draws on; `None`
    /// records and practises nothing.
    store: Option<Store>,
    /// Finished tests are recorded incognito.
    incognito: bool,
    /// Shown on the results screen, e.g. when a record could not be saved.
    notice: Option<String>,
    /// The record of the test on the results screen, as the history holds
    /// it: `i` flags it incognito and back.
    record: Option<Record>,
    screen: Screen,
}

impl App {
    fn new(setup: Setup, store: Option<Store>) -> Self {
        let (incognito, args) = (setup.incognito, setup.args.clone());
        let mut app = App {
            run: Run::new(setup),
            parked: None,
            args,
            store,
            incognito,
            notice: None,
            record: None,
            screen: Screen::Typing(Session::new(Vec::new(), None)),
        };
        app.restart();
        app
    }

    /// A new test, of another work if the source has works.
    fn restart(&mut self) {
        let feed = &mut self.run.feed;
        let start = feed.source.start(&mut feed.rng);
        self.begin(start);
    }

    /// Starts a test at word `start` of the source, wrapping past its end.
    fn begin(&mut self, start: usize) {
        (self.record, self.notice) = (None, None);
        let feed = &mut self.run.feed;
        feed.drawn = 0;
        feed.decorator.reset();
        feed.start = start % feed.source.words().len().max(1);
        let session = match self.run.limit {
            Limit::Words(n) => Session::new(feed.take(n), None),
            Limit::Passage => {
                let n = feed.source.passage_len(feed.start);
                Session::new(feed.take(n.unwrap_or(DEFAULT_WORDS)), None)
            }
            Limit::Time(limit) => Session::new(feed.take(BATCH), Some(limit)),
        };
        self.screen = Screen::Typing(session);
    }

    /// Switches to practice as `oui --practice` runs it, for the language
    /// and limits the app was launched with, setting the run aside.
    fn practise(&mut self) {
        let Some(store) = &self.store else { return };
        let mode = Some(Mode::Practice);
        let args = TestArgs { file: None, mode, ..self.args.clone() };
        match setup(&args, store) {
            Ok(setup) => {
                let run = std::mem::replace(&mut self.run, Run::new(setup));
                self.parked.get_or_insert(run);
                self.restart();
            }
            Err(e) => self.notice = Some(format!("practice: {e:#}")),
        }
    }

    /// Back to the run practice set aside, if it did.
    fn resume_run(&mut self) {
        if let Some(run) = self.parked.take() {
            self.run = run;
        }
    }

    /// The run whose text `c` and the reader go on in: the one set aside
    /// while practising.
    fn text(&self) -> &Run {
        self.parked.as_ref().unwrap_or(&self.run)
    }

    fn title(&self) -> String {
        let mut title = self.run.title.clone();
        if self.record.as_ref().map_or(self.incognito, Record::incognito) {
            title.push_str(" · incognito");
        }
        title
    }

    /// Advances the clock; returns to results once the session is over.
    fn tick(&mut self, now: Instant) {
        let Screen::Typing(session) = &mut self.screen else { return };
        let feed = &mut self.run.feed;
        session.tick(now);
        if session.limit().is_some() && session.remaining() < BATCH / 2 {
            session.extend(feed.take(BATCH));
        }
        if session.finished() {
            if feed.source.ordered() {
                feed.resume = Some(feed.start + session.current());
            }
            let cite = feed.cite().map(str::to_owned);
            let meta = Meta { cite, ..self.run.meta.clone() };
            let store = self.store.as_ref();
            let saved = save(store, session, now, &meta, self.incognito);
            (self.record, self.notice) = match saved {
                Ok(record) => (record, None),
                Err(e) => (None, Some(format!("not saved: {e:#}"))),
            };
            let report = Report::new(session, session.elapsed(now));
            self.screen = Screen::Results {
                report: Box::new(report),
                measure: Measure::default(),
                session: session.clone(),
                review: None,
                reader: None,
            };
        }
    }

    /// Flags the test on the results screen incognito in the history, or
    /// clears the flag.
    fn toggle_incognito(&mut self) {
        let (Some(store), Some(record)) = (&self.store, &mut self.record)
        else {
            return;
        };
        let mut flagged = record.clone();
        flagged.flags.set(INCOGNITO, !record.incognito());
        self.notice = match store.replace(record, &flagged) {
            Ok(true) => {
                *record = flagged;
                None
            }
            Ok(false) => Some("history: no longer holds this test".into()),
            Err(e) => Some(format!("history: {e:#}")),
        };
    }

    fn close_reader(&mut self) {
        if let Screen::Results { reader, .. } = &mut self.screen {
            *reader = None;
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
            Screen::Results { reader: Some(scroll), .. } => match key.code {
                KeyCode::Char('q') => return false,
                KeyCode::Char('O') | KeyCode::Esc => {
                    self.close_reader();
                }
                KeyCode::Down | KeyCode::Char('j') => *scroll = scroll.down(),
                KeyCode::Up | KeyCode::Char('k') => *scroll = scroll.up(),
                KeyCode::Enter => {
                    let passage = scroll.passage;
                    let feed = &self.text().feed;
                    let work = feed.source.work(feed.start);
                    if let Some(passage) = work.get(passage) {
                        self.resume_run();
                        self.begin(passage.start);
                    }
                }
                _ => {}
            },
            Screen::Results { measure, session, review, reader, .. } => {
                let text = self.parked.as_ref().unwrap_or(&self.run);
                match key.code {
                    KeyCode::Char('q') | KeyCode::Esc => return false,
                    KeyCode::Char('i') => self.toggle_incognito(),
                    KeyCode::Char('o') => {
                        *review = if review.is_some() { None } else { Some(0) };
                    }
                    KeyCode::Char('O') => {
                        let readable = !text.feed.reading().passages.is_empty();
                        *reader = readable.then(|| text.feed.opening());
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
                        self.resume_run();
                        self.restart();
                    }
                    KeyCode::Char('c') => {
                        if let Some(at) = text.feed.resume {
                            self.resume_run();
                            self.begin(at);
                        }
                    }
                    KeyCode::Char('p') => self.practise(),
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
        let cite = self.run.feed.cite();
        let text = self.parked.as_ref().unwrap_or(&self.run);
        match &mut self.screen {
            Screen::Typing(session) => {
                ui::typing::draw(frame, session, now, &title, cite)
            }
            Screen::Results { reader: Some(scroll), .. } => {
                let (reading, cite) = (text.feed.reading(), text.feed.cite());
                let title = &text.title;
                *scroll =
                    ui::reader::draw(frame, &reading, title, cite, *scroll);
            }
            Screen::Results { session, review: Some(row), .. } => {
                *row = ui::review::draw(frame, session, &title, cite, *row);
            }
            Screen::Results { report, measure, .. } => {
                let notice = self.notice.as_deref();
                let (layout, measure) = (self.run.layout, *measure);
                let keys = Keys {
                    resume: text.feed.resume.is_some(),
                    incognito: self.record.as_ref().map(Record::incognito),
                    read: !text.feed.reading().passages.is_empty(),
                };
                let title = match cite {
                    Some(cite) => format!("{title} · {cite}"),
                    None => title,
                };
                ui::results::draw(
                    frame, report, layout, measure, &title, notice, keys,
                )
            }
        }
    }
}

/// Records a finished session, unless nothing was typed, flagged
/// incognito if asked; the record made.
fn save(
    store: Option<&Store>,
    session: &Session,
    now: Instant,
    meta: &Meta,
    incognito: bool,
) -> Result<Option<Record>> {
    let Some(store) = store else { return Ok(None) };
    if session.log().is_empty() {
        return Ok(None);
    }
    let at = chrono::Local::now()
        .to_rfc3339_opts(chrono::SecondsFormat::Secs, false);
    let mut record = Record::new(session, now, at, meta.clone());
    record.flags.set(INCOGNITO, incognito);
    store.append(&record).map(|()| Some(record))
}

fn main() -> Result<()> {
    let cli =
        Cli::try_parse().unwrap_or_else(|err| words::with_letters(err).exit());
    let store = Store::open()?;
    match cli.cmd {
        Some(Cmd::Stats {
            last,
            source,
            layout,
            by,
            plot,
            incognito,
            detailed,
            geometry,
            practice,
        }) => {
            let view = View {
                by,
                plot,
                incognito,
                detailed,
                geometry,
                ..View::default()
            };
            stats(&store, last, source.as_deref(), layout, view)?;
            if practice {
                practice_summary(&store, last, source.as_deref())?;
            }
            Ok(())
        }
        Some(Cmd::Read(args)) => {
            let config = config(&oui::config::path()?, &store)?;
            read_app::run(args, config.language, store)
        }
        Some(Cmd::External { dirs }) => {
            let out = store.dir().join("external");
            let today = chrono::Local::now().format("%Y-%m-%d").to_string();
            let added = external::update(&dirs, &out, &today)?;
            let counts: Vec<String> = added
                .passages
                .iter()
                .map(|(language, n)| format!("{n} {language}"))
                .collect();
            match added.articles {
                0 => println!("no new articles for {}", out.display()),
                n => println!(
                    "{n} articles, {} passages into {}",
                    counts.join(", "),
                    out.display()
                ),
            }
            Ok(())
        }
        None => {
            let config = config(&oui::config::path()?, &store)?;
            let last = match cli.test.resume {
                true => Some(last_text(&cli.test, store.load()?.records)?),
                false => None,
            };
            let test = match &last {
                Some(last) => going_on(&cli.test, last),
                None => cli.test.clone(),
            };
            let setup = setup(&merge(&test, &config), &store)?;
            let start = last.map(|l| stopped(&setup.source, &l)).transpose()?;
            let mut app = App::new(setup, Some(store));
            if let Some(at) = start {
                app.begin(at);
            }
            ratatui::run(|terminal| run(terminal, app))
        }
    }
}

/// The config at `rc`; one left where oui used to read it is an error
/// rather than silently ignored.
fn config(rc: &Path, store: &Store) -> Result<Config> {
    let old = store.dir().join("config");
    if !rc.exists() && old.exists() {
        bail!(
            "oui reads its config from {} now: move {} there",
            rc.display(),
            old.display()
        );
    }
    Config::load(rc)
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
    let all = view.incognito;
    let records = select(history.records, last, source);
    let (records, left_out) = counted(records, all, Record::incognito);
    let reads = reads(store.load_reads()?.records, last, source);
    let (reads, reads_left) = counted(reads, all, ReadRecord::incognito);
    view.left_out = left_out;
    if records.is_empty() {
        if left_out > 0 {
            println!(
                "only incognito tests ({left_out}): --incognito counts them"
            );
        } else if reads.is_empty() && reads_left == 0 {
            let history = store.history();
            println!("no tests recorded in {}", history.display());
        }
        print!("{}", summary::reading(&reads, reads_left));
        return Ok(());
    }
    view.layout = layout.unwrap_or_else(|| {
        let words: Vec<String> =
            records.iter().flat_map(|r| r.words.iter().cloned()).collect();
        Layout::detect(&words)
    });
    view.colour = std::io::stdout().is_terminal();
    print!("{}", summary::render(&records, &view));
    print!("{}", summary::reading(&reads, reads_left));
    Ok(())
}

/// `records` but those `flagged` incognito, unless `all`; how many went.
fn counted<T>(
    mut records: Vec<T>,
    all: bool,
    flagged: fn(&T) -> bool,
) -> (Vec<T>, usize) {
    let before = records.len();
    if !all {
        records.retain(|r| !flagged(r));
    }
    let gone = before - records.len();
    (records, gone)
}

/// The reading tests of `source` (all if `None`), the last `last` of them.
fn reads(
    mut records: Vec<ReadRecord>,
    last: Option<NonZeroUsize>,
    source: Option<&str>,
) -> Vec<ReadRecord> {
    records.retain(|r| source.is_none_or(|s| r.source == s));
    if let Some(n) = last {
        records.drain(..records.len().saturating_sub(n.get()));
    }
    records
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
/// language, mode, extras or plain from the config, nor text mode extras.
fn merge(cli: &TestArgs, config: &Config) -> TestArgs {
    fn pick<T>(
        flag: Option<T>,
        set: Option<T>,
        from_config: bool,
    ) -> Option<T> {
        flag.or(set.filter(|_| from_config))
    }
    let from_config = cli.file.is_none();
    let mode = pick(cli.mode, config.mode, from_config);
    let plain = pick(cli.plain, config.plain, from_config);
    let with = match cli.with.is_empty() {
        false => cli.with.clone(),
        true if from_config && !written(mode, plain) => config.with.clone(),
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
        plain,
        words,
        time,
        layout: cli.layout.or(config.layout),
        incognito: cli.incognito,
        resume: cli.resume,
    }
}

/// The last test `--continue` goes on from: of FILE, else of text mode,
/// of the language, mode and plainness given.
fn last_text(cli: &TestArgs, records: Vec<Record>) -> Result<Record> {
    if let Some(mode) = cli.mode.filter(|m| !m.passages()) {
        bail!("--continue goes on in text mode or a FILE, not --mode {mode}");
    }
    let name = cli.file.as_deref().and_then(Path::file_name);
    let name = name.map(|n| n.to_string_lossy().into_owned());
    let mode = |r: &Record| <Mode as ValueEnum>::from_str(&r.meta.mode, false);
    let given = |r: &Record| match &name {
        Some(name) => r.meta.mode == "file" && r.meta.source == *name,
        None => {
            let language = cli.language.as_ref();
            mode(r).is_ok_and(Mode::passages)
                && cli.mode.is_none_or(|m| mode(r) == Ok(m))
                && language.is_none_or(|l| *l == r.meta.source)
        }
    };
    let plain = |r: &Record| cli.plain.is_none_or(|p| p == r.meta.plain);
    let last = records.into_iter().rev().find(|r| given(r) && plain(r));
    let what = name.unwrap_or_else(|| "text mode".into());
    last.ok_or_else(|| anyhow!("no test of {what} to continue"))
}

/// `cli` going on from `last`: in its mode, language and plainness,
/// unless given.
fn going_on(cli: &TestArgs, last: &Record) -> TestArgs {
    let mut args = cli.clone();
    args.plain = Some(cli.plain.unwrap_or(last.meta.plain));
    if cli.file.is_none() {
        let mode = <Mode as ValueEnum>::from_str(&last.meta.mode, false);
        args.mode = cli.mode.or(mode.ok());
        let language = cli.language.clone();
        args.language = language.or_else(|| Some(last.meta.source.clone()));
    }
    args
}

/// The word of `source` after the last one `last` submitted.
fn stopped(source: &Source, last: &Record) -> Result<usize> {
    let cite = last.meta.cite.as_deref();
    match source.locate(cite, &last.words) {
        Some(at) => Ok(at + last.typed.len()),
        None => bail!(
            "{} has changed since its last test: no telling where it stopped",
            cite.unwrap_or(&last.meta.source)
        ),
    }
}

/// Whether `mode` types prose as written, punctuation and all, which
/// takes no extras: passages always, funny mode unless it is plain.
fn written(mode: Option<Mode>, plain: Option<bool>) -> bool {
    match mode {
        Some(Mode::Funny) => plain != Some(true),
        mode => mode.is_some_and(Mode::passages),
    }
}

/// The run's settings, drawing on the passages and history in `store`.
fn setup(cli: &TestArgs, store: &Store) -> Result<Setup> {
    let (source, name, focus) = source(cli, store)?;
    let limit = limit(cli, &source);
    if matches!(source, Source::Passages { .. }) && !cli.with.is_empty() {
        bail!("--with mixes extras into random words, not passages");
    }
    if written(cli.mode, cli.plain) && !cli.with.is_empty() {
        bail!("--with mixes extras into bare words: add --plain to funny mode");
    }
    let mut extras = cli.with.clone();
    extras.sort();
    extras.dedup();
    let with: Vec<String> =
        extras.iter().map(|e| e.name().to_string()).collect();
    let mode = match source {
        Source::Text(_) => "file".to_string(),
        _ => cli.mode.unwrap_or(Mode::Words).to_string(),
    };
    let mut what = match mode.as_str() {
        "words" | "file" => name.clone(),
        _ => format!("{name} {mode}"),
    };
    if !with.is_empty() {
        what = format!("{what} with {}", with.join(", "));
    }
    let prose = matches!(
        source,
        Source::Text(_) | Source::Passages { .. } | Source::Funny(_)
    );
    let plain = prose && cli.plain == Some(true);
    if plain {
        what.push_str(" plain");
    }
    if !focus.is_empty() {
        let show =
            |&(a, b): &(char, char)| format!("{}{}", ui::show(a), ui::show(b));
        let focus: Vec<String> = focus.iter().map(show).collect();
        what = format!("{what} · {}", focus.join(" "));
    }
    let title = match limit {
        Limit::Words(n) => format!("{what} · {n} words"),
        Limit::Time(d) => format!("{what} · {}s", d.as_secs()),
        Limit::Passage => format!("{what} · one passage"),
    };
    let layout = cli.layout.unwrap_or_else(|| Layout::detect(source.words()));
    let meta = Meta {
        source: name,
        mode,
        with,
        plain,
        drill: false,
        layout: layout.name().to_string(),
        cite: None,
    };
    let (incognito, args) = (cli.incognito, cli.clone());
    Ok(Setup { source, limit, layout, title, meta, extras, incognito, args })
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
    let plain = cli.plain == Some(true);
    let prose = |passages| match plain {
        true => words::plain(passages),
        false => passages,
    };
    if let Some(path) = &cli.file {
        let text = fs::read_to_string(path)
            .with_context(|| format!("reading {}", path.display()))?;
        let mut words = words::split(&text);
        if plain {
            words = words.iter().filter_map(|w| words::bare(w)).collect();
        }
        if words.is_empty() {
            bail!("{} has no words to type", path.display());
        }
        let name = path.file_name().unwrap_or(path.as_os_str());
        let name = name.to_string_lossy().into_owned();
        return Ok((Source::Text(words), name, Vec::new()));
    }
    let language =
        cli.language.clone().unwrap_or_else(|| DEFAULT_LANGUAGE.to_string());
    if cli.mode == Some(Mode::Practice) {
        let records = select(store.load()?.records, None, Some(&language));
        let records = practised(records);
        let (targets, words) = practice_pool(&language, &records, store)?;
        let focus = targets.iter().take(FOCUS).map(|t| t.bigram).collect();
        return Ok((Source::Random(words), language, focus));
    }
    if cli.mode == Some(Mode::Text(Texts::Classics)) {
        let passages = prose(words::classics(&language));
        if passages.is_empty() {
            bail!("no classics for '{language}'");
        }
        return Ok((Source::passages(passages), language, Vec::new()));
    }
    let external = store.dir().join("external");
    if cli.mode == Some(Mode::Text(Texts::External)) {
        let passages = prose(words::external(&language, &external)?);
        if passages.is_empty() {
            bail!(
                "no external texts for '{language}' in {} (extract them \
                 with `oui external DIR`)",
                external.display()
            );
        }
        return Ok((Source::passages(passages), language, Vec::new()));
    }
    if let Some(mode @ (Mode::Text(_) | Mode::Funny | Mode::Ngrams(_))) =
        cli.mode
    {
        let texts = store.dir().join("texts");
        let mut passages = words::load_passages(&language, &texts)?;
        passages.extend(words::external(&language, &external)?);
        if mode.passages() || mode == Mode::Funny {
            passages = prose(passages);
        }
        if passages.is_empty() {
            let dir = texts.join(&language);
            bail!(
                "no texts for '{language}' (add .txt files to {})",
                dir.display()
            );
        }
        let source = match mode {
            Mode::Funny => Source::Funny(Chain::new(&passages)),
            Mode::Ngrams(n) => {
                let ngrams = Ngrams::new(&passages, n);
                if ngrams.is_empty() {
                    bail!("no {n} letters in a row in the '{language}' texts");
                }
                Source::Ngrams(ngrams)
            }
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

/// The `records` practice draws on: all but ngrams tests.
fn practised(records: Vec<Record>) -> Vec<Record> {
    records.into_iter().filter(|r| !of_ngrams(&r.meta)).collect()
}

/// Whether a test typed n-grams as words, so its pace is no baseline for
/// typing the language and its spaces fall where the language's never do.
fn of_ngrams(meta: &Meta) -> bool {
    matches!(Mode::from_str(&meta.mode, false), Ok(Mode::Ngrams(_)))
}

/// What practice targets in each language of the selected tests.
fn practice_summary(
    store: &Store,
    last: Option<NonZeroUsize>,
    source: Option<&str>,
) -> Result<()> {
    let records = practised(select(store.load()?.records, last, source));
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
        (None, Source::Random(_) | Source::Funny(_) | Source::Ngrams(_)) => {
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
            args: TestArgs::default(),
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
    fn funny_mode_walks_the_prose_as_written() {
        let args = cli(&["-m", "funnywords", "-l", "russian"]);
        let setup = setup(&args, &no_home()).unwrap();
        assert_eq!(setup.title, "russian funny · 50 words");
        assert!(!setup.meta.plain);
        let Source::Funny(chain) = &setup.source else { unreachable!() };
        let passages = words::classics("russian");
        let written: std::collections::HashSet<&String> =
            passages.iter().flat_map(|p| &p.words).collect();
        assert!(chain.words().iter().all(|w| written.contains(w)));
        let ends = |w: &&String| w.ends_with(['.', ',']);
        assert!(chain.words().iter().any(|w| ends(&w)), "punctuation stays");
        let app = App::new(setup, None);
        let Screen::Typing(session) = &app.screen else { unreachable!() };
        assert_eq!(session.words().len(), DEFAULT_WORDS);
        assert!(session.words()[0].starts_with(char::is_uppercase));

        let err = cli(&["-m", "funny", "-W", "p"]);
        let err = super::setup(&err, &no_home()).err().unwrap().to_string();
        assert!(err.starts_with("--with mixes extras into bare"), "{err}");
        let config = oui::config::parse("mode = funny\nwith = p").unwrap();
        assert!(merge(&cli(&[]), &config).with.is_empty());
        let plain = merge(&cli(&["-P"]), &config);
        assert_eq!(plain.with, [Extra::Punctuation]);
    }

    #[test]
    fn plain_funny_mode_walks_bare_words_and_takes_extras() {
        let args = cli(&["-m", "f", "-l", "russian", "-P", "-W", "p"]);
        let setup = setup(&args, &no_home()).unwrap();
        assert_eq!(
            setup.title,
            "russian funny with punctuation plain · 50 words"
        );
        assert!(setup.meta.plain);
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
    fn ngrams_mode_types_n_grams_as_words_and_takes_extras() {
        let args = cli(&["-m", "n2", "-l", "russian", "-W", "c"]);
        let setup = setup(&args, &no_home()).unwrap();
        assert_eq!(setup.title, "russian ngrams:2 with capitals · 50 words");
        assert_eq!(setup.meta.mode, "ngrams:2");
        assert_eq!(setup.layout, Layout::Jcuken);
        let app = App::new(setup, None);
        let Screen::Typing(session) = &app.screen else { unreachable!() };
        assert_eq!(session.words().len(), DEFAULT_WORDS);
        for word in session.words() {
            let letters = word.chars().filter(|c| c.is_alphabetic()).count();
            assert_eq!(letters, 2, "{word}");
        }
        let mode = |args: &TestArgs| {
            super::setup(args, &no_home()).map(|s| s.meta.mode)
        };
        assert_eq!(mode(&cli(&["--mode", "ngrams"])).unwrap(), "ngrams:1");
        let config = oui::config::parse("mode = ngrams:3").unwrap();
        assert_eq!(mode(&merge(&cli(&[]), &config)).unwrap(), "ngrams:3");
        let err = mode(&cli(&["-m", "n99"])).unwrap_err();
        assert_eq!(
            err.to_string(),
            "no 99 letters in a row in the 'english' texts"
        );
        assert!(Cli::try_parse_from(["oui", "-m", "n0"]).is_err());
        assert!(
            Cli::try_parse_from(["oui", "-m", "ngrams:2", "f.txt"]).is_err()
        );
    }

    #[test]
    fn text_mode_types_one_whole_passage() {
        let args = cli(&["-m", "text", "-l", "french"]);
        let setup = setup(&args, &no_home()).unwrap();
        assert_eq!(setup.title, "french text · one passage");
        assert_eq!(setup.meta.mode, "text");
        let app = App::new(setup, None);
        let Screen::Typing(session) = &app.screen else { unreachable!() };
        let len = app.run.feed.source.passage_len(app.run.feed.start);
        assert_eq!(Some(session.words().len()), len);
        assert!(session.words()[0].starts_with(char::is_uppercase));
        assert!(
            app.run.feed.cite().is_some_and(|c| c.contains(", ")),
            "bundled"
        );
    }

    #[test]
    fn plain_strips_prose_and_names_the_test() {
        let bare = |source: &Source| {
            let plain = |w: &String| words::bare(w).as_ref() == Some(w);
            source.words().iter().all(plain)
        };
        for mode in ["text", "text:classics"] {
            let args = cli(&["-m", mode, "-l", "french", "--plain"]);
            let setup = setup(&args, &no_home()).unwrap();
            assert_eq!(
                setup.title,
                format!("french {mode} plain · one passage")
            );
            assert!(setup.meta.plain && bare(&setup.source), "{mode}");
            let app = App::new(setup, None);
            let Screen::Typing(session) = &app.screen else { unreachable!() };
            let len = app.run.feed.source.passage_len(app.run.feed.start);
            assert_eq!(Some(session.words().len()), len);
            assert!(app.run.feed.cite().is_some(), "still cited");
        }
        let tmp = TempDir::new("plain");
        let store = Store::at(&tmp.0);
        fs::create_dir_all(tmp.0.join("external")).unwrap();
        let ext = "One, two!\n\n1815.\n\n\"Three\" - four.\n";
        fs::write(tmp.0.join("external/english.txt"), ext).unwrap();
        let ext = setup(&cli(&["-m", "t:e", "-P"]), &store).unwrap();
        assert_eq!(ext.title, "english text:external plain · one passage");
        let Source::Passages { words, starts, .. } = &ext.source else {
            unreachable!()
        };
        assert_eq!(words, &["one", "two", "three", "four"]);
        assert_eq!(starts, &[0, 2], "the passage of numbers is gone");
        fs::write(tmp.0.join("external/english.txt"), "1815.\n").unwrap();
        let err = setup(&cli(&["-m", "t:e", "-P"]), &store).err().unwrap();
        assert!(err.to_string().starts_with("no external texts"), "{err}");

        let file = tmp.0.join("notes.txt");
        fs::write(&file, "Hello, World - 42!").unwrap();
        let file = file.to_str().unwrap();
        let notes = setup(&cli(&[file, "--plain"]), &store).unwrap();
        assert_eq!(notes.title, "notes.txt plain · 2 words");
        assert_eq!(notes.source.words(), ["hello", "world"]);
        let raw = setup(&cli(&[file, "--plain=false"]), &store).unwrap();
        assert_eq!(raw.title, "notes.txt · 4 words");
        assert!(!raw.meta.plain);
    }

    #[test]
    fn plain_leaves_bare_modes_alone() {
        for mode in ["words", "n2"] {
            let plain = setup(&cli(&["-m", mode, "-P"]), &no_home()).unwrap();
            assert!(!plain.meta.plain && !plain.title.contains("plain"));
        }
    }

    #[test]
    fn plain_comes_from_the_config_unless_a_flag_or_file_says() {
        let config = oui::config::parse("mode = text:classics\nplain = true");
        let config = config.unwrap();
        assert_eq!(merge(&cli(&[]), &config).plain, Some(true));
        let off = merge(&cli(&["--plain=false"]), &config);
        assert_eq!(off.plain, Some(false), "flags win");
        assert_eq!(merge(&cli(&["notes.txt"]), &config).plain, None);
        assert_eq!(cli(&["-P", "notes.txt"]).plain, Some(true));
    }

    #[test]
    fn classics_mode_types_only_the_bundled_passages() {
        let tmp = TempDir::new("classics");
        let store = Store::at(&tmp.0);
        let mine = tmp.0.join("texts/french");
        fs::create_dir_all(&mine).unwrap();
        fs::write(mine.join("a.txt"), "[[Moi]]\nun deux").unwrap();
        let args = cli(&["-m", "text:classics", "-l", "french"]);
        let classics = setup(&args, &store).unwrap();
        assert_eq!(classics.title, "french text:classics · one passage");
        assert_eq!(classics.meta.mode, "text:classics");
        let Source::Passages { cites, .. } = &classics.source else {
            unreachable!()
        };
        assert_eq!(cites.len(), words::classics("french").len());
        assert!(cites.iter().all(|c| c.as_deref() != Some("Moi")));
        let err = |args: &[&str]| {
            setup(&cli(args), &store).err().unwrap().to_string()
        };
        assert!(err(&["-m", "t:c", "-W", "p"]).starts_with("--with"));
        assert!(err(&["-m", "t:c", "-l", "german"])
            .starts_with("no classics for 'german'"));
    }

    #[test]
    fn ext_mode_types_what_oui_external_wrote() {
        let tmp = TempDir::new("external");
        let store = Store::at(&tmp.0);
        let err = |args: &[&str]| {
            setup(&cli(args), &store).err().unwrap().to_string()
        };
        assert!(
            err(&["-m", "t:e"]).starts_with("no external texts for 'english'")
        );
        fs::create_dir_all(tmp.0.join("external")).unwrap();
        let ext = "[[blog, Post (2026)]]\none two three\n\nfour five\n";
        fs::write(tmp.0.join("external/english.txt"), ext).unwrap();
        let mine = tmp.0.join("texts/english");
        fs::create_dir_all(&mine).unwrap();
        fs::write(mine.join("a.txt"), "[[Moi]]\nsix seven").unwrap();

        let ext = setup(&cli(&["-m", "text:external"]), &store).unwrap();
        assert_eq!(ext.title, "english text:external · one passage");
        assert_eq!(ext.meta.mode, "text:external");
        let Source::Passages { cites, .. } = &ext.source else {
            unreachable!()
        };
        let post = Some("blog, Post (2026)".to_string());
        assert_eq!(cites, &vec![post; 2]);
        assert!(err(&["-m", "t:e", "-W", "p"]).starts_with("--with"));
        for old in ["feed", "ext", "classics"] {
            assert!(Cli::try_parse_from(["oui", "-m", old]).is_err());
        }

        let all = setup(&cli(&["-m", "text"]), &store).unwrap();
        let Source::Passages { cites, .. } = &all.source else {
            unreachable!()
        };
        let classics = words::classics("english").len();
        assert_eq!(cites.len(), classics + 3, "classics, texts and external");
        assert_eq!(cites[classics].as_deref(), Some("Moi"));
        assert_eq!(cites[classics + 2].as_deref(), Some("blog, Post (2026)"));
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
        assert!(merge(&cli(&["-m", "t:c"]), &config).with.is_empty());
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
        for needle in [
            "~/.ouirc",
            "history.jsonl",
            "texts/<language>/",
            "external/",
            "with  ",
            "plain  ",
        ] {
            assert!(help.contains(needle), "help lacks {needle:?}\n{help}");
        }
    }

    #[test]
    fn config_is_read_from_the_rc_and_not_the_old_place() {
        let tmp = TempDir::new("config");
        let store = Store::at(tmp.0.join("home"));
        let rc = tmp.0.join(".ouirc");
        assert_eq!(config(&rc, &store).unwrap(), Config::default());
        fs::create_dir_all(store.dir()).unwrap();
        fs::write(&rc, "words = 7").unwrap();
        let words = |c: Config| c.words.map(NonZeroUsize::get);
        assert_eq!(words(config(&rc, &store).unwrap()), Some(7));
        fs::write(store.dir().join("config"), "words = 9").unwrap();
        assert_eq!(words(config(&rc, &store).unwrap()), Some(7));
        fs::remove_file(&rc).unwrap();
        let err = config(&rc, &store).unwrap_err().to_string();
        assert!(err.starts_with("oui reads its config from "), "{err}");
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
                plot: Plot::Steps,
                incognito: false,
                detailed: false,
                geometry: false,
                practice: false,
            })
        );
        let all = parse(&["stats", "--incognito"]).unwrap().cmd;
        assert!(matches!(all, Some(Cmd::Stats { incognito: true, .. })));
        let ribbon = parse(&["stats", "--plot", "ribbon"]).unwrap().cmd;
        assert!(matches!(ribbon, Some(Cmd::Stats { plot: Plot::Ribbon, .. })));
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
        let ext = parse(&["external", "a", "b"]).unwrap().cmd;
        let dirs = vec![PathBuf::from("a"), PathBuf::from("b")];
        assert_eq!(ext, Some(Cmd::External { dirs }));
        assert_eq!(parse(&["ext", "a", "b"]).unwrap().cmd, ext);
        assert!(parse(&["external"]).is_err(), "a directory is required");
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
        assert_eq!(args.mode, Some(Mode::Practice), "not the config's");
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
        assert!(matches!(app.screen, Screen::Typing(_)), "practice goes on");
        assert_eq!(app.run.meta.mode, "practice");
    }

    #[test]
    fn practice_leaves_ngrams_tests_out() {
        let tmp = TempDir::new("ngrams");
        let store = slow_qu(&tmp);
        let as_ngrams = |record: Record| Record {
            meta: Meta { mode: "ngrams:2".into(), ..record.meta },
            ..record
        };
        let records = store.load().unwrap().records;
        fs::remove_file(store.history()).unwrap();
        store.append(&as_ngrams(records[0].clone())).unwrap();
        let err = setup(&cli(&["-p"]), &store).err().unwrap().to_string();
        assert!(err.starts_with("no english tests to practise on"), "{err}");
        store.append(&records[0]).unwrap();
        assert!(setup(&cli(&["-p"]), &store).is_ok());
    }

    #[test]
    fn practice_needs_history_and_takes_no_file_or_mode() {
        let parse =
            |args: &[&str]| Cli::try_parse_from([&["oui"][..], args].concat());
        assert!(parse(&["-p", "notes.txt"]).is_err());
        assert!(parse(&["-p", "-m", "text"]).is_err());
        assert!(parse(&["-m", "p", "notes.txt"]).is_err());
        assert!(parse(&["-p", "-l", "russian", "-t", "30"]).is_ok());
        let err = |args: &[&str]| {
            setup(&cli(args), &no_home()).err().unwrap().to_string()
        };
        assert_eq!(err(&["-p"]), err(&["--mode", "practice"]));
        let config = oui::config::parse("mode = p").unwrap();
        let merged = merge(&cli(&[]), &config);
        let from_config = setup(&merged, &no_home()).err().unwrap();
        assert_eq!(err(&["-p"]), from_config.to_string());
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
    fn passages_are_cited() {
        let passage = words::Passage {
            words: words::split("ab cd"),
            cite: Some("X, Y (1900)".into()),
        };
        let source = Source::passages(vec![passage]);
        let app = App::new(test_setup(source, Limit::Passage), None);
        assert_eq!(app.run.feed.cite(), Some("X, Y (1900)"));
        assert_eq!(self::app("ab", Limit::Words(1)).run.feed.cite(), None);
    }

    #[test]
    fn c_continues_the_text_where_the_last_test_stopped() {
        let passage = |text: &str, cite: &str| words::Passage {
            words: words::split(text),
            cite: Some(cite.into()),
        };
        let source = Source::passages(vec![
            passage("ab cd", "X"),
            passage("ef gh ij", "X"),
            passage("kl", "Y"),
        ]);
        let tmp = TempDir::new("continue");
        let store = Some(slow_qu(&tmp));
        let mut app = App::new(test_setup(source, Limit::Passage), store);
        let typing = |app: &App| {
            let Screen::Typing(session) = &app.screen else { unreachable!() };
            session.words().to_vec()
        };
        let type_out = |app: &mut App, keys: &str| {
            for c in keys.chars() {
                press(app, KeyCode::Char(c));
            }
        };
        app.begin(0);
        type_out(&mut app, "ab cd");
        assert!(matches!(app.screen, Screen::Results { .. }));
        press(&mut app, KeyCode::Char('c'));
        assert_eq!(typing(&app), ["ef", "gh", "ij"], "the next passage");
        assert_eq!(app.run.feed.cite(), Some("X"));

        type_out(&mut app, "ef ");
        press(&mut app, KeyCode::Esc);
        press(&mut app, KeyCode::Char('c'));
        assert_eq!(typing(&app), ["gh", "ij"], "the rest of a stopped one");
        assert_eq!(app.run.feed.cite(), Some("X"));

        type_out(&mut app, "gh ij");
        press(&mut app, KeyCode::Char('p'));
        assert_eq!(app.run.meta.mode, "practice");
        type_out(&mut app, "x");
        press(&mut app, KeyCode::Esc);
        press(&mut app, KeyCode::Char('c'));
        assert!(app.parked.is_none(), "practice is left behind");
        assert_eq!(typing(&app), ["kl"]);
        assert_eq!(app.run.feed.cite(), Some("Y"));
        type_out(&mut app, "kl");
        press(&mut app, KeyCode::Char('c'));
        assert_eq!(typing(&app), ["ab", "cd"], "wrapping to the first");
    }

    /// The record of a test of `words`, `keys` typed, as `meta` says.
    fn typed(words: &str, keys: &str, meta: Meta) -> Record {
        let mut session = Session::new(words::split(words), None);
        let t0 = Instant::now();
        for c in keys.chars() {
            session.apply(Input::Char(c), t0);
        }
        Record::new(&session, t0, String::new(), meta)
    }

    fn meta(source: &str, mode: &str, cite: Option<&str>) -> Meta {
        let (source, mode) = (source.into(), mode.into());
        Meta { source, mode, cite: cite.map(Into::into), ..Meta::default() }
    }

    #[test]
    fn continue_picks_the_last_test_of_the_text_given() {
        let classics = meta("english", "text:classics", Some("X"));
        let plain = Meta { plain: true, ..meta("french", "text", None) };
        let records = vec![
            typed("ab cd", "ab ", classics),
            typed("ef", "e", meta("notes.txt", "file", None)),
            typed("gh", "g", plain),
            typed("ij", "i", meta("english", "words", None)),
        ];
        let last = |args: &[&str]| {
            let found = last_text(&cli(args), records.clone());
            found.map(|r| r.words[0].clone()).map_err(|e| e.to_string())
        };
        assert_eq!(last(&["-c"]), Ok("gh".into()));
        assert_eq!(last(&["-c", "-l", "english"]), Ok("ab".into()));
        assert_eq!(last(&["-c", "-m", "t:c"]), Ok("ab".into()));
        assert_eq!(last(&["-c", "--plain=false"]), Ok("ab".into()));
        assert_eq!(last(&["-c", "dir/notes.txt"]), Ok("ef".into()));
        assert_eq!(
            last(&["-c", "-m", "t:e"]),
            Err("no test of text mode to continue".into())
        );
        assert_eq!(
            last(&["-c", "other.txt"]),
            Err("no test of other.txt to continue".into())
        );
        assert!(last(&["-c", "-m", "words"]).is_err());
        let parsed = Cli::try_parse_from(["oui", "-c", "-p"]);
        assert!(parsed.is_err(), "practice has nothing to go on in");
    }

    #[test]
    fn continue_types_as_the_last_test_did_unless_told_otherwise() {
        let last = typed("ab", "a", meta("french", "text:classics", None));
        let last = Record { meta: Meta { plain: true, ..last.meta }, ..last };
        let args = going_on(&cli(&["-c"]), &last);
        assert_eq!(args.mode, Some(Mode::Text(Texts::Classics)));
        assert_eq!(args.language.as_deref(), Some("french"));
        assert_eq!(args.plain, Some(true));
        let args = going_on(&cli(&["-c", "-m", "t", "--plain=false"]), &last);
        assert_eq!(args.mode, Some(Mode::Text(Texts::All)));
        assert_eq!(args.plain, Some(false));
        let args = going_on(&cli(&["-c", "notes.txt"]), &last);
        assert_eq!((args.mode, args.language), (None, None));
    }

    #[test]
    fn continue_starts_after_the_last_word_submitted() {
        let passage = |text: &str, cite: &str| words::Passage {
            words: words::split(text),
            cite: Some(cite.into()),
        };
        let source = Source::passages(vec![
            passage("ab cd", "X"),
            passage("ef ab cd ef", "Y"),
        ]);
        let stopped_at = |keys, cite| {
            let last = typed("ab cd ef", keys, meta("english", "text", cite));
            stopped(&source, &last).map_err(|e| e.to_string())
        };
        assert_eq!(stopped_at("ab ", Some("Y")), Ok(4), "on cd");
        assert_eq!(stopped_at("ab c", Some("Y")), Ok(4), "cd again");
        assert_eq!(stopped_at("ab cd ", Some("X")), Ok(2), "into Y");
        assert_eq!(
            stopped_at("ab ", Some("Z")),
            Err("Z has changed since its last test: no telling where it \
                 stopped"
                .into())
        );
    }

    #[test]
    fn shift_o_reads_the_work_and_enter_types_from_the_marked_passage() {
        let passage = |text: &str, cite: &str| words::Passage {
            words: words::split(text),
            cite: Some(cite.into()),
        };
        let source = Source::passages(vec![
            passage("zz", "W"),
            passage("ab cd", "X"),
            passage("ef gh ij", "X"),
            passage("kl", "X"),
            passage("mn", "Y"),
        ]);
        let tmp = TempDir::new("reader");
        let store = Some(slow_qu(&tmp));
        let mut app = App::new(test_setup(source, Limit::Passage), store);
        let type_out = |app: &mut App, keys: &str| {
            for c in keys.chars() {
                press(app, KeyCode::Char(c));
            }
        };
        let reader = |app: &App| match &app.screen {
            Screen::Results { reader, .. } => *reader,
            Screen::Typing(_) => None,
        };
        let at = |passage| Some(Scroll { passage, row: 0 });
        app.begin(1);
        type_out(&mut app, "ab cd");
        press(&mut app, KeyCode::Char('O'));
        assert_eq!(reader(&app), at(1), "opens on the next passage");
        let reading = app.run.feed.reading();
        let texts: Vec<String> =
            reading.passages.iter().map(|p| p.join(" ")).collect();
        assert_eq!(texts, ["ab cd", "ef gh ij", "kl"], "the work cited X");
        assert_eq!(reading.place, 1, "the first is behind");

        use ratatui::backend;
        let mut terminal =
            ratatui::Terminal::new(backend::TestBackend::new(60, 12)).unwrap();
        for _ in 0..2 {
            press(&mut app, KeyCode::Down);
            terminal.draw(|f| app.draw(f, Instant::now())).unwrap();
        }
        assert_eq!(reader(&app), at(2), "a row and a blank further");
        assert!(press(&mut app, KeyCode::Esc), "esc closes, not quits");
        assert_eq!(reader(&app), None);
        press(&mut app, KeyCode::Char('O'));
        press(&mut app, KeyCode::Char('k'));
        terminal.draw(|f| app.draw(f, Instant::now())).unwrap();
        press(&mut app, KeyCode::Enter);
        let Screen::Typing(session) = &app.screen else { unreachable!() };
        assert_eq!(session.words(), ["ab", "cd"], "back on the first");

        type_out(&mut app, "ab cd");
        press(&mut app, KeyCode::Char('c'));
        type_out(&mut app, "ef gh ij");
        press(&mut app, KeyCode::Char('c'));
        type_out(&mut app, "kl");
        assert_eq!(app.run.feed.reading().place, 3, "all of it is behind");
        press(&mut app, KeyCode::Char('p'));
        type_out(&mut app, "x");
        press(&mut app, KeyCode::Esc);
        press(&mut app, KeyCode::Char('O'));
        assert_eq!(reader(&app), at(2), "practice keeps the work, on kl");
        press(&mut app, KeyCode::Down);
        press(&mut app, KeyCode::Up);
        press(&mut app, KeyCode::Enter);
        assert!(app.parked.is_none(), "back in the text");
        let Screen::Typing(session) = &app.screen else { unreachable!() };
        assert_eq!(session.words(), ["kl"]);
    }

    #[test]
    fn shift_o_reads_a_file_but_not_random_words() {
        let mut app = app("ab cd ef", Limit::Words(2));
        for c in "ab cd".chars() {
            press(&mut app, KeyCode::Char(c));
        }
        let reading = app.run.feed.reading();
        assert_eq!(reading.passages, [["ab", "cd", "ef"]]);
        assert_eq!(reading.place, 0, "typing stopped inside it");

        let random = Source::Random(words::split("ab"));
        let mut app = App::new(test_setup(random, Limit::Words(1)), None);
        for c in "ab".chars() {
            press(&mut app, KeyCode::Char(c));
        }
        press(&mut app, KeyCode::Char('O'));
        let Screen::Results { reader, .. } = &app.screen else {
            unreachable!()
        };
        assert_eq!(*reader, None);
    }

    #[test]
    fn c_goes_on_in_a_file_and_does_nothing_for_random_words() {
        let mut app = app("ab cd ef", Limit::Words(2));
        for c in "ab cd".chars() {
            press(&mut app, KeyCode::Char(c));
        }
        press(&mut app, KeyCode::Char('c'));
        let Screen::Typing(session) = &app.screen else { unreachable!() };
        assert_eq!(session.words(), ["ef", "ab"]);

        let random = Source::Random(words::split("ab"));
        let mut app = App::new(test_setup(random, Limit::Words(1)), None);
        for c in "ab".chars() {
            press(&mut app, KeyCode::Char(c));
        }
        press(&mut app, KeyCode::Char('c'));
        assert!(matches!(app.screen, Screen::Results { .. }));
        assert_eq!(app.run.feed.resume, None);
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
    fn p_practises_as_practice_mode_does_until_r() {
        let tmp = TempDir::new("p");
        let store = slow_qu(&tmp);
        let mut app = app("ab cd", Limit::Words(1));
        let type_out = |app: &mut App, keys: &str| {
            for c in keys.chars() {
                press(app, KeyCode::Char(c));
            }
        };
        type_out(&mut app, "ab");
        press(&mut app, KeyCode::Char('p'));
        assert!(matches!(app.screen, Screen::Results { .. }), "no history");
        app.store = Some(Store::at(tmp.0.join("empty")));
        press(&mut app, KeyCode::Char('p'));
        let notice = app.notice.clone().unwrap_or_default();
        assert!(notice.starts_with("practice: no english tests"), "{notice}");

        app.store = Some(store.clone());
        press(&mut app, KeyCode::Char('p'));
        let practice = setup(&cli(&["-p"]), &store).unwrap();
        assert_eq!(app.title(), practice.title, "as `oui -p` names it");
        assert_eq!(app.title(), "english practice · qu · 50 words");
        assert!(app.notice.is_none());
        let words = |app: &App| {
            let Screen::Typing(session) = &app.screen else { unreachable!() };
            session.words().to_vec()
        };
        assert_eq!(words(&app).len(), DEFAULT_WORDS);
        assert!(words(&app).iter().all(|w| w.contains("qu")));
        press(&mut app, KeyCode::Tab);
        assert!(words(&app).iter().all(|w| w.contains("qu")), "tab stays");

        type_out(&mut app, "x");
        press(&mut app, KeyCode::Esc);
        let records = store.load().unwrap().records;
        let last = &records.last().unwrap().meta;
        assert_eq!((last.mode.as_str(), last.drill), ("practice", false));
        assert_eq!(last.source, "english");
        press(&mut app, KeyCode::Char('p'));
        assert!(words(&app).iter().all(|w| w.contains("qu")), "p again");
        type_out(&mut app, "x");
        press(&mut app, KeyCode::Esc);
        press(&mut app, KeyCode::Char('r'));
        assert_eq!(app.title(), "t", "r is the test it was launched for");
        assert!(app.parked.is_none());
        assert_eq!(words(&app).len(), 1);
    }

    #[test]
    fn incognito_records_flagged_and_still_practises_on_history() {
        let tmp = TempDir::new("incognito");
        let store = slow_qu(&tmp);

        let args = cli(&["-i", "-w", "9"]);
        assert!(args.incognito);
        assert!(cli(&["--inkognito"]).incognito);
        let mut setup = test_setup(words("ab cd"), Limit::Words(2));
        setup.meta.source = "t".into();
        setup.incognito = true;
        let mut app = App::new(setup, Some(store.clone()));
        assert_eq!(app.title(), "t · incognito", "while typing too");
        for c in "ab cd".chars() {
            press(&mut app, KeyCode::Char(c));
        }
        assert!(matches!(app.screen, Screen::Results { .. }));
        assert!(app.notice.is_none());
        let records = store.load().unwrap().records;
        assert_eq!(records.len(), 2);
        assert!(!records[0].incognito() && records[1].incognito());
        press(&mut app, KeyCode::Char('p'));
        assert_eq!(app.run.meta.mode, "practice");
        press(&mut app, KeyCode::Char('x'));
        press(&mut app, KeyCode::Esc);
        let records = store.load().unwrap().records;
        assert!(records[2].incognito(), "practice too");
    }

    #[test]
    fn counted_leaves_out_the_flagged_unless_all() {
        let flagged = |n: &u8| *n > 1;
        assert_eq!(counted(vec![1, 2, 3, 1], false, flagged), (vec![1, 1], 2));
        assert_eq!(counted(vec![1, 2], true, flagged), (vec![1, 2], 0));
        assert_eq!(counted(Vec::new(), false, flagged), (vec![], 0));
    }

    #[test]
    fn i_flags_the_shown_test_incognito_and_back() {
        let tmp = TempDir::new("after");
        let store = Store::at(&tmp.0);
        let recorded = || store.load().unwrap().records;
        let flagged = || -> Vec<bool> {
            recorded().iter().map(Record::incognito).collect()
        };
        let setup = test_setup(words("ab cd"), Limit::Words(1));
        let mut app = App::new(setup, Some(store.clone()));
        let type_out = |app: &mut App| {
            for c in "ab".chars() {
                press(app, KeyCode::Char(c));
            }
        };
        type_out(&mut app);
        press(&mut app, KeyCode::Char('r'));
        type_out(&mut app);
        let both = recorded();
        assert_eq!(flagged(), vec![false, false]);

        press(&mut app, KeyCode::Char('i'));
        assert_eq!(flagged(), vec![false, true], "nothing deleted");
        assert_eq!(app.title(), "t · incognito");
        assert!(app.notice.is_none());
        press(&mut app, KeyCode::Char('i'));
        assert_eq!(recorded(), both, "back as it was");
        assert_eq!(app.title(), "t");

        press(&mut app, KeyCode::Char('i'));
        press(&mut app, KeyCode::Char('r'));
        assert_eq!(app.title(), "t", "a new test counts again");
        type_out(&mut app);
        assert_eq!(flagged(), vec![false, true, false]);

        let mut setup = test_setup(words("ab"), Limit::Words(1));
        setup.incognito = true;
        let mut app = App::new(setup, Some(store.clone()));
        type_out(&mut app);
        assert_eq!(flagged(), vec![false, true, false, true]);
        press(&mut app, KeyCode::Char('i'));
        assert_eq!(flagged(), vec![false, true, false, false]);
        assert_eq!(app.title(), "t");

        fs::remove_file(store.history()).unwrap();
        press(&mut app, KeyCode::Char('i'));
        assert!(app.notice.is_some(), "a test gone from the history");
        assert!(!app.record.as_ref().unwrap().incognito());
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
