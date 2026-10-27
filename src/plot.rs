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

    let mut lines = Vec::with_capacity(height + 1);
    for (r, gridrow) in grid.iter().enumerate() {
        let val = lo + range * (span - r as u64) / span;
        let sep = if r == 0 { '┼' } else { '┤' };
        let mut line = format!("{:>label_width$} {sep}", fmt(val));
        line.extend(gridrow);
        lines.push(line);
    }
    let mut bottom = format!("{:>label_width$} └", fmt(lo));
    bottom.extend(std::iter::repeat_n('─', width));
    lines.push(bottom);
    lines
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
    }

    #[test]
    fn degenerate_shapes_render_nothing() {
        assert!(line_chart(5, 8, &[], &plain).is_empty());
        assert!(line_chart(5, 1, &[1, 2], &plain).is_empty());
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
