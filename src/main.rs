//! oui — the binary: CLI, terminal and the event loop over the two screens.

use std::fs;
use std::num::{NonZeroU64, NonZeroUsize};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};
use clap::Parser;
use fastrand::Rng;
use ratatui::crossterm::event::{
    self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers,
};
use ratatui::{DefaultTerminal, Frame};

use oui::keyboard::{Layout, Measure};
use oui::session::{Input, Session};
use oui::stats::Report;
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

/// Typing test that shows which keys and bigrams slow you down.
#[derive(Parser, Debug)]
#[command(name = "oui", version, about)]
struct Cli {
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

struct App {
    feed: Feed,
    limit: Limit,
    layout: Layout,
    title: String,
    screen: Screen,
}

impl App {
    fn new(
        source: Source,
        limit: Limit,
        layout: Layout,
        title: String,
    ) -> Self {
        let mut app = App {
            feed: Feed { source, drill: Vec::new(), drawn: 0, rng: Rng::new() },
            limit,
            layout,
            title,
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
            let report = Report::new(session, session.elapsed(now));
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
            Screen::Results { report, measure } => match key.code {
                KeyCode::Char('q') | KeyCode::Esc => return false,
                KeyCode::Char('r') => {
                    self.feed.drill.clear();
                    self.restart();
                }
                KeyCode::Char('p') => {
                    self.feed.drill = report.weak_bigrams(DRILL);
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
                ui::results::draw(frame, report, self.layout, *measure, &title)
            }
        }
    }
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let (source, name) = source(&cli)?;
    let limit = limit(&cli, &source);
    let title = match limit {
        Limit::Words(n) => format!("{name} · {n} words"),
        Limit::Time(d) => format!("{name} · {}s", d.as_secs()),
    };
    let layout = cli.layout.unwrap_or_else(|| Layout::detect(source.words()));
    let app = App::new(source, limit, layout, title);
    ratatui::run(|terminal| run(terminal, app))
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
fn source(cli: &Cli) -> Result<(Source, String)> {
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

fn limit(cli: &Cli, source: &Source) -> Limit {
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

    fn cli(args: &[&str]) -> Cli {
        Cli::try_parse_from([&["oui"][..], args].concat()).unwrap()
    }

    fn press(app: &mut App, code: KeyCode) -> bool {
        app.key(KeyEvent::new(code, KeyModifiers::NONE), Instant::now())
    }

    fn words(text: &str) -> Source {
        Source::Text(words::split(text))
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
            .is_ok_and(|c| c.layout == Some(Layout::Jcuken)));
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
        let mut app = App::new(
            words("ab cd"),
            Limit::Words(2),
            Layout::Qwerty,
            "t".into(),
        );
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
        let new = || {
            App::new(words("ab"), Limit::Words(1), Layout::Qwerty, "t".into())
        };
        assert!(!press(&mut new(), KeyCode::Esc));
        let mut app = new();
        press(&mut app, KeyCode::Char('a'));
        assert!(press(&mut app, KeyCode::Esc));
        assert!(matches!(app.screen, Screen::Results { .. }));
        assert!(!press(&mut app, KeyCode::Char('q')));
    }

    #[test]
    fn timed_tests_top_up_words_and_texts_continue() {
        let text = words("a b c");
        let limit = Limit::Time(Duration::from_secs(60));
        let mut app = App::new(text, limit, Layout::Qwerty, "t".into());
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
        let text = words("ab cd");
        let mut app =
            App::new(text, Limit::Words(2), Layout::Qwerty, "t".into());
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
    fn tab_toggles_the_heatmap_measure() {
        let mut app =
            App::new(words("a"), Limit::Words(1), Layout::Qwerty, "t".into());
        press(&mut app, KeyCode::Char('a'));
        press(&mut app, KeyCode::Tab);
        let Screen::Results { measure, .. } = app.screen else {
            unreachable!()
        };
        assert_eq!(measure, Measure::Speed);
    }
}
