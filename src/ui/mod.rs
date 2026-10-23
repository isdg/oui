//! The screens, as render functions over plain data so each one can be drawn
//! into a test buffer.

pub mod results;

use std::time::Duration;

use ratatui::text::Line;

/// Widest the screens grow; wider terminals get margins.
pub const MAX_WIDTH: u16 = 100;

pub fn clock(d: Duration) -> String {
    let secs = d.as_secs();
    format!("{}:{:02}", secs / 60, secs % 60)
}

/// A character as listed in a ranking, where a bare space would vanish.
pub fn show(c: char) -> char {
    if c == ' ' {
        '␣'
    } else {
        c
    }
}

fn lines(text: Vec<String>) -> Vec<Line<'static>> {
    text.into_iter().map(Line::from).collect()
}

/// Draws into a `width` × `height` test buffer and returns its rows.
#[cfg(test)]
pub fn render(
    width: u16,
    height: u16,
    draw: impl FnOnce(&mut ratatui::Frame),
) -> Vec<String> {
    let backend = ratatui::backend::TestBackend::new(width, height);
    let mut terminal = ratatui::Terminal::new(backend).unwrap();
    terminal.draw(draw).unwrap();
    let buffer = terminal.backend().buffer();
    let row =
        |y| (0..width).map(|x| buffer[(x, y)].symbol()).collect::<String>();
    (0..height).map(row).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clock_is_minutes_and_padded_seconds() {
        assert_eq!(clock(Duration::from_millis(65_900)), "1:05");
        assert_eq!(clock(Duration::ZERO), "0:00");
    }
}
