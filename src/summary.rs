//! `oui stats`: the recorded tests as plain text in the shape of `plc stat`
//! -- a stats block, WPM per test, both heatmaps and the rankings.

use std::fmt::Write as _;
use std::time::Duration;

use crate::history::Record;
use crate::keyboard::{Layout, Measure};
use crate::plot;
use crate::stats::{self, Report};
use crate::ui::results;

/// Widest the per-test plot grows before neighbouring tests are averaged.
const PLOT_COLS: usize = 70;
const PLOT_ROWS: usize = 8;
/// Columns each ranking takes when printed side by side.
const RANKING_WIDTH: usize = 24;

pub fn render(records: &[Record], layout: Layout) -> String {
    let reports: Vec<Report> = records.iter().map(Record::report).collect();
    let all = stats::combine(&reports);
    let wpms: Vec<f64> =
        records.iter().filter(|r| r.completed).map(|r| r.wpm).collect();

    let mut out = String::new();
    out.push_str("\n     ── History ─────────────────────────\n");
    let _ = writeln!(
        out,
        "     Tests        : {}   ({} completed)",
        records.len(),
        wpms.len()
    );
    if let (Some(first), Some(last)) = (records.first(), records.last()) {
        let (first, last) = (day(&first.at), day(&last.at));
        let _ = writeln!(out, "     First / last : {first} / {last}");
    }
    let _ = writeln!(out, "     Time typing  : {}", hms(all.elapsed));
    if !wpms.is_empty() {
        let recent = &wpms[wpms.len().saturating_sub(10)..];
        let (mean, recent) = (mean(&wpms), mean(recent));
        let best = wpms.iter().copied().fold(0.0, f64::max);
        let _ = writeln!(
            out,
            "     WPM          : {mean:.0} mean · {best:.0} best · \
             {recent:.0} last 10"
        );
    }
    let _ = writeln!(
        out,
        "     Accuracy     : {:.1}%  ({}/{} keys)",
        all.accuracy * 100.0,
        all.hits,
        all.keystrokes
    );

    out.push_str(&wpm_plot(&wpms));
    for measure in [Measure::Errors, Measure::Speed] {
        out.push('\n');
        for line in results::keyboard_lines(&all, layout, measure) {
            let _ = writeln!(out, "{}", format!("   {line}").trim_end());
        }
    }
    out.push('\n');
    let rankings = [
        results::worst_keys(&all),
        results::error_bigrams(&all),
        results::slow_bigrams(&all),
        results::top_typos(&all),
    ];
    for line in side_by_side(&rankings, RANKING_WIDTH) {
        let _ = writeln!(out, "{}", format!("     {line}").trim_end());
    }
    out
}

fn wpm_plot(wpms: &[f64]) -> String {
    let mut out = String::from("\n          wpm — per completed test\n\n");
    if wpms.is_empty() {
        out.push_str("          (no data)\n");
        return out;
    }
    let samples = plot::resample(wpms, PLOT_COLS);
    let values: Vec<u64> = samples.iter().map(|v| v.round() as u64).collect();
    let max = values.iter().copied().max().unwrap_or(0);
    for line in plot::line_chart(max, PLOT_ROWS, &values, &|v| v.to_string()) {
        let _ = writeln!(out, "     {line}");
    }
    let label = max.max(1).to_string().len();
    let per_col = wpms.len() as f64 / values.len() as f64;
    let axis = plot::axis(label + 2, values.len(), per_col);
    let _ = writeln!(out, "{}", format!("     {axis}").trim_end());
    out
}

/// Text blocks laid out as columns `width` wide, row by row.
fn side_by_side(blocks: &[Vec<String>], width: usize) -> Vec<String> {
    let rows = blocks.iter().map(Vec::len).max().unwrap_or(0);
    let cell = |block: &Vec<String>, row: usize| {
        let text = block.get(row).map_or("", String::as_str);
        let pad = width.saturating_sub(text.chars().count());
        format!("{text}{}", " ".repeat(pad))
    };
    let line = |row| blocks.iter().map(|b| cell(b, row)).collect::<String>();
    (0..rows).map(line).collect()
}

fn mean(values: &[f64]) -> f64 {
    values.iter().sum::<f64>() / values.len().max(1) as f64
}

/// The date part of an RFC 3339 timestamp.
fn day(at: &str) -> &str {
    at.get(..10).unwrap_or(at)
}

fn hms(d: Duration) -> String {
    let s = d.as_secs();
    format!("{}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60)
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use super::*;
    use crate::history::Meta;
    use crate::session::{Input, Session};

    fn record(text: &str, keys: &str, at: &str) -> Record {
        let words = text.split(' ').map(str::to_owned).collect();
        let mut s = Session::new(words, None);
        let t0 = Instant::now();
        let ms = Duration::from_millis(150);
        for (i, c) in keys.chars().enumerate() {
            s.apply(Input::Char(c), t0 + ms * i as u32);
        }
        let now = t0 + ms * keys.len() as u32;
        s.finish(now);
        Record::new(&s, now, at.to_string(), Meta::default())
    }

    #[test]
    fn summary_counts_tests_and_spans_dates() {
        let records = [
            record("ab cd", "ab cd", "2026-10-23T09:00:00+02:00"),
            record("ab cd", "ax", "2026-10-24T09:00:00+02:00"),
        ];
        let out = render(&records, Layout::Qwerty);
        assert!(out.contains("Tests        : 2   (1 completed)"), "{out}");
        assert!(out.contains("First / last : 2026-10-23 / 2026-10-24"));
        assert!(out.contains("Accuracy     : 85.7%  (6/7 keys)"));
        assert!(out.contains("wpm — per completed test"));
        assert!(out.contains("keyboard — errors"));
        assert!(out.contains("keyboard — speed"));
        assert!(out.contains("── Worst keys"));
        assert!(out.contains(" b    50%  1/2"), "b missed once in two");
    }

    #[test]
    fn plot_without_completed_tests_says_so() {
        let records = [record("ab", "x", "2026-10-24T09:00:00+02:00")];
        assert!(render(&records, Layout::Qwerty).contains("(no data)"));
    }

    #[test]
    fn columns_pad_to_width_and_fill_short_blocks() {
        let blocks =
            [vec!["ab".to_string(), "c".to_string()], vec!["de".into()]];
        assert_eq!(side_by_side(&blocks, 4), ["ab  de  ", "c       "]);
    }

    #[test]
    fn time_is_hours_minutes_seconds() {
        assert_eq!(hms(Duration::from_secs(3725)), "1:02:05");
        assert_eq!(day("2026-10-24T09:00:00+02:00"), "2026-10-24");
    }
}
