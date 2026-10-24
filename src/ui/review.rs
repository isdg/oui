//! The text of a finished test, opened from the results screen: every word
//! as it was typed, hits green and misses red, words never reached dimmed.

use ratatui::layout::{Constraint, Layout};
use ratatui::style::{Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Padding, Paragraph};
use ratatui::Frame;

use super::typing::{typed_spans, wrap};
use super::MAX_WIDTH;
use crate::session::Session;

/// The screen, its text scrolled down `scroll` rows as far as it goes;
/// returns the scroll shown.
pub fn draw(
    frame: &mut Frame,
    s: &Session,
    title: &str,
    cite: Option<&str>,
    scroll: usize,
) -> usize {
    use Constraint::{Fill, Length};

    let area = frame.area().centered_horizontally(Constraint::Max(MAX_WIDTH));
    let [header, text, footer] =
        Layout::vertical([Length(1), Fill(1), Length(1)]).areas(area);
    let heading = Line::from(vec![" oui".bold(), format!(" · {title}").into()]);
    frame.render_widget(heading, header);

    let mut block = Block::bordered().padding(Padding::horizontal(1));
    if let Some(cite) = cite {
        let cite = Line::from(format!(" {cite} ")).italic().right_aligned();
        block = block.title_bottom(cite);
    }
    let inner = block.inner(text);
    let words: Vec<(&str, Option<&str>)> = (0..s.words().len())
        .map(|i| (s.words()[i].as_str(), typed(s, i)))
        .collect();
    let widths: Vec<usize> = words.iter().map(|&(w, t)| width(w, t)).collect();
    let rows = wrap(&widths, usize::from(inner.width));
    let height = usize::from(inner.height);
    let first = scroll.min(rows.len().saturating_sub(height));
    let render = |row: &std::ops::Range<usize>| {
        let mut spans = Vec::new();
        for i in row.clone() {
            if i > row.start {
                spans.push(Span::raw(" "));
            }
            spans.extend(word(words[i].0, words[i].1));
        }
        Line::from(spans)
    };
    let lines: Vec<Line> =
        rows.iter().skip(first).take(height).map(render).collect();
    frame.render_widget(Paragraph::new(lines).block(block), text);
    let keys = " o results · ↑↓ scroll · r new test · q quit";
    frame.render_widget(Line::from(keys).dim(), footer);
    first
}

/// What was typed for word `i`: the committed word, the one in progress
/// when the test ended, or nothing past it.
fn typed(s: &Session, i: usize) -> Option<&str> {
    match s.typed().get(i) {
        Some(typed) => Some(typed),
        None if i == s.typed().len() && !s.input().is_empty() => {
            Some(s.input())
        }
        None => None,
    }
}

fn width(target: &str, typed: Option<&str>) -> usize {
    let typed = typed.map_or(0, |t| t.chars().count());
    target.chars().count().max(typed)
}

/// A word as typed: whole and green when right, else character by
/// character, with the characters never typed dimmed and underlined.
fn word(target: &str, typed: Option<&str>) -> Vec<Span<'static>> {
    let Some(typed) = typed else {
        return vec![Span::styled(target.to_string(), Style::new().dim())];
    };
    if typed == target {
        return vec![Span::styled(target.to_string(), Style::new().green())];
    }
    let mut spans = typed_spans(target, typed);
    let rest: String = target.chars().skip(typed.chars().count()).collect();
    if !rest.is_empty() {
        spans.push(Span::styled(rest, Style::new().red().dim().underlined()));
    }
    spans
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use super::*;
    use crate::session::Input;
    use crate::ui::render;

    fn finished(words: &[&str], keys: &str) -> Session {
        let words = words.iter().map(|w| w.to_string()).collect();
        let mut s = Session::new(words, None);
        let now = Instant::now();
        keys.chars().for_each(|c| s.apply(Input::Char(c), now));
        s.finish(now);
        s
    }

    fn text(spans: &[Span]) -> Vec<String> {
        spans.iter().map(|s| s.content.to_string()).collect()
    }

    #[test]
    fn words_show_how_they_were_typed() {
        assert_eq!(word("ab", Some("ab"))[0].style, Style::new().green());
        let wrong = word("abc", Some("ax"));
        assert_eq!(text(&wrong), ["a", "b", "c"]);
        assert_eq!(wrong[1].style, Style::new().red());
        assert_eq!(wrong[2].style, Style::new().red().dim().underlined());
        assert_eq!(text(&word("ab", Some("abcd"))), ["a", "b", "c", "d"]);
        assert_eq!(word("ab", None)[0].style, Style::new().dim());
    }

    #[test]
    fn the_word_in_progress_counts_as_typed() {
        let s = finished(&["ab", "cd", "ef"], "ab c");
        assert_eq!((typed(&s, 0), typed(&s, 1)), (Some("ab"), Some("c")));
        assert_eq!(typed(&s, 2), None);
        assert_eq!(width("ab", Some("abcd")), 4);
    }

    #[test]
    fn screen_shows_the_text_and_its_citation() {
        let s = finished(&["ab", "cd", "ef"], "ab cx");
        let cite = Some("Austen, Emma (1815)");
        let rows = render(40, 8, |f| {
            draw(f, &s, "en", cite, 0);
        });
        let screen = rows.join("\n");
        assert!(rows[0].starts_with(" oui · en"), "{screen}");
        assert!(screen.contains("│ ab cd ef"), "{screen}");
        assert!(screen.contains("Austen, Emma (1815) ┘"), "{screen}");
        assert!(rows[7].contains("o results"), "{screen}");
    }

    #[test]
    fn scrolling_stops_at_the_last_full_panel() {
        let words: Vec<String> = (0..12).map(|i| format!("w{i:02}")).collect();
        let words: Vec<&str> = words.iter().map(String::as_str).collect();
        let s = finished(&words, "");
        // 20 columns hold four words a row; three rows of text, two shown.
        let mut shown = 0;
        let rows = render(20, 6, |f| shown = draw(f, &s, "t", None, 9));
        assert_eq!(shown, 1);
        assert!(rows[2].contains("w04"), "{}", rows.join("\n"));
        assert!(rows[3].contains("w08"), "{}", rows.join("\n"));
    }
}
