//! Line chart in the style of `plc stat`: a stepped box-drawing line, one
//! column per sample, a right-aligned label on every row.

use std::fmt::Write as _;
use std::ops::Range;

/// Render `values` as `height + 1` lines: `height` plotted rows, each with
/// its label, then the bottom axis. `max` is the top of the y-axis.
pub fn line_chart(
    max: u64,
    height: usize,
    values: &[u64],
    fmt: &dyn Fn(u64) -> String,
) -> Vec<String> {
    line_chart_from(0, max, height, values, fmt)
}

/// [`line_chart`] over the y-axis `lo..=hi` rather than from zero, for
/// trends that would flatten into one row otherwise.
pub fn line_chart_from(
    lo: u64,
    hi: u64,
    height: usize,
    values: &[u64],
    fmt: &dyn Fn(u64) -> String,
) -> Vec<String> {
    let width = values.len();
    if width == 0 || height < 2 {
        return Vec::new();
    }
    let hi = hi.max(lo + 1);
    let range = hi - lo;
    let span = height as u64 - 1;
    let ticks = ticks(lo, hi, height);
    let label_width = fmt(hi).chars().count().max(fmt(lo).chars().count());

    // Row (0 = top) each column's point lands on.
    let rows: Vec<usize> = values
        .iter()
        .map(|&v| ((hi - v.clamp(lo, hi)) * span / range) as usize)
        .collect();

    let mut grid = vec![vec![' '; width]; height];
    grid[rows[0]][0] = '●';
    for i in 1..width {
        let (r, prev) = (rows[i], rows[i - 1]);
        if prev == r {
            grid[r][i] = '─';
        } else if prev < r {
            grid[prev][i] = '╮';
            for cell in grid.iter_mut().take(r).skip(prev + 1) {
                cell[i] = '│';
            }
            grid[r][i] = '╰';
        } else {
            grid[prev][i] = '╯';
            for cell in grid.iter_mut().take(prev).skip(r + 1) {
                cell[i] = '│';
            }
            grid[r][i] = '╭';
        }
    }

    let text = grid.iter().map(|row| row.iter().collect()).collect();
    frame(&ticks, text, width, label_width, fmt)
}

/// `rows` behind a y-axis labelled with `ticks`, top first, the last of
/// them under the bottom rule `width` columns long.
pub fn frame(
    ticks: &[u64],
    rows: Vec<String>,
    width: usize,
    label_width: usize,
    fmt: &dyn Fn(u64) -> String,
) -> Vec<String> {
    let mut lines = Vec::with_capacity(rows.len() + 1);
    for (r, row) in rows.iter().enumerate() {
        let sep = if r == 0 { '┼' } else { '┤' };
        lines.push(format!("{:>label_width$} {sep}{row}", fmt(ticks[r])));
    }
    let lo = ticks.last().copied().unwrap_or(0);
    let mut bottom = format!("{:>label_width$} └", fmt(lo));
    bottom.extend(std::iter::repeat_n('─', width));
    lines.push(bottom);
    lines
}

/// The y-axis value of each line [`line_chart_from`] draws over `lo..=hi`,
/// top row first, the bottom rule's `lo` last.
pub fn ticks(lo: u64, hi: u64, height: usize) -> Vec<u64> {
    let range = hi.max(lo + 1) - lo;
    let span = (height as u64).saturating_sub(1).max(1);
    let rows = (0..height as u64).map(|r| lo + range * (span - r) / span);
    rows.chain([lo]).collect()
}

/// The y-axis `(lo, step)` of a braille chart `rows` tall: whole numbers
/// `step` apart from `lo`, the tightest that holds `min..=max`.
pub fn scale(min: f64, max: f64, rows: usize) -> (u64, u64) {
    let (min, max) = (min.max(0.0), max.max(min).max(0.0));
    let rows = rows.max(1) as u64;
    let mut step = ((max - min) / rows as f64).ceil().max(1.0) as u64;
    loop {
        let lo = min as u64 / step * step;
        if (lo + step * rows) as f64 >= max {
            return (lo, step);
        }
        step += 1;
    }
}

/// The dot, counted up from 0 at the bottom of a chart `rows` tall, that
/// `value` lands on when [`scale`] gave `(lo, step)`: four dots a row, the
/// top one of each level with the row's label.
pub fn dot(value: f64, lo: u64, step: u64, rows: usize) -> usize {
    let level = ((value - lo as f64) * 4.0 / step as f64).round();
    (level.max(1.0) as usize - 1).min(rows * 4 - 1)
}

/// Braille cells of 2×4 dots: a line traced over a band filled under it.
pub struct Canvas {
    cols: usize,
    rows: usize,
    line: Vec<u8>,
    band: Vec<u8>,
}

/// One cell's bits by its dot's column, then its row from the top.
const BRAILLE: [[u8; 4]; 2] =
    [[0x01, 0x02, 0x04, 0x40], [0x08, 0x10, 0x20, 0x80]];

impl Canvas {
    /// A blank canvas wide enough for `dots` columns of dots.
    pub fn new(dots: usize, rows: usize) -> Self {
        let cols = dots.div_ceil(2);
        let blank = vec![0; cols * rows];
        Canvas { cols, rows, line: blank.clone(), band: blank }
    }

    /// Cells across.
    pub fn cols(&self) -> usize {
        self.cols
    }

    fn set(cells: &mut [u8], cols: usize, rows: usize, x: usize, y: usize) {
        if x >= cols * 2 || y >= rows * 4 {
            return;
        }
        let row = rows - 1 - y / 4;
        cells[row * cols + x / 2] |= BRAILLE[x % 2][3 - y % 4];
    }

    /// Trace a line through dot `ys[x]` of each dot column `x`, each column
    /// reaching halfway to its neighbours so the line never breaks.
    pub fn trace(&mut self, ys: &[usize]) {
        for (x, &y) in ys.iter().enumerate() {
            let (mut low, mut high) = (y, y);
            let near =
                [x.checked_sub(1).map(|i| ys[i]), ys.get(x + 1).copied()];
            for n in near.into_iter().flatten() {
                match n.cmp(&y) {
                    std::cmp::Ordering::Greater => high = high.max((y + n) / 2),
                    std::cmp::Ordering::Less => {
                        low = low.min((y + n).div_ceil(2))
                    }
                    std::cmp::Ordering::Equal => {}
                }
            }
            for y in low..=high {
                Self::set(&mut self.line, self.cols, self.rows, x, y);
            }
        }
    }

    /// Fill dot column `x` of the band from dot `low` up to `high`.
    pub fn fill(&mut self, x: usize, low: usize, high: usize) {
        for y in low..=high {
            Self::set(&mut self.band, self.cols, self.rows, x, y);
        }
    }

    /// Each row's characters, top first, with which of them hold the line.
    /// `cut` cuts the line out of the band, for output with no colour to
    /// tell them apart; a cell the cut would empty shows the line instead.
    pub fn render(&self, cut: bool) -> Vec<(String, Vec<bool>)> {
        let cell = |i: usize| {
            let (line, band) = (self.line[i], self.band[i]);
            let bits = match band & !line {
                kept if cut && kept != 0 => kept,
                _ if cut && line != 0 => line,
                _ => line | band,
            };
            let c = char::from_u32(0x2800 + u32::from(bits));
            (c.filter(|_| bits != 0).unwrap_or(' '), line != 0)
        };
        let row =
            |r: usize| (0..self.cols).map(|c| cell(r * self.cols + c)).unzip();
        (0..self.rows).map(row).collect()
    }
}

/// The lower quartile, median and upper quartile of `values` over a
/// window `half` either side of each of at most `width` columns' centres.
pub fn spread(values: &[f64], width: usize, half: usize) -> Vec<[f64; 3]> {
    let window = |b: Range<usize>| {
        let centre = (b.start + b.end.saturating_sub(1)) / 2;
        let end = (centre + half + 1).min(values.len());
        let mut w = values[centre.saturating_sub(half)..end].to_vec();
        w.sort_by(f64::total_cmp);
        [0.25, 0.5, 0.75].map(|q| quantile(&w, q))
    };
    buckets(values.len(), width).map(window).collect()
}

/// The `q` quantile of `sorted`, interpolated between its neighbours.
fn quantile(sorted: &[f64], q: f64) -> f64 {
    let at = (sorted.len() - 1) as f64 * q;
    let (i, frac) = (at.floor() as usize, at.fract());
    let next = sorted[(i + 1).min(sorted.len() - 1)];
    sorted[i] + (next - sorted[i]) * frac
}

/// Average `values` down into at most `width` columns.
pub fn resample(values: &[f64], width: usize) -> Vec<f64> {
    let mean = |s: &[f64]| s.iter().sum::<f64>() / s.len() as f64;
    buckets(values.len(), width).map(|b| mean(&values[b])).collect()
}

/// The index ranges [`resample`] averages `len` samples over: one per
/// sample when they fit in `width` columns.
pub fn buckets(len: usize, width: usize) -> impl Iterator<Item = Range<usize>> {
    let width = if width == 0 { len } else { width.min(len) };
    let edge = move |i: usize| i * len / width;
    (0..width).map(move |i| edge(i)..edge(i + 1))
}

/// `line` with the characters from `skip` on drawn in the ANSI 256
/// colours of `colours`, one per character; `None` and blanks stay plain.
pub fn tint(line: &str, skip: usize, colours: &[Option<u8>]) -> String {
    let mut out = String::with_capacity(line.len());
    let mut open = None;
    for (i, c) in line.chars().enumerate() {
        let colour = i.checked_sub(skip).and_then(|i| *colours.get(i)?);
        let colour = colour.filter(|_| c != ' ');
        if colour != open {
            if open.is_some() {
                out.push_str("\x1b[0m");
            }
            if let Some(n) = colour {
                let _ = write!(out, "\x1b[38;5;{n}m");
            }
            open = colour;
        }
        out.push(c);
    }
    if open.is_some() {
        out.push_str("\x1b[0m");
    }
    out
}

/// The x-axis under a chart of `cols` columns, indented by `indent`: a mark
/// every ten columns giving the position there, `per_col` units apart.
pub fn axis(indent: usize, cols: usize, per_col: f64) -> String {
    let mut out = " ".repeat(indent);
    let mut col = 0;
    while col < cols {
        if (col + 1) % 10 != 0 {
            out.push(' ');
            col += 1;
            continue;
        }
        let label = format!("{}", ((col + 1) as f64 * per_col).round());
        col += label.chars().count();
        out.push_str(&label);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plain(v: u64) -> String {
        v.to_string()
    }

    #[test]
    fn flat_line_sits_on_one_row() {
        let lines = line_chart(10, 3, &[10, 10, 10], &plain);
        assert_eq!(lines, ["10 ┼●──", " 5 ┤   ", " 0 ┤   ", " 0 └───"]);
    }

    #[test]
    fn steps_join_with_rounded_corners() {
        let lines = line_chart(2, 3, &[0, 2, 0], &plain);
        assert_eq!(lines, ["2 ┼ ╭╮", "1 ┤ ││", "0 ┤●╯╰", "0 └───"]);
    }

    #[test]
    fn values_above_max_are_clamped() {
        let lines = line_chart(4, 2, &[9], &plain);
        assert_eq!(lines[0], "4 ┼●");
    }

    #[test]
    fn a_raised_floor_spreads_a_narrow_range() {
        let lines = line_chart_from(70, 90, 3, &[70, 80, 90], &plain);
        assert_eq!(lines, ["90 ┼  ╭", "80 ┤ ╭╯", "70 ┤●╯ ", "70 └───"]);
        assert_eq!(line_chart_from(50, 50, 2, &[40], &plain)[1], "50 ┤●");
        assert_eq!(ticks(70, 90, 3), [90, 80, 70, 70]);
        assert_eq!(ticks(50, 50, 2), [51, 50, 50]);
    }

    #[test]
    fn degenerate_shapes_render_nothing() {
        assert!(line_chart(5, 8, &[], &plain).is_empty());
        assert!(line_chart(5, 1, &[1, 2], &plain).is_empty());
    }

    #[test]
    fn scale_is_the_tightest_whole_step() {
        assert_eq!(scale(49.2, 75.8, 8), (48, 4));
        assert_eq!(scale(57.0, 101.0, 8), (54, 6));
        assert_eq!(scale(60.0, 60.0, 8), (60, 1));
        assert_eq!(scale(0.0, 0.0, 8), (0, 1));
    }

    #[test]
    fn dots_count_up_with_labels_on_the_top_one() {
        assert_eq!(dot(48.0, 48, 4, 2), 0);
        assert_eq!(dot(49.0, 48, 4, 2), 0);
        assert_eq!(dot(52.0, 48, 4, 2), 3);
        assert_eq!(dot(56.0, 48, 4, 2), 7);
        assert_eq!(dot(99.0, 48, 4, 2), 7);
        assert_eq!(dot(10.0, 48, 4, 2), 0);
    }

    fn text(canvas: &Canvas, cut: bool) -> Vec<String> {
        canvas.render(cut).into_iter().map(|(row, _)| row).collect()
    }

    #[test]
    fn a_trace_joins_halfway_to_each_neighbour() {
        let mut canvas = Canvas::new(3, 2);
        canvas.trace(&[0, 7, 7]);
        assert_eq!(canvas.cols(), 2);
        assert_eq!(text(&canvas, false), ["⢸⠁", "⡇ "]);
        let mut flat = Canvas::new(1, 1);
        flat.trace(&[2]);
        assert_eq!(text(&flat, false), ["⠂"]);
    }

    #[test]
    fn render_marks_line_cells_and_can_cut_the_band() {
        let mut canvas = Canvas::new(2, 1);
        canvas.fill(0, 0, 3);
        canvas.fill(1, 1, 1);
        canvas.trace(&[1, 1]);
        let rows = canvas.render(false);
        assert_eq!(rows, [("⡧".to_string(), vec![true])]);
        assert_eq!(text(&canvas, true), ["⡃"]);
        let mut bare = Canvas::new(2, 2);
        bare.fill(0, 0, 0);
        assert_eq!(bare.render(true)[0], (" ".to_string(), vec![false]));
        assert_eq!(bare.render(true)[1], ("⡀".to_string(), vec![false]));
    }

    #[test]
    fn spread_takes_quartiles_over_a_window() {
        let values = [1.0, 2.0, 3.0, 4.0, 5.0];
        assert_eq!(spread(&values, 5, 0)[2], [3.0, 3.0, 3.0]);
        assert_eq!(spread(&values, 5, 2)[2], [2.0, 3.0, 4.0]);
        assert_eq!(spread(&values, 5, 1)[0], [1.25, 1.5, 1.75]);
        assert_eq!(spread(&values, 1, 0), [[3.0, 3.0, 3.0]]);
        assert!(spread(&[], 5, 1).is_empty());
    }

    #[test]
    fn resample_averages_into_buckets() {
        assert_eq!(resample(&[1.0, 3.0, 5.0, 7.0], 2), [2.0, 6.0]);
        assert_eq!(resample(&[1.0, 2.0], 10), [1.0, 2.0]);
    }

    #[test]
    fn buckets_split_samples_as_resample_does() {
        let b: Vec<_> = buckets(5, 2).collect();
        assert_eq!(b, [0..2, 2..5]);
        assert_eq!(buckets(2, 10).collect::<Vec<_>>(), [0..1, 1..2]);
        assert_eq!(buckets(0, 3).count(), 0);
    }

    #[test]
    fn tint_colours_runs_past_the_label() {
        let colours = [Some(1), Some(1), Some(2), Some(2), None];
        assert_eq!(
            tint("7 ┤●─ ╮╰", 3, &colours),
            "7 ┤\x1b[38;5;1m●─\x1b[0m \x1b[38;5;2m╮\x1b[0m╰"
        );
        assert_eq!(tint("ab", 0, &[]), "ab");
    }

    #[test]
    fn axis_marks_every_tenth_column() {
        assert_eq!(
            axis(2, 25, 1.0),
            format!("  {}10{}20{}", " ".repeat(9), " ".repeat(8), "    ")
        );
        assert_eq!(axis(0, 10, 2.5), format!("{}25", " ".repeat(9)));
    }
}
