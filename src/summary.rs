//! `oui stats`: the recorded tests as plain text in the shape of `plc stat`
//! -- a stats block, WPM per test or per period and the recent texts, then
//! on request heatmaps and rankings, and the fingers, rows and hand moves.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::time::Duration;

use clap::ValueEnum;

use crate::fingering::{Fingering, Mark, Marked};
use crate::geometry::{self, Finger, Geometry};
use crate::history::Record;
use crate::keyboard::{self, Key, Layout, Measure};
use crate::plot;
use crate::practice::Target;
use crate::stats::{self, Report, Rhythm, Tally};
use crate::ui::{results, show};

/// Widest the per-test plot grows before neighbouring tests are averaged.
const PLOT_COLS: usize = 70;
const PLOT_ROWS: usize = 8;
/// Widest a line of practice words grows.
const WORDS_WIDTH: usize = 80;
/// Columns each ranking takes when printed side by side.
const RANKING_WIDTH: usize = 24;
/// Cited tests listed under the plot, most recent first.
const RECENT_TEXTS: usize = 10;
/// Same-finger bigrams listed under the geometry tables.
const SAME_FINGER_TOP: usize = 8;
const FINGERS_LEGEND: &str = "1–4 L pinky to index   7–0 R index to pinky   \
                              ≈ move as one   ? unresolved";

/// Calendar periods `oui stats --by` groups tests into.
#[derive(ValueEnum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum Period {
    Day,
    /// ISO weeks, Monday first.
    Week,
    Month,
}

impl Period {
    fn name(self) -> &'static str {
        match self {
            Period::Day => "day",
            Period::Week => "week",
            Period::Month => "month",
        }
    }

    /// The period an RFC 3339 time falls in, in the zone it was recorded in.
    fn key(self, at: &str) -> Option<String> {
        let time = chrono::DateTime::parse_from_rfc3339(at).ok()?;
        let format = match self {
            Period::Day => "%Y-%m-%d",
            Period::Week => "%G-W%V",
            Period::Month => "%Y-%m",
        };
        Some(time.format(format).to_string())
    }
}

/// What `oui stats` shows beyond the summary, and how.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct View {
    /// The keyboard keys are placed on.
    pub layout: Layout,
    /// Tints the heatmaps with ANSI.
    pub colour: bool,
    /// Adds a table of periods and plots WPM per period, not per test.
    pub by: Option<Period>,
    /// Adds the heatmaps and the key and bigram rankings.
    pub detailed: bool,
    /// Adds the fingers, rows and hand moves.
    pub geometry: bool,
}

/// The summary up to the recent texts, then what `view` adds.
pub fn render(records: &[Record], view: &View) -> String {
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
    if let Some(rhythm) = rhythm(&all.rhythm) {
        let _ = writeln!(out, "     Rhythm       : {rhythm}");
    }

    match view.by {
        None => {
            let modes: Vec<Vec<&str>> = records
                .iter()
                .filter(|r| r.completed)
                .map(|r| vec![r.meta.mode.as_str()])
                .collect();
            let modes = view.colour.then_some(modes.as_slice());
            out.push_str(&wpm_plot("per completed test", &wpms, modes));
        }
        Some(period) => {
            let groups = group(records, period);
            out.push_str(&period_table(&groups, period));
            let (means, modes): (Vec<f64>, Vec<Vec<&str>>) = groups
                .iter()
                .filter_map(|(_, g)| Some((completed_mean(g)?, modes(g))))
                .unzip();
            let modes = view.colour.then_some(modes.as_slice());
            let title = format!("mean per {}", period.name());
            out.push_str(&wpm_plot(&title, &means, modes));
        }
    }
    out.push_str(&recent_texts(records));
    if view.detailed {
        out.push_str(&details(&all, view));
    }
    if view.geometry {
        out.push_str(&geometry_tables(&all, view.layout));
        out.push_str(&fingering(records, view.layout));
    }
    out
}

/// Both heatmaps, then the key and bigram rankings side by side.
fn details(all: &Report, view: &View) -> String {
    let mut out = String::new();
    for measure in [Measure::Errors, Measure::Speed] {
        out.push('\n');
        for line in heatmap(all, view.layout, measure, view.colour) {
            let _ = writeln!(out, "{}", line.trim_end());
        }
    }
    out.push('\n');
    let rankings = [
        results::worst_keys(all),
        results::error_bigrams(all),
        results::slow_bigrams(all),
        results::top_typos(all),
    ];
    for line in side_by_side(&rankings, RANKING_WIDTH) {
        let _ = writeln!(out, "{}", format!("     {line}").trim_end());
    }
    out
}

/// What the latest text-mode tests typed, by their citations; empty when
/// no test was cited.
fn recent_texts(records: &[Record]) -> String {
    let cited = records.iter().rev().filter_map(|r| {
        let cite = r.meta.cite.as_deref()?;
        let stopped = if r.completed { "" } else { "  (stopped)" };
        Some(format!(
            "     {}  {:>4.0} wpm  {:>5.1}%  {cite}{stopped}",
            day(&r.at),
            r.wpm,
            r.accuracy * 100.0
        ))
    });
    let lines: Vec<String> = cited.take(RECENT_TEXTS).collect();
    if lines.is_empty() {
        return String::new();
    }
    let mut out = format!("\n     {}\n", results::rule("Recent texts", 41));
    for line in lines {
        let _ = writeln!(out, "{line}");
    }
    out
}

/// Mean latency, its spread, and the spread as a share of the mean.
fn rhythm(rhythm: &Rhythm) -> Option<String> {
    let (mean, deviation) = (rhythm.mean()?, rhythm.deviation()?);
    let variation = rhythm.variation().unwrap_or(0.0) * 100.0;
    Some(format!(
        "{} ms ± {} ms  ({variation:.0}% variation)",
        mean.as_millis(),
        deviation.as_millis()
    ))
}

fn geometry_tables(r: &Report, layout: Layout) -> String {
    let g = Geometry::new(layout, r);
    let named = |name: &str, t: &Tally| (name.to_string(), *t);
    let fingers: Vec<_> =
        g.fingers.iter().map(|(f, t)| named(f.name(), t)).collect();
    let rows: Vec<_> =
        g.rows.iter().map(|(row, t)| named(row.name(), t)).collect();
    let moves: Vec<_> =
        g.transitions.iter().map(|(m, t)| named(m.name(), t)).collect();
    let pairs = attempts(&moves);
    let same_finger: Vec<_> = g
        .costly_same_finger(SAME_FINGER_TOP)
        .into_iter()
        .map(|((a, b), t)| (format!("{}{}", show(a), show(b)), t))
        .collect();
    let mut out =
        tally_table("Fingers", "Finger", "Keys", &fingers, attempts(&fingers));
    out.push_str(&tally_table("Rows", "Row", "Keys", &rows, attempts(&rows)));
    out.push_str(&tally_table("Hand moves", "Move", "Pairs", &moves, pairs));
    let title = "Same-finger bigrams";
    out.push_str(&tally_table(title, "Bigram", "Pairs", &same_finger, pairs));
    out
}

/// The fingering each layout's tests show, for those with enough of them.
fn fingering(records: &[Record], layout: Layout) -> String {
    let infer = |&l: &Layout| {
        let tests = records.iter().filter(|r| r.meta.layout == l.name());
        let logs: Vec<_> = tests.map(Record::log).collect();
        Some((l, Fingering::infer(l, logs.iter().map(Vec::as_slice))?))
    };
    let yours: Vec<_> =
        Layout::value_variants().iter().filter_map(infer).collect();
    fingering_boards(layout, &yours)
}

/// The standard fingering on `layout`, then each inferred one with the
/// pairs behind its marks; the number row only where one is marked.
fn fingering_boards(layout: Layout, yours: &[(Layout, Fingering)]) -> String {
    let marked = yours.iter().flat_map(|(_, f)| f.marks.keys());
    let number =
        marked.into_iter().any(|k| matches!(k, Key::At { row: 0, .. }));
    let board = |title: &str, layout, marks: &BTreeMap<Key, Marked>| {
        let mut out = format!("\n          fingers — {title}\n\n");
        let rows = keyboard::board(layout, |k| symbol(k, marks));
        for row in &rows[usize::from(!number)..] {
            let _ = writeln!(out, "     {row}");
        }
        out
    };
    let mut out = board("standard", layout, &BTreeMap::new());
    let _ = writeln!(out, "\n     {FINGERS_LEGEND}");
    for (layout, fingering) in yours {
        let median = millis(fingering.baseline);
        let title = format!("yours ({}), median {median} ms", layout.name());
        out.push_str(&board(&title, *layout, &fingering.marks));
        out.push('\n');
        let label = |k| layout.char_at(k).unwrap_or(' ');
        for (&key, marked) in &fingering.marks {
            let pairs: Vec<String> = marked
                .evidence
                .iter()
                .map(|p| {
                    let (a, b) = (label(p.keys.0), label(p.keys.1));
                    format!("{a}{b} {} ms ×{}", millis(p.median), p.count)
                })
                .collect();
            let name = label(key).to_uppercase();
            let mark = symbol(key, &fingering.marks);
            let line = format!("     {name}{mark}  {}", pairs.join(" · "));
            let _ = writeln!(out, "{}", line.trim_end());
        }
    }
    out
}

/// A key's finger as typing courses number them, or its mark.
fn symbol(key: Key, marks: &BTreeMap<Key, Marked>) -> char {
    let digit = |finger| match finger {
        Finger::LeftPinky => '1',
        Finger::LeftRing => '2',
        Finger::LeftMiddle => '3',
        Finger::LeftIndex => '4',
        Finger::RightIndex => '7',
        Finger::RightMiddle => '8',
        Finger::RightRing => '9',
        Finger::RightPinky => '0',
        Finger::Thumb => ' ',
    };
    match marks.get(&key).map(|m| m.mark) {
        Some(Mark::Moved(finger)) => digit(finger),
        Some(Mark::AsOne) => '≈',
        Some(Mark::Unsure) => '?',
        None => geometry::finger(key).map_or(' ', digit),
    }
}

/// The bigrams `--practice` targets in `language`, then the words it draws.
pub fn practice(
    language: &str,
    targets: &[Target],
    words: &[String],
) -> String {
    let title = format!("Practice · {language}");
    let mut out = format!("\n     {}\n", results::rule(&title, 41));
    let _ = writeln!(
        out,
        "     {:<12}{:>6}{:>7}{:>8}{:>8}",
        "Bigram", "Pairs", "Miss", "Mean", "Cost"
    );
    for t in targets {
        let (a, b) = t.bigram;
        let miss = 100.0 * t.tally.miss_rate();
        let mean = t.tally.mean_latency().map_or_else(
            || "—".to_string(),
            |d| format!("{} ms", d.as_millis()),
        );
        let cost = format!("{:.1} s", t.cost.as_secs_f64());
        let _ = writeln!(
            out,
            "     {:<12}{:>6}{miss:>6.1}%{mean:>8}{cost:>8}",
            format!("{}{}", show(a), show(b)),
            t.tally.attempts()
        );
    }
    out.push('\n');
    let mut line = String::from("    ");
    for word in words {
        if line.chars().count() + 1 + word.chars().count() > WORDS_WIDTH {
            let _ = writeln!(out, "{line}");
            line = String::from("    ");
        }
        line.push(' ');
        line.push_str(word);
    }
    if !words.is_empty() {
        let _ = writeln!(out, "{line}");
    }
    out
}

fn attempts(rows: &[(String, Tally)]) -> u32 {
    rows.iter().map(|(_, t)| t.attempts()).sum()
}

/// Tallies as rows of attempts, their share of `total`, miss rate and mean
/// latency.
fn tally_table(
    title: &str,
    label: &str,
    unit: &str,
    rows: &[(String, Tally)],
    total: u32,
) -> String {
    let mut out = format!("\n     {}\n", results::rule(title, 41));
    let _ = writeln!(
        out,
        "     {label:<12}{unit:>6}{:>8}{:>7}{:>8}",
        "Share", "Miss", "Mean"
    );
    if rows.is_empty() {
        out.push_str("     (none)\n");
    }
    for (name, t) in rows {
        let share = 100.0 * f64::from(t.attempts()) / f64::from(total.max(1));
        let miss = 100.0 * t.miss_rate();
        let mean = t.mean_latency().map_or_else(
            || "—".to_string(),
            |d| format!("{} ms", d.as_millis()),
        );
        let _ = writeln!(
            out,
            "     {name:<12}{:>6}{share:>7.1}%{miss:>6.1}%{mean:>8}",
            t.attempts()
        );
    }
    out
}

/// Tests by period, oldest first; records with an unreadable time are left
/// out, since they belong to no period.
fn group(records: &[Record], period: Period) -> Vec<(String, Vec<&Record>)> {
    let mut groups: BTreeMap<String, Vec<&Record>> = BTreeMap::new();
    for record in records {
        if let Some(key) = period.key(&record.at) {
            groups.entry(key).or_default().push(record);
        }
    }
    groups.into_iter().collect()
}

fn period_table(groups: &[(String, Vec<&Record>)], period: Period) -> String {
    let mut out = format!("\n     ── By {} ", period.name());
    out.push_str(&"─".repeat(40 - out.chars().count().min(40) + 5));
    out.push('\n');
    let name = capitalise(period.name());
    let _ =
        writeln!(out, "     {name:<10}  Tests      Time   WPM  Best  Accuracy");
    for (key, records) in groups {
        let ms: u64 = records.iter().map(|r| r.elapsed_ms).sum();
        let time = hms(Duration::from_millis(ms));
        let wpms: Vec<f64> = completed(records).collect();
        let (wpm, best) = match wpms.is_empty() {
            true => ("—".to_string(), "—".to_string()),
            false => {
                let best = wpms.iter().copied().fold(0.0, f64::max);
                (format!("{:.0}", mean(&wpms)), format!("{best:.0}"))
            }
        };
        let keys = records.iter().flat_map(|r| &r.keys);
        let (hits, total) =
            keys.fold((0, 0), |(h, t), k| (h + usize::from(k.1 == k.2), t + 1));
        let accuracy = 100.0 * hits as f64 / total.max(1) as f64;
        let _ = writeln!(
            out,
            "     {key:<10}  {:>5}  {time:>8}  {wpm:>4}  {best:>4}  {accuracy:>7.1}%",
            records.len()
        );
    }
    out
}

fn completed<'a>(records: &'a [&Record]) -> impl Iterator<Item = f64> + 'a {
    records.iter().filter(|r| r.completed).map(|r| r.wpm)
}

fn modes<'a>(records: &[&'a Record]) -> Vec<&'a str> {
    let completed = records.iter().filter(|r| r.completed);
    completed.map(|r| r.meta.mode.as_str()).collect()
}

fn completed_mean(records: &[&Record]) -> Option<f64> {
    let wpms: Vec<f64> = completed(records).collect();
    (!wpms.is_empty()).then(|| mean(&wpms))
}

fn capitalise(word: &str) -> String {
    let mut chars = word.chars();
    chars
        .next()
        .map_or_else(String::new, |c| c.to_uppercase().chain(chars).collect())
}

/// The WPM trend; with `modes`, the tests behind each value, every column
/// is drawn in the colour of the mode most of its tests were in.
fn wpm_plot(title: &str, wpms: &[f64], modes: Option<&[Vec<&str>]>) -> String {
    let mut out = format!("\n          wpm — {title}\n\n");
    if wpms.is_empty() {
        out.push_str("          (no data)\n");
        return out;
    }
    let samples = plot::resample(wpms, PLOT_COLS);
    let values: Vec<u64> = samples.iter().map(|v| v.round() as u64).collect();
    let max = values.iter().copied().max().unwrap_or(0);
    // From the slowest value's ten, not zero: a trend spans a few WPM.
    let lo = values.iter().copied().min().unwrap_or(0) / 10 * 10;
    let fmt = |v: u64| v.to_string();
    let label = max.max(lo + 1).to_string().len().max(lo.to_string().len());
    let shown: Vec<&str> = match modes {
        Some(modes) => plot::buckets(modes.len(), PLOT_COLS)
            .map(|b| dominant(&modes[b].concat()))
            .collect(),
        None => Vec::new(),
    };
    let columns: Vec<Option<u8>> =
        shown.iter().map(|m| Some(mode_colour(m))).collect();
    let lines = plot::line_chart_from(lo, max, PLOT_ROWS, &values, &fmt);
    for (row, line) in lines.iter().enumerate() {
        let line = match row < PLOT_ROWS {
            true => plot::tint(line, label + 2, &columns),
            false => line.clone(),
        };
        let _ = writeln!(out, "     {line}");
    }
    let per_col = wpms.len() as f64 / values.len() as f64;
    let axis = plot::axis(label + 2, values.len(), per_col);
    let _ = writeln!(out, "{}", format!("     {axis}").trim_end());
    if modes.is_some() {
        let mut seen: Vec<&str> = Vec::new();
        for mode in shown {
            if !seen.contains(&mode) {
                seen.push(mode);
            }
        }
        let key = |m: &&str| {
            plot::tint(&format!("● {m}"), 0, &[Some(mode_colour(m))])
        };
        let keys: Vec<String> = seen.iter().map(key).collect();
        let _ =
            writeln!(out, "\n{}{}", " ".repeat(label + 7), keys.join("   "));
    }
    out
}

/// The mode most of `modes` are in; ties go to the earliest.
fn dominant<'a>(modes: &[&'a str]) -> &'a str {
    let count = |m: &&str| modes.iter().filter(|n| *n == m).count();
    let mut best = "";
    let mut most = 0;
    for mode in modes {
        let n = count(mode);
        if n > most {
            (best, most) = (mode, n);
        }
    }
    best
}

/// A muted ANSI 256 colour per kind of test, so no mode outshouts another.
fn mode_colour(mode: &str) -> u8 {
    match mode {
        "words" => 109,
        "text" => 144,
        "funny" => 181,
        "practice" => 146,
        "file" => 152,
        _ => 248,
    }
}

fn heatmap(
    r: &Report,
    layout: Layout,
    measure: Measure,
    colour: bool,
) -> Vec<String> {
    let heat = keyboard::heat(layout, &r.keys);
    let tiles = keyboard::tiles(layout, &heat, measure);
    let (rows, legend) = if colour {
        let legend = keyboard::ansi(&[(0, keyboard::legend(measure))]);
        (keyboard::ansi(&tiles), legend.concat())
    } else {
        (keyboard::plain(&tiles), keyboard::plain_legend(measure))
    };
    let mut out =
        vec![format!("          keyboard — {}", measure.name()), String::new()];
    out.extend(rows.into_iter().map(|row| format!("     {row}")));
    out.push(String::new());
    out.push(format!("     {legend}"));
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

fn millis(d: Duration) -> u128 {
    (d + Duration::from_micros(500)).as_millis()
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

    fn summary() -> View {
        View { layout: Layout::Qwerty, ..View::default() }
    }

    #[test]
    fn practice_lists_targets_then_wraps_the_words() {
        let tally = Tally {
            hits: 9,
            misses: 1,
            timed: 9,
            latency: Duration::from_millis(1800),
        };
        let cost = Duration::from_millis(12_340);
        let targets = [Target { bigram: (' ', 'p'), tally, cost }];
        let words: Vec<String> = (0..30).map(|i| format!("pp{i}")).collect();
        let out = practice("english", &targets, &words);
        assert!(out.contains("── Practice · english"), "{out}");
        assert!(out.contains("     ␣p              10  10.0%  200 ms  12.3 s"));
        let lines: Vec<&str> =
            out.rsplit("\n\n").next().unwrap().lines().collect();
        assert!(lines.iter().all(|l| l.chars().count() <= WORDS_WIDTH));
        assert_eq!(lines.join(" ").split_whitespace().count(), 30);
    }

    fn detailed() -> View {
        View { detailed: true, ..summary() }
    }

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
        let out = render(&records, &summary());
        assert!(out.contains("Tests        : 2   (1 completed)"), "{out}");
        assert!(out.contains("First / last : 2026-10-23 / 2026-10-24"));
        assert!(out.contains("Accuracy     : 85.7%  (6/7 keys)"));
        assert!(out.contains("wpm — per completed test"));
        assert!(!out.contains("keyboard —") && !out.contains("Worst keys"));
        assert!(!out.contains("── Fingers"));
        let out = render(&records, &detailed());
        assert!(out.contains("keyboard — errors"));
        assert!(out.contains("keyboard — speed"));
        assert!(out.contains("── Worst keys"));
        assert!(out.contains(" b    50%  1/2"), "b missed once in two");
    }

    #[test]
    fn plot_without_completed_tests_says_so() {
        let records = [record("ab", "x", "2026-10-24T09:00:00+02:00")];
        assert!(render(&records, &summary()).contains("(no data)"));
    }

    #[test]
    fn colour_draws_each_test_in_its_mode() {
        let at = |mode: &str, at| {
            let mut r = record("ab", "ab", at);
            r.meta.mode = mode.into();
            r
        };
        let records = [
            at("words", "2026-10-24T08:00:00+02:00"),
            at("text", "2026-10-24T09:00:00+02:00"),
        ];
        assert!(!render(&records, &summary()).contains('\x1b'));
        let out = render(&records, &View { colour: true, ..summary() });
        assert!(out.contains("\x1b[38;5;109m●"), "{out}");
        assert!(out.contains("\x1b[38;5;144m"), "{out}");
        assert!(out.contains("\x1b[38;5;109m●\x1b[0m words   "));
    }

    #[test]
    fn colour_draws_each_period_in_its_commonest_mode() {
        let at = |mode: &str, at| {
            let mut r = record("ab", "ab", at);
            r.meta.mode = mode.into();
            r
        };
        let records = [
            at("text", "2026-10-23T08:00:00+02:00"),
            at("words", "2026-10-24T08:00:00+02:00"),
            at("words", "2026-10-24T09:00:00+02:00"),
            at("text", "2026-10-24T10:00:00+02:00"),
        ];
        let by = View { by: Some(Period::Day), ..summary() };
        assert!(!render(&records, &by).contains('\x1b'));
        let out = render(&records, &View { colour: true, ..by });
        let mean = out.split("mean per day").nth(1).unwrap();
        assert!(mean.contains("\x1b[38;5;144m●"), "{out}");
        assert!(mean.contains("\x1b[38;5;109m─"), "{out}");
        assert!(mean.contains("\x1b[38;5;144m●\x1b[0m text   "));
        assert!(mean.ends_with("m words\n"), "only drawn modes listed");
    }

    #[test]
    fn the_commonest_mode_names_a_column() {
        assert_eq!(dominant(&["text", "words", "words"]), "words");
        assert_eq!(dominant(&["funny", "text"]), "funny", "ties go first");
    }

    #[test]
    fn heatmaps_are_one_tile_row_per_key_row() {
        let records = [record("ab", "ab", "2026-10-24T09:00:00+02:00")];
        let out = render(&records, &detailed());
        assert!(out.contains("       A· S  D"), "{out}");
        assert!(out.contains("· clean   ░ <5%"));
        assert!(!out.contains('\x1b'), "no ANSI unless asked");
        assert!(render(&records, &View { colour: true, ..detailed() })
            .contains("\x1b[48;5;236;38;5;252m A \x1b[0m"));
    }

    #[test]
    fn periods_key_by_local_day_iso_week_and_month() {
        let at = "2025-12-29T23:30:00+01:00";
        assert_eq!(Period::Day.key(at).as_deref(), Some("2025-12-29"));
        assert_eq!(
            Period::Week.key(at).as_deref(),
            Some("2026-W01"),
            "ISO year"
        );
        assert_eq!(Period::Month.key(at).as_deref(), Some("2025-12"));
        assert_eq!(Period::Day.key("yesterday"), None);
    }

    #[test]
    fn grouping_orders_periods_and_drops_undated_tests() {
        let records = [
            record("ab", "ab", "2026-10-24T09:00:00+02:00"),
            record("ab", "ab", "2026-10-20T09:00:00+02:00"),
            record("ab", "ab", "2026-10-24T21:00:00+02:00"),
            record("ab", "ab", "not a time"),
        ];
        let groups = group(&records, Period::Day);
        let keys: Vec<(&str, usize)> =
            groups.iter().map(|(k, g)| (k.as_str(), g.len())).collect();
        assert_eq!(keys, [("2026-10-20", 1), ("2026-10-24", 2)]);
        assert_eq!(group(&records, Period::Week).len(), 1);
    }

    #[test]
    fn by_period_adds_a_table_and_plots_its_means() {
        let records = [
            record("ab cd", "ab cd", "2026-10-13T09:00:00+02:00"),
            record("ab cd", "ab cd", "2026-10-20T09:00:00+02:00"),
            record("ab cd", "ax", "2026-10-21T09:00:00+02:00"),
        ];
        let out =
            render(&records, &View { by: Some(Period::Week), ..summary() });
        assert!(out.contains("── By week"), "{out}");
        assert!(
            out.contains("Week        Tests      Time   WPM  Best  Accuracy")
        );
        assert!(out.contains("2026-W42"), "{out}");
        let row = out.lines().find(|l| l.starts_with("     2026-W43")).unwrap();
        assert!(row.contains("    2  ") && row.ends_with("85.7%"), "{row}");
        assert!(out.contains("wpm — mean per week"));
        assert!(!out.contains("per completed test"));
    }

    #[test]
    fn a_period_without_completed_tests_shows_dashes() {
        let records = [record("ab", "x", "2026-10-24T09:00:00+02:00")];
        let out =
            render(&records, &View { by: Some(Period::Day), ..summary() });
        let row =
            out.lines().find(|l| l.starts_with("     2026-10-24")).unwrap();
        assert!(row.contains("   —     —"), "{row}");
    }

    #[test]
    fn recent_texts_list_cited_tests_newest_first() {
        let cited = |text: &str, keys: &str, at: &str, cite: &str| {
            let mut r = record(text, keys, at);
            r.meta.cite = Some(cite.to_string());
            r
        };
        let records = [
            cited("ab", "ab", "2026-10-22T09:00:00+02:00", "Austen, Emma"),
            record("ab", "ab", "2026-10-23T09:00:00+02:00"),
            cited("ab cd", "ab", "2026-10-24T09:00:00+02:00", "Gogol, Nos"),
        ];
        let out = render(&records, &summary());
        let texts = out.split("── Recent texts").nth(1).unwrap();
        let lines: Vec<&str> = texts.lines().skip(1).take(2).collect();
        assert!(lines[0].starts_with("     2026-10-24"), "{out}");
        assert!(lines[0].ends_with("Gogol, Nos  (stopped)"), "{out}");
        assert!(lines[1].starts_with("     2026-10-22"), "{out}");
        assert!(lines[1].ends_with("100.0%  Austen, Emma"), "{out}");
        let plain = render(&records[1..2], &summary());
        assert!(!plain.contains("Recent texts"));
    }

    #[test]
    fn rhythm_shows_mean_spread_and_variation() {
        let records = [record("ab cd", "ab cd", "2026-10-24T09:00:00+02:00")];
        let out = render(&records, &summary());
        assert!(
            out.contains("Rhythm       : 150 ms ± 0 ms  (0% variation)"),
            "{out}"
        );
        let one = [record("a", "a", "2026-10-24T09:00:00+02:00")];
        assert!(!render(&one, &summary()).contains("Rhythm"));
    }

    #[test]
    fn geometry_replaces_heatmaps_with_finger_row_and_move_tables() {
        let records = [record("de fj", "de fj", "2026-10-24T09:00:00+02:00")];
        let out = render(&records, &View { geometry: true, ..summary() });
        for needle in [
            "── Fingers",
            "Finger        Keys   Share   Miss    Mean",
            "L middle         2   40.0%   0.0%  150 ms",
            "Thumb            1   20.0%   0.0%  150 ms",
            "── Rows",
            "Home             3   60.0%",
            "── Hand moves",
            "Same finger      1   25.0%   0.0%  150 ms",
            "Alternate        1   25.0%",
            "Space            2   50.0%",
            "── Same-finger bigrams",
            "de               1   25.0%   0.0%  150 ms",
        ] {
            assert!(out.contains(needle), "missing {needle:?}\n{out}");
        }
        assert!(out.contains("wpm — per completed test"));
        assert!(!out.contains("keyboard —") && !out.contains("Worst keys"));
        let both = View { geometry: true, ..detailed() };
        let out = render(&records, &both);
        let at = |needle| out.find(needle).unwrap_or_else(|| panic!("{out}"));
        assert!(at("Worst keys") < at("── Fingers"), "details, then geometry");
    }

    #[test]
    fn geometry_without_same_finger_bigrams_says_none() {
        let records = [record("fj", "fj", "2026-10-24T09:00:00+02:00")];
        let out = render(&records, &View { geometry: true, ..summary() });
        let tail = out.split("── Same-finger bigrams").nth(1).unwrap();
        assert!(tail.contains("(none)"), "{out}");
    }

    #[test]
    fn geometry_ends_on_the_standard_fingering_alone_when_data_is_thin() {
        let records = [record("fj", "fj", "2026-10-24T09:00:00+02:00")];
        let out = render(&records, &View { geometry: true, ..summary() });
        let tail = out.split("fingers — standard").nth(1).unwrap();
        assert!(tail.contains("\n          Z1 X2 C3 V4 B4 N7"), "{out}");
        assert!(tail.contains("≈ move as one   ? unresolved"), "{out}");
        assert!(!out.contains("yours") && !out.contains("`1"), "{out}");
    }

    #[test]
    fn inferred_boards_mark_keys_and_list_their_evidence() {
        use crate::fingering::Pair;
        let (q, j) = (Layout::Qwerty, Layout::Jcuken);
        let ms = Duration::from_millis;
        let key = |l: Layout, c| l.locate(c).unwrap();
        let pair = |l, a, b, median, count| Pair {
            keys: (key(l, a), key(l, b)),
            median: ms(median),
            count,
        };
        let marked = |mark, evidence| Marked { mark, evidence };
        let qwerty = Fingering {
            baseline: ms(134),
            marks: BTreeMap::from([
                (
                    key(q, 'b'),
                    marked(
                        Mark::Moved(Finger::RightIndex),
                        vec![
                            pair(q, 'm', 'b', 234, 17),
                            pair(q, 'u', 'b', 209, 9),
                        ],
                    ),
                ),
                (key(q, 'i'), marked(Mark::AsOne, vec![])),
            ]),
        };
        let jcuken = Fingering {
            baseline: ms(126),
            marks: BTreeMap::from([(
                key(j, 'т'),
                marked(Mark::Unsure, vec![pair(j, 'т', 'о', 79, 123)]),
            )]),
        };
        let out = fingering_boards(q, &[(q, qwerty), (j, jcuken)]);
        for needle in [
            "fingers — standard\n\n        Q1 W2 E3 R4 T4 Y7 U7 I8 O9 P0",
            "fingers — yours (qwerty), median 134 ms\n\n        Q1",
            "        Q1 W2 E3 R4 T4 Y7 U7 I≈ O9 P0",
            "          Z1 X2 C3 V4 B7 N7 M7 ,8",
            "\n     I≈\n     B7  mb 234 ms ×17 · ub 209 ms ×9\n",
            "fingers — yours (jcuken), median 126 ms",
            "          Я1 Ч2 С3 М4 И4 Т? Ь7",
            "\n     Т?  то 79 ms ×123\n",
        ] {
            assert!(out.contains(needle), "missing {needle:?}\n{out}");
        }
        assert!(!out.contains("`1"), "no number row without marks on it");
        let five = Fingering {
            marks: BTreeMap::from([(
                key(q, '5'),
                marked(Mark::Unsure, vec![]),
            )]),
            ..Fingering::default()
        };
        let out = fingering_boards(q, &[(q, five)]);
        assert_eq!(
            out.matches("\n      `1 11 22 33 44 54 67").count(),
            1,
            "{out}"
        );
        assert!(out.contains("\n      `1 11 22 33 44 5? 67"), "{out}");
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
