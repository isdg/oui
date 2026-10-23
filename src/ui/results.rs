//! The results screen: a stats block and WPM plot side by side, then the key
//! heatmap beside the rankings of weak keys and bigrams.

use ratatui::layout::{Constraint, Layout as Split};
use ratatui::style::{Color, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use super::{clock, lines, show, MAX_WIDTH};
use crate::keyboard::{self, Layout, Measure};
use crate::plot;
use crate::stats::Report;

/// Height of the WPM plot, as in `plc stat`.
const PLOT_ROWS: usize = 8;
/// Entries per ranking.
const TOP: usize = 5;

pub fn draw(
    frame: &mut Frame,
    report: &Report,
    layout: Layout,
    measure: Measure,
    title: &str,
    notice: Option<&str>,
) {
    use Constraint::{Fill, Length};

    let area = frame.area().centered_horizontally(Constraint::Max(MAX_WIDTH));
    let [header, top, _, bottom, _, footer] = Split::vertical([
        Length(1),
        Length(12),
        Length(1),
        Length(14),
        Fill(1),
        Length(1),
    ])
    .areas(area);
    let [stats, chart] = Split::horizontal([Length(41), Fill(1)]).areas(top);
    let [board, rankings] =
        Split::horizontal([Length(50), Fill(1)]).areas(bottom);
    let [upper, lower] =
        Split::vertical([Length(7), Length(6)]).areas(rankings);
    let [keys, typos] = Split::horizontal([Fill(1), Fill(1)]).areas(upper);
    let [errors, slow] = Split::horizontal([Fill(1), Fill(1)]).areas(lower);

    let heading = Line::from(vec![" oui".bold(), format!(" · {title}").into()]);
    frame.render_widget(heading, header);
    frame.render_widget(Paragraph::new(lines(summary(report))), stats);
    frame.render_widget(
        Paragraph::new(lines(wpm_plot(report, chart.width))),
        chart,
    );
    let heat = keyboard_lines(report, layout, measure);
    frame.render_widget(Paragraph::new(heat), board);
    for (area, ranking) in [
        (keys, worst_keys(report)),
        (typos, top_typos(report)),
        (errors, error_bigrams(report)),
        (slow, slow_bigrams(report)),
    ] {
        frame.render_widget(Paragraph::new(lines(ranking)), area);
    }
    let keys = " r new test · p drill weak bigrams · tab errors/speed · q quit";
    frame.render_widget(Line::from(keys).dim(), footer);
    if let Some(notice) = notice {
        let notice = Line::from(format!("{notice} ")).red().right_aligned();
        frame.render_widget(notice, footer);
    }
}

fn summary(r: &Report) -> Vec<String> {
    vec![
        rule("Results", 34),
        format!("  WPM          : {:.0}", r.wpm),
        format!("  Raw WPM      : {:.0}", r.raw_wpm),
        format!(
            "  Accuracy     : {:.1}%  ({}/{} keys)",
            r.accuracy * 100.0,
            r.hits,
            r.keystrokes
        ),
        format!("  Time         : {}", clock(r.elapsed)),
        format!("  Words        : {} / {} correct", r.correct_words, r.words),
    ]
}

fn wpm_plot(r: &Report, width: u16) -> Vec<String> {
    let peak = r.timeline.iter().copied().fold(0.0, f64::max).round() as u64;
    let label = peak.max(1).to_string().len() + 2;
    let mut out =
        vec![format!("{}wpm — 5 s window", " ".repeat(label)), String::new()];
    let cols = usize::from(width).saturating_sub(label + 1);
    if r.timeline.is_empty() || cols == 0 {
        out.push(format!("{}(no data)", " ".repeat(label)));
        return out;
    }
    let samples = plot::resample(&r.timeline, cols);
    let values: Vec<u64> = samples.iter().map(|v| v.round() as u64).collect();
    let max = values.iter().copied().max().unwrap_or(0);
    out.extend(plot::line_chart(max, PLOT_ROWS, &values, &|v| v.to_string()));
    let secs_per_col = r.timeline.len() as f64 / values.len() as f64;
    out.push(plot::axis(label, values.len(), secs_per_col));
    out
}

/// The heatmap as tinted key tiles under a title, with its legend.
fn keyboard_lines(
    r: &Report,
    layout: Layout,
    measure: Measure,
) -> Vec<Line<'static>> {
    let heat = keyboard::heat(layout, &r.keys);
    let title = format!("  keyboard — {}", measure.name());
    let mut out = vec![Line::from(title), Line::default()];
    for (indent, tiles) in keyboard::tiles(layout, &heat, measure) {
        out.push(tile_line(2 + indent, &tiles));
    }
    out.push(Line::default());
    out.push(tile_line(2, &keyboard::legend(measure)));
    out
}

fn tile_line(indent: usize, tiles: &[keyboard::Tile]) -> Line<'static> {
    let mut spans = vec![Span::raw(" ".repeat(indent))];
    for tile in tiles {
        let style = match tile.level {
            Some(l) => {
                let (bg, fg) = keyboard::TINTS[l];
                Style::new().bg(Color::Indexed(bg)).fg(Color::Indexed(fg))
            }
            None => Style::new().dim(),
        };
        spans.push(Span::styled(tile.text.clone(), style));
    }
    Line::from(spans)
}

pub fn worst_keys(r: &Report) -> Vec<String> {
    let keys = r.worst_keys(TOP).into_iter().map(|(c, t)| {
        let rate = t.miss_rate() * 100.0;
        format!(" {}   {rate:>3.0}%  {}/{}", show(c), t.misses, t.attempts())
    });
    ranking("Worst keys", keys)
}

pub fn top_typos(r: &Report) -> Vec<String> {
    let typos = r
        .top_typos(TOP)
        .into_iter()
        .map(|((want, got), n)| format!(" {}→{}  ×{n}", show(want), show(got)));
    ranking("Typos", typos)
}

pub fn error_bigrams(r: &Report) -> Vec<String> {
    let bigrams = r.error_bigrams(TOP).into_iter().map(|((a, b), t)| {
        format!(" {}{}   {}/{}", show(a), show(b), t.misses, t.attempts())
    });
    ranking("Error bigrams", bigrams)
}

pub fn slow_bigrams(r: &Report) -> Vec<String> {
    let bigrams = r.slow_bigrams(TOP).into_iter().map(|((a, b), d)| {
        format!(" {}{}   {} ms", show(a), show(b), d.as_millis())
    });
    ranking("Slow bigrams", bigrams)
}

fn ranking(title: &str, entries: impl Iterator<Item = String>) -> Vec<String> {
    let mut out = vec![rule(title, 22)];
    out.extend(entries);
    if out.len() == 1 {
        out.push(" (none)".to_string());
    }
    out
}

/// A `── Title ───` section rule, `width` columns wide.
pub fn rule(title: &str, width: usize) -> String {
    let head = format!("── {title} ");
    let fill = width.saturating_sub(head.chars().count());
    format!("{head}{}", "─".repeat(fill))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::time::Duration;

    use super::*;
    use crate::stats::Tally;
    use crate::ui::render;

    fn report() -> Report {
        let tally = |hits, misses| Tally { hits, misses, ..Tally::default() };
        Report {
            elapsed: Duration::from_secs(47),
            wpm: 64.2,
            raw_wpm: 71.0,
            accuracy: 0.968,
            hits: 301,
            keystrokes: 311,
            words: 50,
            correct_words: 48,
            timeline: (0..47).map(|s| 40.0 + f64::from(s)).collect(),
            keys: BTreeMap::from([('o', tally(14, 3)), ('e', tally(30, 0))]),
            bigrams: BTreeMap::from([(('q', 'u'), tally(2, 2))]),
            typos: BTreeMap::from([(('o', 'i'), 2)]),
        }
    }

    #[test]
    fn summary_aligns_label_value_rows() {
        let lines = summary(&report());
        assert_eq!(lines[1], "  WPM          : 64");
        assert_eq!(lines[3], "  Accuracy     : 96.8%  (301/311 keys)");
        assert_eq!(lines[4], "  Time         : 0:47");
        assert_eq!(lines[5], "  Words        : 48 / 50 correct");
    }

    #[test]
    fn plot_spans_the_timeline_with_a_axis() {
        let lines = wpm_plot(&report(), 60);
        assert_eq!(lines.len(), 2 + PLOT_ROWS + 2);
        assert!(lines[2].starts_with("86 ┼"));
        assert!(lines[2].ends_with('─') || lines[2].ends_with('╭'));
        assert!(lines.last().unwrap().contains("10"));
    }

    #[test]
    fn plot_without_samples_says_so() {
        let empty = Report::default();
        assert_eq!(wpm_plot(&empty, 60).last().unwrap().trim(), "(no data)");
    }

    #[test]
    fn rankings_list_entries_or_none() {
        let r = report();
        assert_eq!(worst_keys(&r)[1], " o    18%  3/17");
        assert_eq!(error_bigrams(&r)[1], " qu   2/4");
        assert_eq!(top_typos(&r)[1], " o→i  ×2");
        assert_eq!(slow_bigrams(&r)[1], " (none)");
        assert_eq!(rule("Typos", 12), "── Typos ───");
    }

    #[test]
    fn screen_shows_every_section() {
        let rows = render(100, 30, |f| {
            draw(f, &report(), Layout::Qwerty, Measure::Speed, "english", None)
        });
        let screen = rows.join("\n");
        for needle in [
            "oui · english",
            "WPM          : 64",
            "wpm — 5 s window",
            "keyboard — speed",
            "── Worst keys",
            "── Slow bigrams",
            "tab errors/speed",
        ] {
            assert!(screen.contains(needle), "missing {needle:?}\n{screen}");
        }
    }
}
