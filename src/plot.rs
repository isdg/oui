//! Line chart in the style of `plc stat`: a stepped box-drawing line, one
//! column per sample, a right-aligned label on every row.

/// Render `values` as `height + 1` lines: `height` plotted rows, each with
/// its label, then the bottom axis. `max` is the top of the y-axis.
pub fn line_chart(
    max: u64,
    height: usize,
    values: &[u64],
    fmt: &dyn Fn(u64) -> String,
) -> Vec<String> {
    let width = values.len();
    if width == 0 || height < 2 {
        return Vec::new();
    }
    let max = max.max(1);
    let span = height as u64 - 1;
    let label_width = fmt(max).chars().count().max(fmt(0).chars().count());

    // Row (0 = top) each column's point lands on.
    let rows: Vec<usize> = values
        .iter()
        .map(|&v| ((max - v.min(max)) * span / max) as usize)
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
        let val = max * (span - r as u64) / span;
        let sep = if r == 0 { '┼' } else { '┤' };
        let mut line = format!("{:>label_width$} {sep}", fmt(val));
        line.extend(gridrow);
        lines.push(line);
    }
    let mut bottom = format!("{:>label_width$} └", fmt(0));
    bottom.extend(std::iter::repeat_n('─', width));
    lines.push(bottom);
    lines
}

/// Average `values` down into at most `width` columns.
pub fn resample(values: &[f64], width: usize) -> Vec<f64> {
    if width == 0 || values.len() <= width {
        return values.to_vec();
    }
    let edge = |i: usize| i * values.len() / width;
    let mean = |s: &[f64]| s.iter().sum::<f64>() / s.len() as f64;
    (0..width).map(|i| mean(&values[edge(i)..edge(i + 1)])).collect()
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
    fn axis_marks_every_tenth_column() {
        assert_eq!(
            axis(2, 25, 1.0),
            format!("  {}10{}20{}", " ".repeat(9), " ".repeat(8), "    ")
        );
        assert_eq!(axis(0, 10, 2.5), format!("{}25", " ".repeat(9)));
    }
}
