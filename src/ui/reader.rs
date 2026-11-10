//! A work to read, opened from the results screen: its passages in order,
//! those behind the reader's place dimmed, the one at the top marked as
//! where typing would start.

use ratatui::layout::{Constraint, Layout};
use ratatui::style::{Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Padding, Paragraph};
use ratatui::Frame;

use super::typing::wrap;
use super::MAX_WIDTH;

const MARK: &str = "▌ ";

/// A work laid out for reading.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Reading<'a> {
    /// Its passages in order, each as its words.
    pub passages: Vec<&'a [String]>,
    /// The first passage not yet typed; those before it are dimmed.
    pub place: usize,
}

/// Where the reader is scrolled to: the row of a passage shown at the top.
/// A row past a passage's last stands for that last row.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Scroll {
    pub passage: usize,
    pub row: usize,
}

impl Scroll {
    pub fn down(self) -> Self {
        Scroll { row: self.row.saturating_add(1), ..self }
    }

    pub fn up(self) -> Self {
        match (self.passage, self.row) {
            (0, 0) => self,
            (passage, 0) => Scroll { passage: passage - 1, row: usize::MAX },
            (passage, row) => Scroll { passage, row: row - 1 },
        }
    }

    /// The scroll as the passages' row counts allow it: a row past a
    /// passage's rows moves into the next passage, or stays on the last.
    fn within(self, rows: &[usize]) -> Self {
        let Some(last) = rows.len().checked_sub(1) else {
            return Scroll::default();
        };
        let passage = self.passage.min(last);
        let up = self.row == usize::MAX;
        match self.row < rows[passage] {
            true => Scroll { passage, row: self.row },
            false if up || passage == last => {
                Scroll { passage, row: rows[passage].saturating_sub(1) }
            }
            false => Scroll { passage: passage + 1, row: 0 },
        }
    }
}

/// The screen, scrolled to `scroll` as far as the work goes; returns the
/// scroll shown, whose passage is the marked one.
pub fn draw(
    frame: &mut Frame,
    reading: &Reading,
    title: &str,
    cite: Option<&str>,
    scroll: Scroll,
) -> Scroll {
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
    let width = usize::from(inner.width).saturating_sub(MARK.chars().count());
    // Each passage's rows of words, then a blank row before the next.
    let wrapped: Vec<Vec<&[String]>> = reading
        .passages
        .iter()
        .map(|words| {
            let widths: Vec<usize> =
                words.iter().map(|w| w.chars().count()).collect();
            let rows = wrap(&widths, width.max(1));
            rows.into_iter().map(|row| &words[row]).collect()
        })
        .collect();
    let last = wrapped.len().saturating_sub(1);
    let rows = |(i, rows): (usize, &Vec<&[String]>)| {
        rows.len() + usize::from(i < last)
    };
    let counts: Vec<usize> = wrapped.iter().enumerate().map(rows).collect();
    let scroll = scroll.within(&counts);

    let line = |passage: usize, row: Option<&&[String]>| {
        let marked = passage == scroll.passage && row.is_some();
        let mark = if marked { MARK } else { "  " };
        let style = match passage < reading.place {
            true => Style::new().dim(),
            false => Style::new(),
        };
        let words = row.map(|words| words.join(" ")).unwrap_or_default();
        Line::from(vec![Span::raw(mark), Span::styled(words, style)])
    };
    let lines: Vec<Line> = wrapped
        .iter()
        .enumerate()
        .flat_map(|(i, rows)| (0..counts[i]).map(move |row| (i, rows.get(row))))
        .skip(counts[..scroll.passage].iter().sum::<usize>() + scroll.row)
        .take(usize::from(inner.height))
        .map(|(passage, row)| line(passage, row))
        .collect();
    frame.render_widget(Paragraph::new(lines).block(block), text);
    let keys = " O results · ↑↓ scroll · enter type from ▌ · q quit";
    frame.render_widget(Line::from(keys).dim(), footer);
    scroll
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::render;

    fn passages(texts: &[&str]) -> Vec<Vec<String>> {
        let words = |t: &&str| t.split(' ').map(str::to_owned).collect();
        texts.iter().map(words).collect()
    }

    fn reading(passages: &[Vec<String>], place: usize) -> Reading<'_> {
        let passages = passages.iter().map(Vec::as_slice).collect();
        Reading { passages, place }
    }

    fn at(passage: usize, row: usize) -> Scroll {
        Scroll { passage, row }
    }

    #[test]
    fn scrolling_crosses_passages_and_stops_at_the_ends() {
        // Two rows and a blank, then one row.
        let rows = [3, 1];
        assert_eq!(at(0, 2).down().within(&rows), at(1, 0));
        assert_eq!(at(1, 0).down().within(&rows), at(1, 0), "the last row");
        assert_eq!(at(1, 0).up().within(&rows), at(0, 2));
        assert_eq!(at(0, 1).up().within(&rows), at(0, 0));
        assert_eq!(at(0, 0).up().within(&rows), at(0, 0));
        assert_eq!(at(7, 0).within(&rows), at(1, 0), "no such passage");
        assert_eq!(at(3, 2).within(&[]), at(0, 0));
    }

    #[test]
    fn screen_shows_the_work_with_the_top_passage_marked() {
        let work = passages(&["aa bb cc", "dd ee", "ff"]);
        let cite = Some("Austen, Emma (1815)");
        let mut shown = Scroll::default();
        let rows = render(60, 9, |f| {
            shown = draw(f, &reading(&work, 1), "en", cite, at(1, 0));
        });
        let screen = rows.join("\n");
        assert_eq!(shown, at(1, 0));
        assert!(rows[0].starts_with(" oui · en"), "{screen}");
        assert!(rows[2].starts_with("│ ▌ dd ee"), "{screen}");
        assert!(rows[4].starts_with("│   ff"), "{screen}");
        assert!(!screen.contains("aa bb"), "scrolled past\n{screen}");
        assert!(screen.contains("Austen, Emma (1815) ┘"), "{screen}");
        assert!(rows[8].contains("enter type from ▌"), "{screen}");
    }

    #[test]
    fn passages_wrap_and_those_behind_the_place_are_dimmed() {
        let work = passages(&["aa bb cc dd", "ee"]);
        let mut shown = Scroll::default();
        // 13 columns leave seven for words: two a row.
        let backend = ratatui::backend::TestBackend::new(13, 8);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        let reading = reading(&work, 1);
        terminal
            .draw(|f| shown = draw(f, &reading, "t", None, at(0, 9)))
            .unwrap();
        assert_eq!(shown, at(1, 0), "past its rows is the next passage");
        terminal
            .draw(|f| shown = draw(f, &reading, "t", None, at(0, 0)))
            .unwrap();
        let buffer = terminal.backend().buffer();
        let row = |y| -> String {
            (0..13).map(|x| buffer[(x, y)].symbol()).collect()
        };
        assert!(row(2).starts_with("│ ▌ aa bb "), "{}", row(2));
        assert!(row(3).starts_with("│ ▌ cc dd "), "{}", row(3));
        assert!(row(5).starts_with("│   ee "), "{}", row(5));
        let dim = |x, y| {
            let style = buffer[(x, y)].style();
            style.add_modifier.contains(ratatui::style::Modifier::DIM)
        };
        assert!(dim(4, 2) && !dim(4, 5), "only what is behind the place");
    }
}
