//! `oui read`: a text read silently against the clock, then probes telling
//! whether it was taken in. The test is a state machine over plain data;
//! the pages it turns are laid out by whoever draws it.

use std::num::NonZeroU32;
use std::ops::Range;
use std::time::{Duration, Instant};

use crate::external;
use crate::probes::Probe;
use crate::reads::{self, Asked, ReadRecord, Turn, View};
use crate::sentences;
use crate::stats;
use crate::words::{self, Passage};

/// Words a test reads unless told otherwise.
pub const WORDS: usize = 400;
/// Probes asked after the text.
pub const PROBES: usize = 10;

/// Characters a blinking word stays for at least, however short.
const SHORT: usize = 4;
/// Characters longer it stays before a comma, and twice that before a
/// full stop.
const PAUSE: usize = 3;

/// A text to read, as paragraphs of words.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Text {
    pub paragraphs: Vec<Vec<String>>,
    pub cite: Option<String>,
    /// Running prose, rather than excerpts of a work.
    pub continuous: bool,
}

impl Text {
    /// The prose of a file: of markdown if `markdown`, cited by its front
    /// matter if it is an archive's article, else blocks between blank
    /// lines.
    pub fn file(text: &str, markdown: bool) -> Self {
        let (paragraphs, cite) = match markdown {
            true => {
                let (cite, body) = match external::article(text) {
                    Some((article, body)) => (Some(article.cite()), body),
                    None => (None, text),
                };
                let cite = cite.map(|c| c[2..c.len() - 2].to_string());
                let prose = external::prose(body);
                (prose.iter().map(|p| words::split(p)).collect(), cite)
            }
            false => {
                let passages = words::passages(text);
                let cite = passages.first().and_then(|p| p.cite.clone());
                (passages.into_iter().map(|p| p.words).collect(), cite)
            }
        };
        Text { paragraphs, cite, continuous: true }
    }

    /// The cited works among `passages`, each the excerpts under one
    /// citation.
    pub fn works(passages: Vec<Passage>) -> Vec<Self> {
        let cites = passages.iter().map(|p| p.cite.as_deref());
        let works = words::works(cites);
        let mut passages = passages.into_iter();
        let text = |work: Range<usize>| {
            let of: Vec<Passage> = passages.by_ref().take(work.len()).collect();
            let cite = of.first()?.cite.clone()?;
            let paragraphs = of.into_iter().map(|p| p.words).collect();
            Some(Text { paragraphs, cite: Some(cite), continuous: false })
        };
        works.into_iter().filter_map(text).collect()
    }

    pub fn words(&self) -> usize {
        self.paragraphs.iter().map(Vec::len).sum()
    }

    /// The paragraph starting at word `start`, if one does.
    pub fn paragraph_at(&self, start: usize) -> Option<usize> {
        let mut at = 0;
        let starts = self.paragraphs.iter().map(|p| {
            at += p.len();
            at - p.len()
        });
        starts.take_while(|&s| s <= start).position(|s| s == start)
    }

    /// The paragraphs a test reads from paragraph `from`: enough for
    /// `words` words, or fewer so that as many sentences are left unread
    /// as the `probes` may need for foils. `None` if no stretch gives
    /// that many to ask on either side.
    pub fn stretch(
        &self,
        from: usize,
        words: usize,
        probes: usize,
    ) -> Option<Range<usize>> {
        let asked = |p: &Vec<String>| {
            let sentences = sentences::split(p).into_iter();
            sentences.filter(|s| sentences::eligible(&p[s.clone()])).count()
        };
        let asked: Vec<usize> = self.paragraphs.iter().map(asked).collect();
        let sum = |range: Range<usize>| asked[range].iter().sum::<usize>();
        let last = self.paragraphs.len();
        let fits =
            |&end: &usize| sum(from..end) >= probes && sum(end..last) >= probes;
        let ends: Vec<usize> = (from + 1..=last).filter(fits).collect();
        let mut read = 0;
        let enough = |&&end: &&usize| {
            read = self.paragraphs[from..end].iter().map(Vec::len).sum();
            read >= words
        };
        let end = ends.iter().find(enough).or(ends.last())?;
        Some(from..*end)
    }
}

/// What the reader can do: turn the page either way, or answer a probe.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Input {
    Next,
    Back,
    Yes,
    No,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    /// The text is hidden until the clock starts.
    Ready,
    Reading,
    Probing,
    Done,
}

/// Where a test's text is from, for its record.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Origin {
    pub source: String,
    pub cite: Option<String>,
    pub file: Option<String>,
    pub continuous: bool,
    /// The first word read, among the `total` of the text.
    pub start: usize,
    pub total: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadTest {
    paragraphs: Vec<Vec<String>>,
    probes: Vec<Probe>,
    /// The words each page shows, in order and covering them all.
    pages: Vec<Range<usize>>,
    stage: Stage,
    /// The first word of the page shown.
    top: usize,
    /// When the page or probe shown appeared.
    since: Option<Instant>,
    turns: Vec<Turn>,
    answers: Vec<(bool, u64)>,
    /// The WPM the text goes past at and how, rather than being paged
    /// through.
    pace: Option<(u32, View)>,
}

impl ReadTest {
    pub fn new(paragraphs: Vec<Vec<String>>, probes: Vec<Probe>) -> Self {
        let words = paragraphs.iter().map(Vec::len).sum();
        ReadTest {
            paragraphs,
            probes,
            pages: std::iter::once(0..words).collect(),
            stage: Stage::Ready,
            top: 0,
            since: None,
            turns: Vec::new(),
            answers: Vec::new(),
            pace: None,
        }
    }

    /// The test with its text going past at `wpm`, five characters a
    /// word, for the reader to keep up with.
    pub fn paced(self, wpm: NonZeroU32, view: View) -> Self {
        ReadTest { pace: Some((wpm.get(), view)), ..self }
    }

    pub fn pace(&self) -> Option<u32> {
        self.pace.map(|(wpm, _)| wpm)
    }

    pub fn view(&self) -> Option<View> {
        self.pace.map(|(_, view)| view)
    }

    /// Characters of the text, a space after each word included.
    pub fn chars(&self) -> usize {
        let words = self.paragraphs.iter().flatten();
        words.map(|w| w.chars().count() + 1).sum()
    }

    /// Milliseconds a character takes to slide past at `pace`.
    fn per_char(pace: u32) -> f64 {
        60_000.0 / (f64::from(pace) * stats::CHARS_PER_WORD)
    }

    /// How long the whole text takes to go past, if it does.
    pub fn slide(&self) -> Option<Duration> {
        let ms = Self::per_char(self.pace()?) * self.chars() as f64;
        Some(Duration::from_millis(ms.round() as u64))
    }

    /// Characters of a sliding text gone past by `now`.
    pub fn slid(&self, now: Instant) -> usize {
        let Some(pace) = self.pace().filter(|_| self.stage == Stage::Reading)
        else {
            return 0;
        };
        let slid = self.shown(now) as f64 / Self::per_char(pace);
        (slid as usize).min(self.chars())
    }

    /// The word a blinking text shows at `now`. Words share the time the
    /// text would take to slide past, each by its [`dwell`].
    pub fn blinked(&self, now: Instant) -> usize {
        let (Some(slide), Stage::Reading) = (self.slide(), self.stage) else {
            return 0;
        };
        let dwells = self.paragraphs.iter().flatten().map(|w| dwell(w));
        let dwells: Vec<usize> = dwells.collect();
        let whole: usize = dwells.iter().sum();
        let gone = self.shown(now) as f64 / slide.as_millis() as f64;
        let mut end = 0;
        let shown = |dwell: &usize| {
            end += dwell;
            gone * (whole as f64) < end as f64
        };
        let last = dwells.len().saturating_sub(1);
        dwells.iter().position(shown).unwrap_or(last)
    }

    /// Advances the clock: a sliding text ends once all of it has gone
    /// past, at the pace set whatever the frame it is noticed on.
    pub fn tick(&mut self, now: Instant) {
        let Some(slide) = self.slide() else { return };
        let ms = u64::try_from(slide.as_millis()).unwrap_or(u64::MAX);
        if self.stage == Stage::Reading && self.shown(now) >= ms {
            self.turns.push(Turn(0, self.words(), ms));
            self.since = Some(now);
            self.close();
        }
    }

    /// Ends the reading: on to the probes, if there are any.
    fn close(&mut self) {
        self.stage = match self.probes.is_empty() {
            true => Stage::Done,
            false => Stage::Probing,
        };
    }

    pub fn paragraphs(&self) -> &[Vec<String>] {
        &self.paragraphs
    }

    pub fn words(&self) -> usize {
        self.paragraphs.iter().map(Vec::len).sum()
    }

    pub fn stage(&self) -> Stage {
        self.stage
    }

    /// The page shown, by its place among the pages.
    pub fn page(&self) -> usize {
        let behind = self.pages.partition_point(|p| p.end <= self.top);
        behind.min(self.pages.len() - 1)
    }

    pub fn pages(&self) -> &[Range<usize>] {
        &self.pages
    }

    /// The probe to answer, by its place among them.
    pub fn probe(&self) -> Option<(usize, &Probe)> {
        let at = self.answers.len();
        let asked = self.stage == Stage::Probing;
        self.probes.get(at).filter(|_| asked).map(|p| (at, p))
    }

    pub fn probes(&self) -> &[Probe] {
        &self.probes
    }

    /// Lays the text out anew, as when the terminal is resized: the page
    /// holding the first word shown stays, and the time on the old one is
    /// kept as a turn of its own.
    pub fn lay(&mut self, pages: Vec<Range<usize>>, now: Instant) {
        let paged = self.pace.is_none();
        if !paged || pages.is_empty() || pages == self.pages {
            return;
        }
        if self.stage == Stage::Reading {
            self.turn(now);
        }
        self.pages = pages;
        self.top = self.pages[self.page()].start;
    }

    pub fn apply(&mut self, input: Input, now: Instant) {
        match (self.stage, input) {
            (Stage::Ready, Input::Next) => {
                self.stage = Stage::Reading;
                self.since = Some(now);
            }
            // A sliding text turns no pages.
            (Stage::Reading, _) if self.pace.is_some() => {}
            (Stage::Reading, Input::Next) => {
                self.turn(now);
                match self.pages.get(self.page() + 1) {
                    Some(next) => self.top = next.start,
                    None => self.close(),
                }
            }
            (Stage::Reading, Input::Back) => {
                if let Some(before) = self.page().checked_sub(1) {
                    self.turn(now);
                    self.top = self.pages[before].start;
                }
            }
            (Stage::Probing, Input::Yes | Input::No) => {
                let ms = self.shown(now);
                self.answers.push((input == Input::Yes, ms));
                self.since = Some(now);
                if self.answers.len() == self.probes.len() {
                    self.stage = Stage::Done;
                }
            }
            _ => {}
        }
    }

    /// Milliseconds the page or probe shown has been up.
    fn shown(&self, now: Instant) -> u64 {
        let shown = self.since.map(|since| now.duration_since(since));
        shown.map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
    }

    /// Logs the time on the page shown and restarts its clock.
    fn turn(&mut self, now: Instant) {
        let page = self.pages[self.page()].clone();
        self.turns.push(Turn(page.start, page.end, self.shown(now)));
        self.since = Some(now);
    }

    /// The finished test as recorded, read on a terminal of `size`
    /// columns and rows.
    pub fn record(
        &self,
        at: String,
        origin: Origin,
        size: (u16, u16),
    ) -> ReadRecord {
        let chars = self.chars();
        let elapsed_ms = self.turns.iter().map(|t| t.2).sum();
        let asked = |(probe, &(answer, ms)): (&Probe, &(bool, u64))| {
            Asked(probe.text.clone(), probe.old, answer, ms)
        };
        let mut record = ReadRecord {
            v: reads::VERSION,
            at,
            source: origin.source,
            cite: origin.cite,
            file: origin.file,
            continuous: origin.continuous,
            start: origin.start,
            total: origin.total,
            words: self.words(),
            chars,
            elapsed_ms,
            wpm: 0.0,
            pace: self.pace().map(f64::from),
            view: self.view(),
            turns: self.turns.clone(),
            probes: self.probes.iter().zip(&self.answers).map(asked).collect(),
            cols: size.0,
            rows: size.1,
        };
        record.wpm = stats::wpm(chars, record.elapsed());
        record
    }
}

/// The share of the time a blinking word stays for: its length, a short
/// word's no less than [`SHORT`], and a [`PAUSE`] where the text has one.
fn dwell(word: &str) -> usize {
    let letters = word.trim_end_matches(|c: char| !c.is_alphanumeric());
    let pause = match word[letters.len()..].chars().next() {
        Some('.' | '?' | '!' | '…') => 2 * PAUSE,
        Some(',' | ';' | ':' | '—' | '–') => PAUSE,
        _ => 0,
    };
    word.chars().count().max(SHORT) + 1 + pause
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    const MS: Duration = Duration::from_millis(1);

    /// A paragraph of `n` sentences worth asking about.
    fn paragraph(n: usize) -> Vec<String> {
        let sentence = "This one went on for eight whole words.";
        words::split(&vec![sentence; n].join(" "))
    }

    fn text(sentences: &[usize]) -> Text {
        let paragraphs = sentences.iter().map(|&n| paragraph(n)).collect();
        Text { paragraphs, ..Text::default() }
    }

    fn probes(old: &[bool]) -> Vec<Probe> {
        let probe = |&old| Probe { text: format!("Was it {old}?"), old };
        old.iter().map(probe).collect()
    }

    fn test(old: &[bool]) -> ReadTest {
        let words = |t: &str| words::split(t);
        ReadTest::new(vec![words("a b c"), words("d e")], probes(old))
    }

    #[test]
    fn a_stretch_reads_enough_words_and_leaves_foils() {
        // Sixteen words a paragraph, two sentences to ask about in each.
        let text = text(&[2; 10]);
        assert_eq!(text.stretch(0, 40, 4), Some(0..3), "48 words");
        assert_eq!(text.stretch(3, 40, 4), Some(3..6));
        assert_eq!(text.stretch(0, 500, 4), Some(0..8), "foils kept back");
        assert_eq!(text.stretch(0, 1, 4), Some(0..2), "enough to ask of");
        assert_eq!(text.stretch(7, 40, 4), None, "too little left");
        assert_eq!(text.stretch(0, 40, 11), None);
        assert_eq!(text.stretch(10, 40, 0), None, "nothing to read");
    }

    #[test]
    fn a_paragraph_is_found_by_its_first_word() {
        let text = text(&[1, 2, 1]);
        assert_eq!(text.words(), 32);
        assert_eq!(text.paragraph_at(0), Some(0));
        assert_eq!(text.paragraph_at(8), Some(1));
        assert_eq!(text.paragraph_at(24), Some(2));
        assert_eq!(text.paragraph_at(9), None, "inside one");
        assert_eq!(text.paragraph_at(32), None, "past the end");
    }

    #[test]
    fn files_are_running_prose_and_markdown_cites_its_front_matter() {
        let plain = Text::file("[[A, B (1900)]]\nOne two.\n\nThree.\n", false);
        assert_eq!(plain.paragraphs, [vec!["One", "two."], vec!["Three."]]);
        assert_eq!(plain.cite.as_deref(), Some("A, B (1900)"));
        assert!(plain.continuous);
        let article =
            "---\ntitle: T\nfeed: f\nurl: u\npublished: 2020-01-02\n---\n\
                       # Head\n\nSome *prose* here.\n\n```\ncode\n```\n";
        let markdown = Text::file(article, true);
        assert_eq!(markdown.paragraphs, [vec!["Some", "prose", "here."]]);
        assert_eq!(markdown.cite.as_deref(), Some("f, T (2020)"));
        assert_eq!(Text::file("Just *this*.\n", true).cite, None);
    }

    #[test]
    fn works_are_the_excerpts_under_each_citation() {
        let passage = |word: &str, cite: Option<&str>| Passage {
            words: vec![word.to_string()],
            cite: cite.map(str::to_owned),
        };
        let works = Text::works(vec![
            passage("a", Some("X")),
            passage("b", Some("X")),
            passage("c", None),
            passage("d", Some("Y")),
        ]);
        let cites: Vec<_> = works.iter().map(|w| w.cite.as_deref()).collect();
        assert_eq!(cites, [Some("X"), Some("Y")], "uncited ones are none");
        assert_eq!(works[0].paragraphs, [vec!["a"], vec!["b"]]);
        assert!(!works[0].continuous);
    }

    #[test]
    fn the_clock_runs_from_the_first_page_to_the_last() {
        let mut t = test(&[true, false]);
        let t0 = Instant::now();
        t.lay(vec![0..3, 3..5], t0);
        assert_eq!((t.stage(), t.page()), (Stage::Ready, 0));
        t.apply(Input::Yes, t0);
        t.apply(Input::Back, t0);
        assert_eq!(t.stage(), Stage::Ready, "only next starts it");
        t.apply(Input::Next, t0 + MS * 1000);
        t.apply(Input::Next, t0 + MS * 1500);
        assert_eq!((t.stage(), t.page()), (Stage::Reading, 1));
        t.apply(Input::Back, t0 + MS * 1700);
        t.apply(Input::Back, t0 + MS * 1800);
        assert_eq!(t.page(), 0, "no page before the first");
        t.apply(Input::Next, t0 + MS * 2000);
        t.apply(Input::Yes, t0 + MS * 2100);
        assert_eq!(t.stage(), Stage::Reading, "no probe yet");
        t.apply(Input::Next, t0 + MS * 2400);
        assert_eq!(t.stage(), Stage::Probing);
        assert_eq!(
            t.turns,
            [
                Turn(0, 3, 500),
                Turn(3, 5, 200),
                Turn(0, 3, 300),
                Turn(3, 5, 400)
            ]
        );
    }

    #[test]
    fn probes_are_answered_in_order_off_the_clock() {
        let mut t = test(&[true, false]);
        let t0 = Instant::now();
        assert_eq!(t.probe(), None);
        t.apply(Input::Next, t0);
        t.apply(Input::Next, t0 + MS * 600);
        assert_eq!(t.probe().map(|(i, p)| (i, p.old)), Some((0, true)));
        t.apply(Input::Next, t0 + MS * 700);
        t.apply(Input::No, t0 + MS * 900);
        assert_eq!(t.probe().map(|(i, p)| (i, p.old)), Some((1, false)));
        t.apply(Input::No, t0 + MS * 1000);
        assert_eq!((t.stage(), t.probe()), (Stage::Done, None));
        t.apply(Input::Yes, t0 + MS * 1100);
        let origin = Origin { source: "english".into(), ..Origin::default() };
        let record = t.record("t".into(), origin, (80, 24));
        assert_eq!(record.elapsed_ms, 600);
        assert_eq!((record.words, record.chars), (5, 10));
        assert_eq!(record.wpm, 200.0, "two five-char words in 0.6 s");
        assert_eq!(
            record.probes,
            [
                Asked("Was it true?".into(), true, false, 300),
                Asked("Was it false?".into(), false, false, 100)
            ]
        );
        assert_eq!((record.cols, record.rows), (80, 24));
    }

    #[test]
    fn a_paced_text_slides_past_at_its_pace_and_ends_by_itself() {
        // Ten characters at 600 WPM, fifty a second: 200 ms.
        let pace = NonZeroU32::new(600).unwrap();
        let mut t = test(&[true]).paced(pace, View::Slide);
        let t0 = Instant::now();
        assert_eq!(t.slide(), Some(Duration::from_millis(200)));
        assert_eq!(t.slid(t0), 0, "not started");
        t.apply(Input::Next, t0);
        t.lay(vec![0..2, 2..5], t0);
        assert_eq!(t.pages().len(), 1, "nothing to lay out");
        t.tick(t0 + MS * 100);
        assert_eq!((t.stage(), t.slid(t0 + MS * 100)), (Stage::Reading, 5));
        t.apply(Input::Next, t0 + MS * 120);
        t.apply(Input::Back, t0 + MS * 130);
        assert!(t.turns.is_empty(), "no pages to turn");
        t.tick(t0 + MS * 260);
        assert_eq!(t.stage(), Stage::Probing, "the end was noticed late");
        t.apply(Input::Yes, t0 + MS * 300);
        let record = t.record("t".into(), Origin::default(), (80, 24));
        assert_eq!(record.turns, [Turn(0, 5, 200)]);
        assert_eq!((record.pace, record.wpm), (Some(600.0), 600.0));
        assert_eq!(record.view, Some(View::Slide));
        assert_eq!(record.probes[0].3, 40, "answered from the late frame");
        assert_eq!(test(&[]).slide(), None, "paged tests do not slide");
        let mut unasked = test(&[]).paced(pace, View::Slide);
        unasked.apply(Input::Next, t0);
        unasked.tick(t0 + MS * 200);
        assert_eq!(unasked.stage(), Stage::Done);
    }

    #[test]
    fn a_blinking_text_shows_a_word_at_a_time_for_its_dwell() {
        assert_eq!(dwell("a"), 5, "as long as a word of four");
        assert_eq!(dwell("reading"), 8);
        assert_eq!((dwell("so,"), dwell("end.\u{201d}")), (8, 12));
        assert_eq!(dwell("—"), 8);
        // Five words of one dwell share the 200 ms of 600 WPM.
        let pace = NonZeroU32::new(600).unwrap();
        let mut t = test(&[true]).paced(pace, View::Blink);
        let t0 = Instant::now();
        assert_eq!(t.blinked(t0), 0, "not started");
        t.apply(Input::Next, t0);
        let at = |t: &ReadTest, ms| t.blinked(t0 + MS * ms);
        assert_eq!([at(&t, 0), at(&t, 39), at(&t, 40)], [0, 0, 1]);
        assert_eq!([at(&t, 100), at(&t, 199), at(&t, 900)], [2, 4, 4]);
        t.tick(t0 + MS * 200);
        assert_eq!(t.stage(), Stage::Probing, "as long as it slides");
        t.apply(Input::No, t0 + MS * 300);
        let record = t.record("t".into(), Origin::default(), (80, 24));
        assert_eq!((record.view, record.wpm), (Some(View::Blink), 600.0));
        assert_eq!(test(&[]).view(), None);
    }

    #[test]
    fn a_text_without_probes_ends_with_its_last_page() {
        let mut t = test(&[]);
        let t0 = Instant::now();
        t.apply(Input::Next, t0);
        t.apply(Input::Next, t0 + MS * 100);
        assert_eq!(t.stage(), Stage::Done);
    }

    #[test]
    fn a_new_layout_keeps_the_place_and_the_time_spent() {
        let mut t = test(&[true]);
        let t0 = Instant::now();
        t.lay(vec![0..2, 2..4, 4..5], t0);
        t.apply(Input::Next, t0);
        t.apply(Input::Next, t0 + MS * 100);
        assert_eq!(t.page(), 1);
        t.lay(vec![0..2, 2..4, 4..5], t0 + MS * 150);
        assert_eq!(t.turns.len(), 1, "the same layout is no turn");
        t.lay(vec![0..3, 3..5], t0 + MS * 300);
        assert_eq!((t.page(), t.top), (0, 0), "the page holding word 2");
        t.lay(vec![], t0 + MS * 350);
        assert_eq!(t.pages().len(), 2, "no pages are no layout");
        t.apply(Input::Next, t0 + MS * 400);
        t.apply(Input::Next, t0 + MS * 450);
        assert_eq!(
            t.turns,
            [Turn(0, 2, 100), Turn(2, 4, 200), Turn(0, 3, 100), Turn(3, 5, 50)]
        );
    }
}
