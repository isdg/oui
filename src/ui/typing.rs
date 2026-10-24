//! The typing screen: a header, the sliding line, the input box at the
//! centre, and the whole text below it as in ttyper.

use std::cmp::Ordering;
use std::ops::Range;
use std::time::Instant;

use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Padding, Paragraph};
use ratatui::Frame;

use super::{clock, MAX_WIDTH};
use crate::session::Session;
use crate::stats;

/// Words either side of the cursor the slider lays out.
const REACH: usize = 20;

/// The screen; `cite` names where the text is from, under it.
pub fn draw(
    frame: &mut Frame,
    s: &Session,
    now: Instant,
    title: &str,
    cite: Option<&str>,
) {
    use Constraint::{Fill, Length};

    let area = frame.area().centered_horizontally(Constraint::Max(MAX_WIDTH));
    let [header, _, slider, input, text, footer] = Layout::vertical([
        Length(1),
        Fill(1),
        Length(3),
        Length(3),
        Fill(1),
        Length(1),
    ])
    .areas(area);
    draw_header(frame, header, s, now, title);
    draw_slider(frame, slider, s);
    draw_input(frame, input, s);
    draw_text(frame, text, s, cite);
    let keys = " esc results · tab restart · ctrl-c quit";
    frame.render_widget(Line::from(keys).dim(), footer);
}

fn draw_header(
    frame: &mut Frame,
    area: Rect,
    s: &Session,
    now: Instant,
    title: &str,
) {
    let elapsed = s.elapsed(now);
    let shown =
        s.limit().map_or(elapsed, |limit| limit.saturating_sub(elapsed));
    let wpm = stats::wpm(s.correct_chars(), elapsed);
    let accuracy = stats::accuracy(s.log()) * 100.0;
    let live = format!("{}   {wpm:.0} wpm   {accuracy:.0}% ", clock(shown));
    let left = Line::from(vec![" oui".bold(), format!(" · {title}").into()]);
    frame.render_widget(left, area);
    frame.render_widget(Line::from(live).right_aligned(), area);
}

/// One line scrolled so the cursor sits at its centre.
fn draw_slider(frame: &mut Frame, area: Rect, s: &Session) {
    let block = Block::bordered();
    let inner = block.inner(area);
    let cur = s.current();
    let from = cur.saturating_sub(REACH);
    let to = (cur + REACH + 1).min(s.words().len());
    let mut spans = vec![Span::raw(" ".repeat(usize::from(inner.width / 2)))];
    let mut cursor = s.input().chars().count();
    for i in from..to {
        if i > from {
            spans.push(Span::raw(" "));
        }
        if i < cur {
            cursor += width(s, i) + 1;
        }
        spans.extend(word(s, i));
    }
    let scroll = u16::try_from(cursor).unwrap_or(u16::MAX);
    let slider = Paragraph::new(Line::from(spans)).scroll((0, scroll));
    frame.render_widget(slider.block(block), area);
}

fn draw_input(frame: &mut Frame, area: Rect, s: &Session) {
    let Some(target) = s.words().get(s.current()) else { return };
    let block = Block::bordered()
        .title(format!(" {target} "))
        .padding(Padding::horizontal(1));
    let inner = block.inner(area);
    let typed = s.input().chars().count();
    let mut spans = typed_spans(target, s.input());
    let ghost: String = target.chars().skip(typed).collect();
    spans.push(Span::styled(ghost, Style::new().dim()));
    frame.render_widget(Paragraph::new(Line::from(spans)).block(block), area);
    let col = u16::try_from(typed).unwrap_or(u16::MAX);
    let col = col.min(inner.width.saturating_sub(1));
    frame.set_cursor_position((inner.x + col, inner.y));
}

/// The whole text, wrapped and scrolled only as far as [`first_row`] needs.
fn draw_text(frame: &mut Frame, area: Rect, s: &Session, cite: Option<&str>) {
    let mut block = Block::bordered().padding(Padding::horizontal(1));
    if let Some(cite) = cite {
        let cite = Line::from(format!(" {cite} ")).italic().right_aligned();
        block = block.title_bottom(cite);
    }
    let inner = block.inner(area);
    let widths: Vec<usize> =
        (0..s.words().len()).map(|i| width(s, i)).collect();
    let rows = wrap(&widths, usize::from(inner.width));
    let at = rows.iter().position(|r| r.contains(&s.current()));
    let height = usize::from(inner.height);
    let first = first_row(at.unwrap_or(rows.len()), rows.len(), height);
    let render = |row: &Range<usize>| {
        let mut spans = Vec::new();
        for i in row.clone() {
            if i > row.start {
                spans.push(Span::raw(" "));
            }
            spans.extend(word(s, i));
        }
        Line::from(spans)
    };
    let lines: Vec<Line> =
        rows.iter().skip(first).take(height).map(render).collect();
    frame.render_widget(Paragraph::new(lines).block(block), area);
}

/// The top line to show: the first while the text fits, else the one above
/// the current line, but never so low that the panel's bottom goes empty.
fn first_row(current: usize, rows: usize, height: usize) -> usize {
    current.saturating_sub(1).min(rows.saturating_sub(height))
}

/// Columns word `i` takes: its target, or the input if that overflows it.
fn width(s: &Session, i: usize) -> usize {
    let target = s.words()[i].chars().count();
    if i == s.current() {
        target.max(s.input().chars().count())
    } else {
        target
    }
}

/// Word `i` coloured by how it was typed; the current one carries a cursor.
fn word(s: &Session, i: usize) -> Vec<Span<'static>> {
    let target = &s.words()[i];
    match i.cmp(&s.current()) {
        Ordering::Less if s.typed()[i] == *target => {
            vec![Span::styled(target.clone(), Style::new().green())]
        }
        Ordering::Less => {
            vec![Span::styled(target.clone(), Style::new().red().underlined())]
        }
        Ordering::Greater => {
            vec![Span::styled(target.clone(), Style::new().dim())]
        }
        Ordering::Equal => {
            let typed = s.input().chars().count();
            let mut spans = typed_spans(target, s.input());
            for (j, c) in target.chars().enumerate().skip(typed) {
                let style = Style::new().bold();
                let style = if j == typed { style.reversed() } else { style };
                spans.push(Span::styled(c.to_string(), style));
            }
            spans
        }
    }
}

/// The typed part of a word: target characters green when hit and red when
/// missed, then any overflow as typed.
fn typed_spans(target: &str, input: &str) -> Vec<Span<'static>> {
    let mut want = target.chars();
    let span = |c: char| match want.next() {
        Some(w) if w == c => Span::styled(w.to_string(), Style::new().green()),
        Some(w) => Span::styled(w.to_string(), Style::new().red()),
        None => Span::styled(c.to_string(), Style::new().light_red()),
    };
    input.chars().map(span).collect()
}

/// Greedy word wrap: the word ranges of each line, none wider than `max`
/// unless a single word is.
fn wrap(widths: &[usize], max: usize) -> Vec<Range<usize>> {
    let mut rows = Vec::new();
    let (mut start, mut used) = (0, 0);
    for (i, &w) in widths.iter().enumerate() {
        if i > start && used + 1 + w > max {
            rows.push(start..i);
            (start, used) = (i, 0);
        }
        used += if i > start { 1 + w } else { w };
    }
    if start < widths.len() {
        rows.push(start..widths.len());
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::Input;
    use crate::ui::render;

    fn session(words: &[&str], keys: &str) -> Session {
        let words = words.iter().map(|w| w.to_string()).collect();
        let mut s = Session::new(words, None);
        let now = Instant::now();
        keys.chars().for_each(|c| s.apply(Input::Char(c), now));
        s
    }

    #[test]
    fn wrap_breaks_before_overflowing_words() {
        assert_eq!(wrap(&[2, 2, 2], 5), [0..2, 2..3]);
        assert_eq!(wrap(&[9, 1], 5), [0..1, 1..2]);
        assert_eq!(wrap(&[], 5), []);
    }

    #[test]
    fn text_that_fits_never_scrolls() {
        assert_eq!(first_row(3, 4, 9), 0);
        assert_eq!(first_row(0, 0, 9), 0);
    }

    #[test]
    fn long_text_keeps_a_line_of_context_and_a_full_panel() {
        assert_eq!(first_row(5, 20, 4), 4);
        assert_eq!(first_row(19, 20, 4), 16);
        assert_eq!(first_row(0, 20, 4), 0);
    }

    #[test]
    fn overflow_widens_only_the_current_word() {
        let s = session(&["ab", "cd"], "abxy");
        assert_eq!((width(&s, 0), width(&s, 1)), (4, 2));
    }

    #[test]
    fn typed_spans_show_targets_then_overflow() {
        let spans = typed_spans("ab", "xbz");
        let text: Vec<_> =
            spans.iter().map(|s| s.content.to_string()).collect();
        assert_eq!(text, ["a", "b", "z"]);
        assert_eq!(spans[0].style, Style::new().red());
        assert_eq!(spans[1].style, Style::new().green());
    }

    #[test]
    fn slider_centres_the_cursor() {
        let s = session(&["ab", "cd", "ef"], "ab ");
        let rows =
            render(40, 16, |f| draw(f, &s, Instant::now(), "english", None));
        let centre = |r: &String| r.chars().nth(20) == Some('c');
        assert!(rows.iter().any(centre), "{}", rows.join("\n"));
    }

    #[test]
    fn screen_shows_header_input_and_text() {
        let s = session(&["ab", "cd", "ef"], "ab c");
        let rows =
            render(40, 16, |f| draw(f, &s, Instant::now(), "english", None));
        let screen = rows.join("\n");
        assert!(rows[0].starts_with(" oui · english"), "{screen}");
        assert!(screen.contains("┌ cd "), "input titled with the word");
        assert!(screen.contains("│ ab cd ef"), "text area");
        assert!(screen.contains("esc results"));
        assert!(!screen.contains("Austen"));
    }

    #[test]
    fn text_area_cites_its_source_underneath() {
        let s = session(&["ab", "cd"], "");
        let cite = Some("Austen, Emma (1815)");
        let rows = render(40, 16, |f| draw(f, &s, Instant::now(), "en", cite));
        let bottom = rows.iter().rfind(|r| r.contains('└')).unwrap();
        assert!(bottom.ends_with("─ Austen, Emma (1815) ┘"), "{bottom}");
    }
}
