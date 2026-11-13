//! The screens of `oui read`: the text hidden until its clock starts, then
//! a page at a time, the probes one by one, and the results.

use std::ops::Range;
use std::time::{Duration, Instant};

use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Stylize;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Padding, Paragraph};
use ratatui::Frame;

use super::results::rule;
use super::typing::{slide, wrap};
use super::{clock, lines, MAX_WIDTH};
use crate::read::ReadTest;
use crate::reads::{Asked, ReadRecord, Turn, View, PASS};

/// Columns of text at most: longer lines are slower to read.
const MEASURE: u16 = 72;
/// Characters either side of its centre a sliding text lays out.
const REACH: usize = 120;
/// Columns of the results' summary, the pages beside it.
const SUMMARY: u16 = 48;
/// Columns and rows around the text: its border and padding, the header
/// and the footer.
const CHROME: (u16, u16) = (4, 4);

/// The columns and rows of text a terminal of `cols` by `rows` shows.
pub fn text_size(cols: u16, rows: u16) -> (usize, usize) {
    let width = cols.min(MEASURE + CHROME.0).saturating_sub(CHROME.0);
    let height = rows.saturating_sub(CHROME.1);
    (usize::from(width.max(1)), usize::from(height.max(1)))
}

/// The text wrapped at `width`: each row as its words, counted through
/// the paragraphs, with an empty row between two paragraphs.
fn rows(paragraphs: &[Vec<String>], width: usize) -> Vec<Range<usize>> {
    let mut out = Vec::new();
    let mut at = 0;
    for words in paragraphs.iter().filter(|p| !p.is_empty()) {
        if at > 0 {
            out.push(at..at);
        }
        let widths: Vec<usize> =
            words.iter().map(|w| w.chars().count()).collect();
        let wrapped = wrap(&widths, width);
        out.extend(wrapped.into_iter().map(|r| at + r.start..at + r.end));
        at += words.len();
    }
    out
}

/// The words each page of `height` rows shows; none opens on an empty row.
pub fn pages(
    paragraphs: &[Vec<String>],
    width: usize,
    height: usize,
) -> Vec<Range<usize>> {
    let mut out: Vec<Range<usize>> = Vec::new();
    let mut filled = height;
    for row in rows(paragraphs, width) {
        match out.last_mut().filter(|_| filled < height) {
            Some(page) => page.end = row.end,
            None if row.is_empty() => continue,
            None => {
                out.push(row);
                filled = 0;
            }
        }
        filled += 1;
    }
    out
}

/// The header, the bordered area for text and the footer.
fn frame_of(frame: &mut Frame, title: &str) -> (Rect, Rect) {
    use Constraint::{Fill, Length};

    let width = Constraint::Max(MEASURE + CHROME.0);
    let area = frame.area().centered_horizontally(width);
    let [header, text, footer] =
        Layout::vertical([Length(1), Fill(1), Length(1)]).areas(area);
    let heading = Line::from(vec![" oui".bold(), format!(" · {title}").into()]);
    frame.render_widget(heading, header);
    (text, footer)
}

fn block(cite: Option<&str>) -> Block<'_> {
    let block = Block::bordered().padding(Padding::horizontal(1));
    match cite {
        Some(cite) => {
            let cite = Line::from(format!(" {cite} ")).italic();
            block.title_bottom(cite.right_aligned())
        }
        None => block,
    }
}

/// Lines set in the middle of `area`, top to bottom.
fn centred(frame: &mut Frame, area: Rect, block: Block, text: Vec<Line>) {
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let height = u16::try_from(text.len()).unwrap_or(u16::MAX);
    let middle = inner.centered_vertically(Constraint::Max(height));
    frame.render_widget(Paragraph::new(text), middle);
}

/// Before the clock starts: what is about to be read, the text unseen.
pub fn ready(frame: &mut Frame, test: &ReadTest, title: &str) {
    let (area, footer) = frame_of(frame, title);
    let (words, probes) = (test.words(), test.probes().len());
    let then = format!("then {probes} sentences to recognise.");
    let past = test.slide().map(clock).unwrap_or_default();
    let text = match test.view() {
        Some(View::Slide) => vec![
            format!("{words} words slide past in {past}, {then}"),
            String::new(),
            "Keep up with the text: it does not wait, and it".to_string(),
            "does not come back.".to_string(),
        ],
        Some(View::Blink) => vec![
            format!("{words} words blink past in {past}, {then}"),
            String::new(),
            "Keep your eyes on the bold letter: each word comes".to_string(),
            "to it, and none comes back.".to_string(),
        ],
        None => vec![
            format!("{words} words on {} pages, {then}", test.pages().len()),
            String::new(),
            "Read as you would read anything: the clock starts".to_string(),
            "with the first page and stops with the last.".to_string(),
        ],
    };
    let text = lines(text);
    centred(frame, area, block(None), text);
    let keys = " space start · esc quit";
    frame.render_widget(Line::from(keys).dim(), footer);
}

/// The page of the text the test is on.
pub fn page(
    frame: &mut Frame,
    test: &ReadTest,
    title: &str,
    cite: Option<&str>,
) {
    let (area, footer) = frame_of(frame, title);
    let block = block(cite);
    let width = usize::from(block.inner(area).width).max(1);
    let words: Vec<&str> =
        test.paragraphs().iter().flatten().map(String::as_str).collect();
    let shown = &test.pages()[test.page()];
    // The empty row between two paragraphs shows only between two rows.
    let on_page = |row: &Range<usize>| match row.is_empty() {
        true => shown.start < row.start && row.end < shown.end,
        false => shown.start <= row.start && row.end <= shown.end,
    };
    let text: Vec<Line> = rows(test.paragraphs(), width)
        .into_iter()
        .filter(on_page)
        .map(|row| Line::from(words[row].join(" ")))
        .collect();
    frame.render_widget(Paragraph::new(text).block(block), area);
    let keys = " space next · ← back · esc abandon";
    frame.render_widget(Line::from(keys).dim(), footer);
    let at = format!("{}/{} ", test.page() + 1, test.pages().len());
    frame.render_widget(Line::from(at).dim().right_aligned(), footer);
}

/// The stripe of the typing screen under the header, for a paced text
/// to go through: none while the text is paused, and kept out of sight.
fn band(frame: &mut Frame, test: &ReadTest, title: &str) -> Option<Rect> {
    use Constraint::{Fill, Length};

    let area = frame.area().centered_horizontally(Constraint::Max(MAX_WIDTH));
    let [header, _, band, _, footer] =
        Layout::vertical([Length(1), Fill(1), Length(3), Fill(1), Length(1)])
            .areas(area);
    let heading = Line::from(vec![" oui".bold(), format!(" · {title}").into()]);
    frame.render_widget(heading, header);
    let keys = match test.paused() {
        true => " space go on · esc abandon",
        false => " space pause · esc abandon",
    };
    frame.render_widget(Line::from(keys).dim(), footer);
    if test.paused() {
        slide(frame, band, vec!["paused".dim()], 3);
    }
    (!test.paused()).then_some(band)
}

/// A paced text sliding past the centre of the stripe.
pub fn stripe(frame: &mut Frame, test: &ReadTest, title: &str, now: Instant) {
    let Some(band) = band(frame, test, title) else { return };
    let words: Vec<&str> =
        test.paragraphs().iter().flatten().map(String::as_str).collect();
    let text: Vec<char> = words.join(" ").chars().collect();
    let at = test.slid(now).min(text.len());
    let from = at.saturating_sub(REACH);
    let to = (at + REACH).min(text.len());
    let shown: String = text[from..to].iter().collect();
    slide(frame, band, vec![Span::raw(shown)], at - from);
}

/// A paced text a word at a time, each with its [`pivot`] at the centre
/// of the stripe.
pub fn blink(frame: &mut Frame, test: &ReadTest, title: &str, now: Instant) {
    let Some(band) = band(frame, test, title) else { return };
    let mut words = test.paragraphs().iter().flatten();
    let Some(word) = words.nth(test.blinked(now)) else { return };
    let letters: Vec<char> = word.chars().collect();
    let at = pivot(letters.len());
    let part = |of: &[char]| Span::raw(of.iter().collect::<String>());
    let (before, rest) = letters.split_at(at);
    let (on, after) = rest.split_at(1.min(rest.len()));
    slide(frame, band, vec![part(before), part(on).bold(), part(after)], at);
}

/// The letter of a word of `letters` the eye takes it in best from: a
/// little left of its middle.
fn pivot(letters: usize) -> usize {
    match letters {
        0..=1 => 0,
        2..=5 => 1,
        6..=9 => 2,
        10..=13 => 3,
        _ => 4,
    }
}

/// A sentence to recognise, the text out of sight.
pub fn probe(frame: &mut Frame, test: &ReadTest, title: &str) {
    let (area, footer) = frame_of(frame, title);
    let Some((at, probe)) = test.probe() else { return };
    let block = block(None);
    let width = usize::from(block.inner(area).width).max(1);
    let words: Vec<&str> = probe.text.split(' ').collect();
    let widths: Vec<usize> = words.iter().map(|w| w.chars().count()).collect();
    let mut text = vec![
        Line::from("Was this sentence in the text you just read?").dim(),
        Line::default(),
    ];
    let sentence = wrap(&widths, width).into_iter();
    text.extend(sentence.map(|row| Line::from(words[row].join(" ")).bold()));
    centred(frame, area, block, text);
    let keys = " y → it was · n ← it was not";
    frame.render_widget(Line::from(keys).dim(), footer);
    let at = format!("{}/{} ", at + 1, test.probes().len());
    frame.render_widget(Line::from(at).dim().right_aligned(), footer);
}

/// The finished test; `incognito` tells whether the history holds it
/// flagged so, when it holds it at all.
pub fn results(
    frame: &mut Frame,
    record: &ReadRecord,
    title: &str,
    notice: Option<&str>,
    incognito: Option<bool>,
) {
    use Constraint::{Fill, Length};

    let (area, footer) = frame_of(frame, title);
    let [top, bottom] = Layout::vertical([Length(9), Fill(1)]).areas(area);
    let [stats, pages] =
        Layout::horizontal([Length(SUMMARY), Fill(1)]).areas(top);
    frame.render_widget(Paragraph::new(lines(summary(record))), stats);
    let mut turns = vec![rule("Pages", 24)];
    turns.extend(record.turns.iter().map(turn));
    frame.render_widget(Paragraph::new(lines(turns)), pages);
    let width = usize::from(bottom.width);
    let mut asked_of = vec![rule("Sentences", 34)];
    asked_of.extend(record.probes.iter().map(|p| asked(p, width)));
    frame.render_widget(Paragraph::new(lines(asked_of)), bottom);
    let incognito = match incognito {
        Some(false) => "i incognito · ",
        Some(true) => "i count · ",
        None => "",
    };
    let keys = format!(" r next text · {incognito}q quit");
    frame.render_widget(Line::from(keys).dim(), footer);
    if let Some(notice) = notice {
        let notice = Line::from(format!("{notice} ")).red().right_aligned();
        frame.render_widget(notice, footer);
    }
}

fn summary(r: &ReadRecord) -> Vec<String> {
    let (right, of) = r.right();
    let verdict = match r.counts() {
        true => "counts".to_string(),
        false => format!("under {:.0}%: not counted", PASS * 100.0),
    };
    let share = |(n, of): (usize, usize)| format!("{n}/{of}");
    vec![
        rule("Reading", 34),
        format!("  WPM          : {:.0}  (5 characters a word)", r.wpm),
        format!("  Words/min    : {:.0}", r.words_a_minute()),
        format!("  Time         : {}", clock(r.elapsed())),
        format!("  Words        : {}", r.words),
        format!("  Recognised   : {right}/{of}  ({verdict})"),
        format!("  Hits         : {}", share(r.hits())),
        format!("  False alarms : {}", share(r.false_alarms())),
    ]
}

/// A stretch of text shown: its words, the time on it and the pace.
fn turn(&Turn(start, end, ms): &Turn) -> String {
    let words = end - start;
    let pace = match ms {
        0 => 0.0,
        ms => words as f64 * 60_000.0 / ms as f64,
    };
    let time = clock(Duration::from_millis(ms));
    format!("{words:>5} words {time:>5} {pace:>5.0}/min")
}

/// A probe on a row of `width`: whether the answer was right, where the
/// sentence was from, and as much of it as fits.
fn asked(Asked(text, old, answer, _): &Asked, width: usize) -> String {
    let mark = if old == answer { '✓' } else { '✗' };
    let from = if *old { "read" } else { "new " };
    let room = width.saturating_sub(12);
    let mut shown: String = text.chars().take(room).collect();
    if shown.len() < text.len() {
        shown.pop();
        shown.push('…');
    }
    format!("  {mark} {from}  {shown}")
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use super::*;
    use crate::probes::Probe;
    use crate::read::{Input, Origin};
    use crate::ui::render;
    use crate::words;

    fn test() -> ReadTest {
        let paragraphs = ["aa bb cc dd", "ee ff", "gg"];
        let paragraphs = paragraphs.iter().map(|p| words::split(p)).collect();
        let probe = |text: &str, old| Probe { text: text.to_string(), old };
        let probes =
            vec![probe("It was read here.", true), probe("No.", false)];
        ReadTest::new(paragraphs, probes)
    }

    fn laid(width: usize, height: usize) -> ReadTest {
        let mut test = test();
        let pages = pages(test.paragraphs(), width, height);
        test.lay(pages, Instant::now());
        test
    }

    #[test]
    fn the_text_keeps_to_a_measure_inside_its_frame() {
        assert_eq!(text_size(80, 24), (72, 20));
        assert_eq!(text_size(200, 50), (72, 46), "no wider than the measure");
        assert_eq!(text_size(40, 10), (36, 6));
        assert_eq!(text_size(2, 2), (1, 1), "never nothing");
    }

    #[test]
    fn pages_fill_with_rows_and_open_on_text() {
        let text = test();
        let paragraphs = text.paragraphs();
        // Five columns hold two words: rows aa bb / cc dd / – / ee ff / –
        // / gg.
        assert_eq!(rows(paragraphs, 5), [0..2, 2..4, 4..4, 4..6, 6..6, 6..7]);
        assert_eq!(pages(paragraphs, 5, 2), [0..4, 4..6, 6..7]);
        assert_eq!(pages(paragraphs, 5, 3), [0..4, 4..7]);
        assert_eq!(pages(paragraphs, 5, 9).len(), 1);
        assert_eq!(pages(paragraphs, 5, 1), [0..2, 2..4, 4..6, 6..7]);
        assert!(pages(&[], 5, 3).is_empty());
    }

    #[test]
    fn the_text_is_hidden_until_the_clock_starts() {
        let test = laid(5, 2);
        let rows = render(70, 10, |f| ready(f, &test, "english"));
        let screen = rows.join("\n");
        assert!(rows[0].starts_with(" oui · english"), "{screen}");
        assert!(screen.contains("7 words on 3 pages, then 2 sentences"));
        assert!(!screen.contains("aa bb"), "{screen}");
        assert!(rows[9].contains("space start"), "{screen}");
    }

    #[test]
    fn a_page_shows_its_rows_and_its_place() {
        // Nine columns leave five for text, six rows two.
        let (width, height) = text_size(9, 6);
        let mut test = laid(width, height);
        test.apply(Input::Next, Instant::now());
        let cite = Some("A, B");
        let rows = render(9, 6, |f| page(f, &test, "en", cite));
        let screen = rows.join("\n");
        assert!(rows[2].starts_with("│ aa bb"), "{screen}");
        assert!(rows[3].starts_with("│ cc dd"), "{screen}");
        assert!(!screen.contains("ee"), "the next page\n{screen}");
        test.apply(Input::Next, Instant::now());
        let rows = render(40, 6, |f| page(f, &test, "en", cite));
        let screen = rows.join("\n");
        assert!(rows[2].starts_with("│ ee ff"), "{screen}");
        assert!(rows[4].contains("A, B ┘"), "{screen}");
        assert!(rows[5].contains("space next"), "{screen}");
        assert!(rows[5].trim_end().ends_with("2/3"), "{screen}");
    }

    #[test]
    fn a_paced_text_slides_through_the_centre_of_its_stripe() {
        // 600 WPM is fifty characters a second.
        let pace = std::num::NonZeroU32::new(600).unwrap();
        let mut test = test().paced(pace, View::Slide);
        let rows = render(70, 10, |f| ready(f, &test, "en"));
        let screen = rows.join("\n");
        assert!(screen.contains("7 words slide past in 0:00, then 2"));
        let t0 = Instant::now();
        test.apply(Input::Next, t0);
        let at = |ms| {
            let now = t0 + Duration::from_millis(ms);
            render(24, 7, |f| stripe(f, &test, "en", now))
        };
        // Twenty-two columns inside the border: the centre is the 12th.
        let rows = at(0);
        assert_eq!(rows[3], "│           aa bb cc dd│", "{rows:?}");
        let rows = at(120);
        assert_eq!(rows[3], "│     aa bb cc dd ee ff│", "six gone by");
        let rows = at(400);
        assert_eq!(rows[3], "│dd ee ff gg           │", "its end");
        assert!(rows[6].contains("space pause"));
    }

    #[test]
    fn a_blinking_text_sets_each_word_by_its_pivot() {
        assert_eq!([pivot(1), pivot(2), pivot(5), pivot(6)], [0, 1, 1, 2]);
        assert_eq!([pivot(9), pivot(10), pivot(13), pivot(14)], [2, 3, 3, 4]);
        let words = words::split("a recognition of it.");
        let pace = std::num::NonZeroU32::new(600).unwrap();
        let mut test =
            ReadTest::new(vec![words], Vec::new()).paced(pace, View::Blink);
        let rows = render(70, 10, |f| ready(f, &test, "en"));
        let screen = rows.join("\n");
        assert!(screen.contains("4 words blink past in 0:00, then 0"));
        let t0 = Instant::now();
        test.apply(Input::Next, t0);
        // Dwells of 5, 12, 5 and 11 share 440 ms: 67, 160, 67 and 147.
        let at = |ms| {
            let now = t0 + Duration::from_millis(ms);
            render(24, 7, |f| blink(f, &test, "en", now))
        };
        assert_eq!(at(0)[3], "│           a          │");
        assert_eq!(at(100)[3], "│        recognition   │", "its fourth");
        assert_eq!(at(230)[3], "│          of          │");
        let rows = at(300);
        assert_eq!(rows[3], "│          it.         │");
        assert!(rows[6].contains("space pause"));
    }

    #[test]
    fn a_paused_text_is_out_of_sight() {
        let pace = std::num::NonZeroU32::new(600).unwrap();
        let t0 = Instant::now();
        for view in [View::Slide, View::Blink] {
            let mut test = test().paced(pace, view);
            test.apply(Input::Next, t0);
            test.apply(Input::Pause, t0);
            let rows = render(24, 7, |f| match view {
                View::Slide => stripe(f, &test, "en", t0),
                View::Blink => blink(f, &test, "en", t0),
            });
            assert_eq!(rows[3], "│        paused        │", "{rows:?}");
            assert!(rows[6].contains("space go on"), "{rows:?}");
        }
    }

    #[test]
    fn a_probe_shows_one_sentence_and_no_text() {
        let mut test = laid(72, 20);
        test.apply(Input::Next, Instant::now());
        test.apply(Input::Next, Instant::now());
        let rows = render(60, 9, |f| probe(f, &test, "en"));
        let screen = rows.join("\n");
        assert!(screen.contains("It was read here."), "{screen}");
        assert!(!screen.contains("aa bb") && !screen.contains("No."));
        assert!(rows[8].contains("y → it was"), "{screen}");
        assert!(rows[8].trim_end().ends_with("1/2"), "{screen}");
    }

    #[test]
    fn results_show_both_rates_and_every_answer() {
        let mut test = laid(72, 20);
        let t0 = Instant::now();
        test.apply(Input::Next, t0);
        test.apply(Input::Next, t0 + Duration::from_millis(1000));
        test.apply(Input::Yes, t0);
        test.apply(Input::Yes, t0);
        let record = test.record("t".into(), Origin::default(), (80, 24));
        let notice = Some("not saved");
        let rows =
            render(76, 20, |f| results(f, &record, "en", notice, Some(false)));
        let screen = rows.join("\n");
        assert!(screen.contains("WPM          : 252"), "{screen}");
        assert!(screen.contains("Words/min    : 420"), "{screen}");
        assert!(screen.contains("Recognised   : 1/2  (under 80%"), "{screen}");
        assert!(screen.contains("Hits         : 1/1"), "{screen}");
        assert!(screen.contains("False alarms : 1/1"), "{screen}");
        assert!(screen.contains("7 words  0:01   420/min"), "{screen}");
        assert!(screen.contains("✓ read  It was read here."), "{screen}");
        assert!(screen.contains("✗ new   No."), "{screen}");
        assert!(rows[19].contains("i incognito · q quit"), "{screen}");
        assert!(rows[19].contains("not saved"), "{screen}");
    }

    #[test]
    fn a_long_sentence_is_cut_to_the_row() {
        let long = Asked("abcdefghijklmnopqrstuvwxyz".into(), true, true, 0);
        assert_eq!(asked(&long, 20), "  ✓ read  abcdefg…");
    }
}
